//! Sending a call again after it failed, as gRFC A6 has it: a number of attempts, a backoff drawn
//! below a growing bound, the codes worth another try, and the copy of what the call sent that a
//! later attempt replays.
//!
//! A call stays retryable while no response head has reached its reader and what it sent fits its
//! replay ceiling and the channel's total of replay bytes. Past either, it is committed: it goes
//! on, and is not tried again.

use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use bytes::Bytes;
use tonic::codegen::tokio_stream::Stream;

use super::call::RequestMessages;
use super::error::GrpcChannelConfigError;
use super::status::GrpcStatusCode;

/// When a failed call is sent again, and what it may keep to send it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RetryConfig {
    /// Attempts in all, the first included; 1 never retries.
    pub max_attempts: u32,
    /// The bound of the first backoff.
    pub initial_backoff: Duration,
    /// What the bound grows to and no further.
    pub max_backoff: Duration,
    /// What each bound is multiplied by, at least 1.
    pub backoff_multiplier: f64,
    /// The codes a call is tried again for.
    pub retryable_codes: Vec<GrpcStatusCode>,
    /// The bytes one call may keep for a replay.
    pub call_replay_bytes: usize,
    /// The bytes all of a channel's calls may keep for a replay together.
    pub channel_replay_bytes: usize,
}

/// The policy `GrpcClient` gives grpc-dotnet, with grpc-dotnet's replay limits.
impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(5),
            backoff_multiplier: 1.5,
            retryable_codes: vec![
                GrpcStatusCode::Unavailable,
                GrpcStatusCode::Aborted,
                GrpcStatusCode::Unknown,
            ],
            call_replay_bytes: 1024 * 1024,
            channel_replay_bytes: 16 * 1024 * 1024,
        }
    }
}

impl RetryConfig {
    pub(crate) fn admissible(&self) -> Result<(), GrpcChannelConfigError> {
        let refuse = |why: &str| {
            Err(GrpcChannelConfigError::Retry {
                why: why.to_owned(),
            })
        };
        if self.max_attempts == 0 {
            return refuse("`max_attempts` of zero makes no attempt at all; 1 never retries");
        }
        if self.initial_backoff.is_zero() {
            return refuse("an `initial_backoff` of zero retries at once, with no backoff");
        }
        if self.max_backoff < self.initial_backoff {
            return refuse("`max_backoff` is below `initial_backoff`, the bound it starts from");
        }
        if !(self.backoff_multiplier.is_finite() && self.backoff_multiplier >= 1.0) {
            return refuse("`backoff_multiplier` has to be a finite number of at least 1");
        }
        Ok(())
    }

    /// What a call may keep, or nothing when no attempt can follow the first.
    pub(crate) fn replay_limit(&self) -> Option<usize> {
        (self.max_attempts > 1).then_some(self.call_replay_bytes)
    }

    /// The bound after `bound`: multiplied, and held to `max_backoff`.
    pub(crate) fn next_bound(&self, bound: Duration) -> Duration {
        Duration::try_from_secs_f64(bound.as_secs_f64() * self.backoff_multiplier)
            .unwrap_or(self.max_backoff)
            .min(self.max_backoff)
    }
}

/// A backoff drawn uniformly below `bound`, so that calls that failed together do not come back
/// together.
pub(crate) fn jittered(bound: Duration) -> Duration {
    bound.mul_f64(fastrand::f64())
}

/// The replay bytes a channel's calls hold together, against their limit.
#[derive(Debug)]
pub(crate) struct ChannelReplay {
    used: AtomicUsize,
    limit: usize,
}

impl ChannelReplay {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            used: AtomicUsize::new(0),
            limit,
        }
    }

    fn reserve(&self, bytes: usize) -> bool {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|after| *after <= self.limit)
            })
            .is_ok()
    }

    fn release(&self, bytes: usize) {
        self.used.fetch_sub(bytes, Ordering::AcqRel);
    }

    #[cfg(test)]
    fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
}

/// What a call sent, kept for the attempts after the first, and the messages it is still to
/// send. Dropping it lets the copy go and ends the last attempt's stream.
pub(crate) struct Replay(Arc<Mutex<Kept>>);

struct Kept {
    live: RequestMessages,
    messages: Vec<Bytes>,
    bytes: usize,
    call_limit: usize,
    channel: Arc<ChannelReplay>,
    committed: bool,
    ended: bool,
    /// The current attempt; a stream of any other reads nothing more.
    attempt: u64,
    /// How many of the kept messages the current attempt has sent.
    replayed: usize,
    /// The current attempt's stream, when it waits on `live`: superseding it wakes it, to end.
    parked: Option<Waker>,
}

impl Kept {
    /// No attempt follows. The copy goes now, or once the current attempt has sent all of it:
    /// a replay cut short would send the server a stream with a hole.
    fn commit(&mut self) {
        self.committed = true;
        if self.replayed >= self.messages.len() {
            self.let_go();
        }
    }

    fn let_go(&mut self) {
        self.messages = Vec::new();
        self.channel.release(std::mem::take(&mut self.bytes));
    }

