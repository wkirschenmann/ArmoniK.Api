//! Sending a call again after it failed, as gRFC A6 has it: a number of attempts, a backoff drawn
//! below a growing bound, the codes worth another try, and what the call sent, kept for a later
//! attempt to replay.
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
    /// Attempts in all, the first included; 1 retries nothing. A call its peer never processed
    /// goes again besides, whatever this is, while every message it sent is kept.
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
            return refuse("`max_attempts` of zero makes no attempt at all; 1 is the least");
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
/// send. Dropping it lets what it kept go and ends the last attempt's stream.
pub(crate) struct Replay(Arc<Mutex<Kept>>);

struct Kept {
    live: RequestMessages,
    messages: Vec<Bytes>,
    bytes: usize,
    call_limit: usize,
    channel: Arc<ChannelReplay>,
    committed: bool,
    /// A message went out that no later attempt can send again.
    lost: bool,
    ended: bool,
    /// The current attempt; a stream of any other reads nothing more.
    attempt: u64,
    /// How many of the kept messages the current attempt has sent.
    replayed: usize,
    /// The current attempt's stream, when it waits on `live`: superseding it wakes it, to end.
    parked: Option<Waker>,
}

impl Kept {
    /// No attempt follows. What was kept goes now, or once the current attempt has sent all of it:
    /// a replay cut short would send the server a stream with a hole.
    fn commit(&mut self) {
        self.committed = true;
        if self.replayed >= self.messages.len() {
            self.let_go();
        }
    }

    fn let_go(&mut self) {
        self.lost |= !self.messages.is_empty();
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
            lost: false,
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

    /// The attempt that failed reads nothing more, while the call waits for the next one; what
    /// that one could be is read under the same lock, so no message the failed one takes after is
    /// missed.
    pub(crate) fn supersede(&self) -> Standing {
        let mut kept = self.kept();
        kept.supersede();
        Standing {
            retryable: !kept.committed,
            whole: !kept.lost,
        }
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

/// What a call sent, kept for another attempt: a stream of messages, or one framed request.
pub(crate) enum Sent {
    Stream(Replay),
    One(OneReplay),
}

impl Sent {
    /// A new attempt's request: what tonic encodes, and the body the engine's own service gives
    /// the request instead when there is one.
    pub(crate) fn attempt(&self) -> (Attempt, Option<Bytes>) {
        match self {
            Self::Stream(replay) => (Attempt::Stream(replay.attempt()), None),
            Self::One(one) => (Attempt::Framed, Some(one.request.clone())),
        }
    }

    pub(crate) fn supersede(&self) -> Standing {
        match self {
            Self::Stream(replay) => replay.supersede(),
            Self::One(one) => one.standing(),
        }
    }

    pub(crate) fn commit(&self) {
        match self {
            Self::Stream(replay) => replay.commit(),
            Self::One(one) => one.commit(),
        }
    }
}

/// One attempt's messages as tonic encodes them: the call's stream, or nothing when the request's
/// body is the framed one.
pub(crate) enum Attempt {
    Stream(AttemptMessages),
    Framed,
}

impl Stream for Attempt {
    type Item = Bytes;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            Self::Stream(messages) => Pin::new(messages).poll_next(cx),
            Self::Framed => Poll::Ready(None),
        }
    }
}

/// The one request of a call that sends one, which every attempt sends again whole. Held for the
/// whole call, so a transparent retry can always send it; retried under the policy while it fits
/// the call's ceiling and the channel's total, to which it is charged until the call commits.
pub(crate) struct OneReplay {
    request: Bytes,
    channel: Arc<ChannelReplay>,
    /// What it holds of the channel's total; zero once committed, or when it never fitted.
    reserved: AtomicUsize,
}

impl OneReplay {
    pub(crate) fn new(
        request: Bytes,
        call_limit: Option<usize>,
        channel: Arc<ChannelReplay>,
    ) -> Self {
        let len = request.len();
        let kept = call_limit.is_some_and(|limit| len <= limit) && channel.reserve(len);
        Self {
            request,
            channel,
            reserved: AtomicUsize::new(if kept { len } else { 0 }),
        }
    }

