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
use super::cause::Cause;
use super::error::GrpcChannelConfigError;
use super::status::GrpcStatusCode;
use crate::metrics::Metrics;

/// When a failed call is sent again.
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
    /// The failures a call is tried again for. Empty retries nothing. A call is not tried again for
    /// what the engine ended or refused itself, whatever the list names.
    pub failures: Vec<Cause>,
}

/// What a channel keeps of the messages its calls sent, so that a call can be sent again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReplayConfig {
    /// The bytes one call may keep; a call that sends more is not sent again.
    pub call_bytes: usize,
    /// The bytes all of a channel's calls may keep together; a call whose message would pass it is
    /// not sent again.
    pub channel_bytes: usize,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            call_bytes: 1024 * 1024,
            channel_bytes: 16 * 1024 * 1024,
        }
    }
}

/// The failures worth another try by default: `UNAVAILABLE` from the server, and a dial or a
/// connection that failed. `google.rpc.Code` advises UNAVAILABLE alone for retrying the same call.
pub fn default_failures() -> Vec<Cause> {
    vec![
        Cause::Status(GrpcStatusCode::Unavailable),
        Cause::Dial,
        Cause::Connection,
    ]
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_secs(120),
            backoff_multiplier: 2.0,
            failures: default_failures(),
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
    /// Where a call that outgrows a ceiling is counted.
    metrics: Metrics,
}

impl ChannelReplay {
    #[cfg(test)]
    pub(crate) fn new(limit: usize) -> Self {
        Self::counting_in(limit, Metrics::new())
    }