    /// The current attempt is over: its stream ends at its next poll, which this wakes.
    fn supersede(&mut self) {
        self.attempt += 1;
        self.replayed = 0;
        if let Some(parked) = self.parked.take() {
            parked.wake();
        }
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        self.channel.release(self.bytes);
    }
}

impl Replay {
    /// `live`, kept up to `call_limit` and the channel's total; a call with no retry keeps
    /// nothing, being committed from the start.
    pub(crate) fn new(
        live: RequestMessages,
        call_limit: Option<usize>,
        channel: Arc<ChannelReplay>,
    ) -> Self {
        Self(Arc::new(Mutex::new(Kept {
            live,
            messages: Vec::new(),
            bytes: 0,
            call_limit: call_limit.unwrap_or(0),
            channel,
            committed: call_limit.is_none(),
            ended: false,
            attempt: 0,
            replayed: 0,
            parked: None,
        })))
    }

    fn kept(&self) -> MutexGuard<'_, Kept> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The messages of a new attempt: what was kept, then what the call goes on to send. The
    /// attempt before it reads nothing more, so a body a failed stream still holds cannot take a
    /// message from the one that replaced it.
    pub(crate) fn attempt(&self) -> AttemptMessages {
        let mut kept = self.kept();
        kept.supersede();
        AttemptMessages {
            kept: Arc::clone(&self.0),
            attempt: kept.attempt,
        }
    }

    /// The attempt that failed reads nothing more, while the call waits for the next one; false
    /// when the call is committed, and there is no next one.
    pub(crate) fn supersede(&self) -> bool {
        let mut kept = self.kept();
        kept.supersede();
        !kept.committed
    }

    /// No attempt follows.
    pub(crate) fn commit(&self) {
        self.kept().commit();
    }

    #[cfg(test)]
    fn is_committed(&self) -> bool {
        self.kept().committed
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        let mut kept = self.kept();
        kept.supersede();
        kept.let_go();
    }
}

/// One attempt's request messages, as the stream tonic's client encodes.
pub(crate) struct AttemptMessages {
    kept: Arc<Mutex<Kept>>,
    attempt: u64,
}

impl Stream for AttemptMessages {
    type Item = Bytes;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let mut kept = this.kept.lock().unwrap_or_else(PoisonError::into_inner);
        if kept.attempt != this.attempt {
            return Poll::Ready(None);
        }
        if let Some(message) = kept.messages.get(kept.replayed).cloned() {
            kept.replayed += 1;
            if kept.committed && kept.replayed == kept.messages.len() {
                kept.let_go();
            }
            return Poll::Ready(Some(message));
        }
        if kept.ended {
            return Poll::Ready(None);
        }
        match Pin::new(&mut kept.live).poll_next(cx) {
            Poll::Pending => {
                kept.parked = Some(cx.waker().clone());
                Poll::Pending
            }
            Poll::Ready(None) => {
                kept.ended = true;
                Poll::Ready(None)
            }
            Poll::Ready(Some(message)) => {
                if !kept.committed {
                    let fits = kept
                        .bytes
                        .checked_add(message.len())
                        .is_some_and(|after| after <= kept.call_limit);
                    if fits && kept.channel.reserve(message.len()) {
                        // A copy of its own, so the host's buffer goes back once it is encoded.
                        kept.messages.push(Bytes::copy_from_slice(&message));
                        kept.bytes += message.len();
                        kept.replayed += 1;
                    } else {
                        kept.commit();
                    }
                }
                Poll::Ready(Some(message))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::sync::watch;
    use tonic::codegen::tokio_stream::StreamExt;

    use super::super::call::create;

    #[test]
    fn a_policy_that_could_not_back_off_is_refused() {
        for config in [
            RetryConfig {
                max_attempts: 0,
                ..RetryConfig::default()
            },
            RetryConfig {
                initial_backoff: Duration::ZERO,
                ..RetryConfig::default()
            },
            RetryConfig {
                max_backoff: Duration::from_millis(500),
                ..RetryConfig::default()
            },
            RetryConfig {
                backoff_multiplier: 0.5,
                ..RetryConfig::default()
            },
            RetryConfig {
                backoff_multiplier: f64::NAN,
                ..RetryConfig::default()
            },
        ] {
            assert!(config.admissible().is_err(), "{config:?}");
        }
        assert!(RetryConfig::default().admissible().is_ok());
    }

    #[test]
    fn the_bound_grows_by_the_multiplier_to_the_maximum() {
        let config = RetryConfig::default();
        let mut bound = config.initial_backoff;
        let mut bounds = Vec::new();
        for _ in 0..5 {
            bounds.push(bound);
            bound = config.next_bound(bound);
        }
        assert_eq!(
            bounds,
            [1000, 1500, 2250, 3375, 5000].map(Duration::from_millis)
        );
        for _ in 0..100 {
            assert!(jittered(Duration::from_secs(1)) < Duration::from_secs(1));
        }
    }

    /// The kept copy is what a second attempt sends, and the channel's total counts it until the
    /// call is committed.
    #[tokio::test]
    async fn a_second_attempt_replays_what_the_first_sent() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, closed);
        let (mut send, _recv, _control) = call.split();
        let channel = Arc::new(ChannelReplay::new(1024));
        let replay = Replay::new(live, Some(64), Arc::clone(&channel));

        send.send_message(Bytes::from_static(b"one"))
            .await
            .expect("sent");
        let mut first = replay.attempt();
        assert_eq!(first.next().await, Some(Bytes::from_static(b"one")));
        assert_eq!(channel.used(), 3);

        send.send_message(Bytes::from_static(b"two"))
            .await
            .expect("sent");
        drop(send);
        let mut second = replay.attempt();
        assert_eq!(first.next().await, None, "the first attempt reads no more");
        assert_eq!(second.next().await, Some(Bytes::from_static(b"one")));
        assert_eq!(second.next().await, Some(Bytes::from_static(b"two")));
        assert_eq!(second.next().await, None);

        replay.commit();
        assert_eq!(channel.used(), 0);
        assert!(replay.is_committed());
    }