    fn standing(&self) -> Standing {
        Standing {
            retryable: self.reserved.load(Ordering::Acquire) > 0,
            whole: true,
        }
    }

    fn commit(&self) {
        self.channel
            .release(self.reserved.swap(0, Ordering::AcqRel));
    }
}

impl Drop for OneReplay {
    fn drop(&mut self) {
        self.commit();
    }
}

/// What the attempt after a failed one could be.
pub(crate) struct Standing {
    /// Nothing has committed the call, so the policy may try it again.
    pub(crate) retryable: bool,
    /// Every message the call sent is kept, so a next attempt sends them all.
    pub(crate) whole: bool,
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
                let fits = !kept.committed
                    && kept
                        .bytes
                        .checked_add(message.len())
                        .is_some_and(|after| after <= kept.call_limit)
                    && kept.channel.reserve(message.len());
                if fits {
                    // Shared with the encoder, which only reads it; its length is what the
                    // budget is charged.
                    kept.messages.push(message.clone());
                    kept.bytes += message.len();
                    kept.replayed += 1;
                } else {
                    kept.commit();
                    kept.lost = true;
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

    /// What was kept is what a second attempt sends, and the channel's total counts it until the
    /// call is committed.
    #[tokio::test]
    async fn a_second_attempt_replays_what_the_first_sent() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, None, closed);
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
        assert!(!replay.supersede().whole, "nothing is kept");
    }

    /// What a replay sends is the message the first attempt sent, not a copy of it.
    #[tokio::test]
    async fn a_replay_keeps_the_message_rather_than_a_copy() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, None, closed);
        let (mut send, _recv, _control) = call.split();
        let replay = Replay::new(live, Some(64), Arc::new(ChannelReplay::new(1024)));

        let message = Bytes::from(b"one".to_vec());
        send.send_message(message.clone()).await.expect("sent");
        let mut first = replay.attempt();
        assert_eq!(
            first.next().await.map(|sent| sent.as_ptr()),
            Some(message.as_ptr())
        );
        drop(send);

        let mut second = replay.attempt();
        assert_eq!(
            second.next().await.map(|kept| kept.as_ptr()),
            Some(message.as_ptr())
        );
    }

    /// A head that arrives while an attempt is still replaying commits the call, and the attempt
    /// still sends what was kept before the messages that follow.
    #[tokio::test]
    async fn a_commit_during_a_replay_lets_the_replay_finish() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, None, closed);
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
        assert_eq!(channel.used(), 6, "what was kept is still being sent");
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
        let (call, live, _driving) = create(4, None, closed);
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
        let (call, live, _driving) = create(4, None, closed);
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
            let (call, live, _driving) = create(4, None, closed);
            let (mut send, _recv, _control) = call.split();
            let channel = Arc::new(ChannelReplay::new(channel_limit));
            let replay = Replay::new(live, Some(call_limit), Arc::clone(&channel));

            send.send_message(Bytes::from_static(b"12345"))
                .await
                .expect("sent");
            let mut first = replay.attempt();
            assert_eq!(first.next().await, Some(Bytes::from_static(b"12345")));
            assert_eq!(channel.used(), 0);
            let standing = replay.supersede();
            assert!(!standing.retryable, "{call_limit}/{channel_limit}");
            assert!(!standing.whole, "{call_limit}/{channel_limit}");
        }
    }

    /// A call with no policy keeps nothing: it is whole until it sends a message, and then not.
    #[tokio::test]
    async fn a_call_with_no_policy_is_whole_until_it_sends() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, None, closed);
        let (mut send, _recv, _control) = call.split();
        let channel = Arc::new(ChannelReplay::new(1024));
        let replay = Replay::new(live, None, Arc::clone(&channel));

        let standing = replay.supersede();
        assert!(standing.whole && !standing.retryable);
        send.send_message(Bytes::from_static(b"sent"))
            .await
            .expect("sent");
        let mut first = replay.attempt();
        assert_eq!(first.next().await, Some(Bytes::from_static(b"sent")));
        assert!(!replay.supersede().whole);
        assert_eq!(channel.used(), 0);
    }
}