    pub(crate) fn counting_in(limit: usize, metrics: Metrics) -> Self {
        Self {
            used: AtomicUsize::new(0),
            limit,
            metrics,
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
    /// `live`, kept up to `call_limit` bytes and the channel's total, whether or not the call has a
    /// retry policy: a call that sends past either is committed. A limit of 0 keeps nothing.
    pub(crate) fn new(
        live: RequestMessages,
        call_limit: usize,
        channel: Arc<ChannelReplay>,
    ) -> Self {
        Self(Arc::new(Mutex::new(Kept {
            live,
            messages: Vec::new(),
            bytes: 0,
            call_limit,
            channel,
            committed: false,
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

    /// Compresses the messages the call's attempts take from its stream from now on, which are
    /// the ones it sends after the first attempt has taken its turn.
    pub(crate) fn compress_with(&self, encoding: Option<super::compression::Encoding>) {
        self.kept().live.compress_with(encoding);
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
    /// A new attempt's request body, which the engine's own service puts below tonic's client.
    pub(crate) fn attempt(&self) -> RequestBody {
        match self {
            Self::Stream(replay) => RequestBody::Stream(replay.attempt()),
            Self::One(one) => RequestBody::Framed(one.request.clone()),
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

/// One attempt's request body, framed already: the call's stream of messages, or its one request.
pub(crate) enum RequestBody {
    Stream(AttemptMessages),
    Framed(Bytes),
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
    pub(crate) fn new(request: Bytes, call_limit: usize, channel: Arc<ChannelReplay>) -> Self {
        let len = request.len();
        let kept = len <= call_limit && channel.reserve(len);
        if !kept {
            channel.metrics.not_replayable();
        }
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

/// One attempt's framed messages, the stream its request body is made of.
pub(crate) struct AttemptMessages {
    kept: Arc<Mutex<Kept>>,
    attempt: u64,
}

impl AttemptMessages {
    /// Whether the call was stopped this side while its request was open: read on every path
    /// that ends the messages, a superseded attempt's included.
    pub(crate) fn cut(&self) -> bool {
        self.kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .live
            .cut()
    }
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
                let len = message.len();
                let message = message.into_body();
                let open = !kept.committed;
                let fits = open
                    && kept
                        .bytes
                        .checked_add(len)
                        .is_some_and(|after| after <= kept.call_limit)
                    && kept.channel.reserve(len);
                if fits {
                    // Shared with the request body, which only reads it.
                    kept.messages.push(message.clone());
                    kept.bytes += len;
                    kept.replayed += 1;
                } else {
                    // Counted once: a call committed by a head or by this is not open again.
                    if open {
                        kept.channel.metrics.not_replayable();
                    }
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
    use super::super::request::{FramedMessage, FRAME_PREFIX};

    /// A message as the request body carries it, its frame's prefix taken off.
    fn unframed(framed: Option<Bytes>) -> Option<Bytes> {
        framed.map(|framed| framed.slice(FRAME_PREFIX..))
    }

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
            [5000, 10000, 20000, 40000, 80000].map(Duration::from_millis)
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
        let replay = Replay::new(live, 64, Arc::clone(&channel));

        send.send_message(Bytes::from_static(b"one"))
            .await
            .expect("sent");
        let mut first = replay.attempt();
        assert_eq!(
            unframed(first.next().await),
            Some(Bytes::from_static(b"one"))
        );
        assert_eq!(channel.used(), 3);

        send.send_message(Bytes::from_static(b"two"))
            .await
            .expect("sent");
        drop(send);
        let mut second = replay.attempt();
        assert_eq!(first.next().await, None, "the first attempt reads no more");
        assert_eq!(
            unframed(second.next().await),
            Some(Bytes::from_static(b"one"))
        );
        assert_eq!(
            unframed(second.next().await),
            Some(Bytes::from_static(b"two"))
        );
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
        let replay = Replay::new(live, 64, Arc::new(ChannelReplay::new(1024)));

        let message = FramedMessage::copy_of(b"one").expect("a message");
        let framed = message.body().as_ptr();
        send.send_framed(message).await.expect("sent");
        let mut first = replay.attempt();
        assert_eq!(first.next().await.map(|sent| sent.as_ptr()), Some(framed));
        drop(send);

        let mut second = replay.attempt();
        assert_eq!(second.next().await.map(|kept| kept.as_ptr()), Some(framed));
    }

    /// A head that arrives while an attempt is still replaying commits the call, and the attempt
    /// still sends what was kept before the messages that follow.
    #[tokio::test]
    async fn a_commit_during_a_replay_lets_the_replay_finish() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, None, closed);
        let (mut send, _recv, _control) = call.split();
        let channel = Arc::new(ChannelReplay::new(1024));
        let replay = Replay::new(live, 64, Arc::clone(&channel));

        for message in [&b"one"[..], b"two"] {
            send.send_message(Bytes::from_static(message))
                .await
                .expect("sent");
        }
        let mut first = replay.attempt();
        first.next().await;
        first.next().await;

        let mut second = replay.attempt();
        assert_eq!(
            unframed(second.next().await),
            Some(Bytes::from_static(b"one"))
        );
        replay.commit();
        assert_eq!(channel.used(), 6, "what was kept is still being sent");
        assert_eq!(
            unframed(second.next().await),
            Some(Bytes::from_static(b"two"))
        );
        assert_eq!(channel.used(), 0, "and goes once it is");

        send.send_message(Bytes::from_static(b"three"))
            .await
            .expect("sent");
        assert_eq!(
            unframed(second.next().await),
            Some(Bytes::from_static(b"three"))
        );
    }

    /// A failed attempt waiting on the host's next message is woken when it is superseded, and
    /// ends, rather than taking that message from the attempt after it.
    #[tokio::test]
    async fn a_superseded_attempt_waiting_for_a_message_ends() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, None, closed);
        let (mut send, _recv, _control) = call.split();
        let channel = Arc::new(ChannelReplay::new(1024));
        let replay = Replay::new(live, 64, Arc::clone(&channel));

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
        assert_eq!(
            unframed(second.next().await),
            Some(Bytes::from_static(b"late"))
        );
    }

    /// A call that ends with no attempt to follow gives the channel back its share at once, even
    /// while a stream of it is still held.
    #[tokio::test]
    async fn a_replay_dropped_gives_its_bytes_back() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, None, closed);
        let (mut send, _recv, _control) = call.split();
        let channel = Arc::new(ChannelReplay::new(1024));
        let replay = Replay::new(live, 64, Arc::clone(&channel));

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
            let replay = Replay::new(live, call_limit, Arc::clone(&channel));

            send.send_message(Bytes::from_static(b"12345"))
                .await
                .expect("sent");
            let mut first = replay.attempt();
            assert_eq!(
                unframed(first.next().await),
                Some(Bytes::from_static(b"12345"))
            );
            assert_eq!(channel.used(), 0);
            let standing = replay.supersede();
            assert!(!standing.retryable, "{call_limit}/{channel_limit}");
            assert!(!standing.whole, "{call_limit}/{channel_limit}");
        }
    }

    /// A call whose ceiling is nothing keeps nothing: it can go again until it sends a message,
    /// and then it is committed and not whole.
    #[tokio::test]
    async fn a_call_with_a_ceiling_of_nothing_is_whole_until_it_sends() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = create(4, None, closed);
        let (mut send, _recv, _control) = call.split();
        let channel = Arc::new(ChannelReplay::new(1024));
        let replay = Replay::new(live, 0, Arc::clone(&channel));

        let standing = replay.supersede();
        assert!(standing.whole && standing.retryable);
        send.send_message(Bytes::from_static(b"sent"))
            .await
            .expect("sent");
        let mut first = replay.attempt();
        assert_eq!(
            unframed(first.next().await),
            Some(Bytes::from_static(b"sent"))
        );
        let standing = replay.supersede();
        assert!(!standing.whole && !standing.retryable);
        assert_eq!(channel.used(), 0);
    }
}