    /// A head that arrives while an attempt is still replaying commits the call, and the attempt
    /// still sends what was kept before the messages that follow.
    #[tokio::test]
    async fn a_commit_during_a_replay_lets_the_replay_finish() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, closed);
        let (mut send, _recv, _control) = call.split();
        let channel = Arc::new(ChannelReplay::new(1024));
        let replay = Replay::new(live, Some(64), Arc::clone(&channel));

        for message in [&b"one"[..], b"two"] {
            send.send_message(Bytes::from_static(message))
                .await
                .expect("sent");
        }
        let mut first = replay.attempt();
        first.next().await;
        first.next().await;

        let mut second = replay.attempt();
        assert_eq!(second.next().await, Some(Bytes::from_static(b"one")));
        replay.commit();
        assert_eq!(channel.used(), 6, "the copy is still being sent");
        assert_eq!(second.next().await, Some(Bytes::from_static(b"two")));
        assert_eq!(channel.used(), 0, "and goes once it is");

        send.send_message(Bytes::from_static(b"three"))
            .await
            .expect("sent");
        assert_eq!(second.next().await, Some(Bytes::from_static(b"three")));
    }

    /// A failed attempt waiting on the host's next message is woken when it is superseded, and
    /// ends, rather than taking that message from the attempt after it.
    #[tokio::test]
    async fn a_superseded_attempt_waiting_for_a_message_ends() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, closed);
        let (mut send, _recv, _control) = call.split();
        let channel = Arc::new(ChannelReplay::new(1024));
        let replay = Replay::new(live, Some(64), Arc::clone(&channel));

        let mut first = replay.attempt();
        let waiting = tokio::spawn(async move { first.next().await });
        tokio::task::yield_now().await;
        replay.supersede();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), waiting)
                .await
                .expect("the superseded attempt was woken")
                .expect("it did not panic"),
            None
        );

        send.send_message(Bytes::from_static(b"late"))
            .await
            .expect("sent");
        let mut second = replay.attempt();
        assert_eq!(second.next().await, Some(Bytes::from_static(b"late")));
    }

    /// A call that ends with no attempt to follow gives the channel back its share at once, even
    /// while a stream of it is still held.
    #[tokio::test]
    async fn a_replay_dropped_gives_its_bytes_back() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, closed);
        let (mut send, _recv, _control) = call.split();
        let channel = Arc::new(ChannelReplay::new(1024));
        let replay = Replay::new(live, Some(64), Arc::clone(&channel));

        send.send_message(Bytes::from_static(b"kept"))
            .await
            .expect("sent");
        let mut held = replay.attempt();
        held.next().await;
        assert_eq!(channel.used(), 4);
        drop(replay);
        assert_eq!(channel.used(), 0);
        assert_eq!(held.next().await, None);
    }

    #[tokio::test]
    async fn a_call_past_its_ceiling_or_the_channels_is_committed() {
        for (call_limit, channel_limit) in [(4, 1024), (64, 4)] {
            let (_closed, closed) = watch::channel(false);
            let (call, live, _driving) = create(4, closed);
            let (mut send, _recv, _control) = call.split();
            let channel = Arc::new(ChannelReplay::new(channel_limit));
            let replay = Replay::new(live, Some(call_limit), Arc::clone(&channel));

            send.send_message(Bytes::from_static(b"12345"))
                .await
                .expect("sent");
            let mut first = replay.attempt();
            assert_eq!(first.next().await, Some(Bytes::from_static(b"12345")));
            assert_eq!(channel.used(), 0);
            assert!(!replay.supersede(), "{call_limit}/{channel_limit}");
        }
    }

    /// A policy of one attempt keeps no copy, there being no attempt to replay it.
    #[test]
    fn a_policy_that_never_retries_keeps_nothing() {
        let never = RetryConfig {
            max_attempts: 1,
            ..RetryConfig::default()
        };
        assert_eq!(never.replay_limit(), None);
        assert_eq!(RetryConfig::default().replay_limit(), Some(1024 * 1024));
    }
}
