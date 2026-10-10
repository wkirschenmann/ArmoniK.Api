use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::Poll;

use bytes::{Buf, BufMut, Bytes};
use http::uri::PathAndQuery;
use http::HeaderMap;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;
use tonic::codec::{BufferSettings, Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::metadata::MetadataMap;
use tonic::Code;

use super::call::{
    Answered, CallControl, HeadOrigin, OwnedMessage, ReadGate, RequestMessages, ResponseHead,
    ResponseSink,
};
use super::cause;
use super::channel::Inner;
use super::compression::{compressed, CompressionBudget, Encoding};
use super::contained::contained;
use super::metadata::Metadata;
use super::origin::{Origin, Pushback};
use super::request::{FramedMessage, RequestSlot};
use super::retry::{jittered, OneReplay, Replay, RequestBody, Sent};
use super::status::GrpcStatusCode;
use super::status::{Failure, GrpcStatus, Unprocessed};
use crate::metrics::{CallCounters, CallGuard};

pub(crate) struct Driving<S> {
    stop: Stop,
    sink: S,
    control: CallControl,
}

impl<S> Driving<S> {
    /// What the call counts.
    pub(crate) fn counters(&self) -> CallCounters {
        self.control.counters.clone()
    }
}

impl Driving<()> {
    pub(crate) fn new(
        over: watch::Receiver<bool>,
        channel_closed: watch::Receiver<bool>,
        control: CallControl,
    ) -> Self {
        Self {
            stop: Stop {
                over,
                channel_closed,
                cut: control.cut.clone(),
            },
            sink: (),
            control,
        }
    }

    pub(crate) fn with_sink<S: ResponseSink>(self, sink: S) -> Driving<S> {
        Driving {
            stop: self.stop,
            sink,
            control: self.control,
        }
    }
}

/// The sink, and what the driver knows of the response it hands on.
struct Responding<S> {
    sink: S,
    /// Marked by the last attempt's service once a response came, which decides the head when
    /// none was given.
    answered: Answered,
    head_given: bool,
    counters: CallCounters,
}

/// What a call sends: a stream of messages the caller writes, or its one request, given once.
pub(crate) enum Sending {
    Stream(RequestMessages),
    One(RequestSlot),
}

/// What a call sends: where, with what metadata, and the messages the caller will write - and
/// when it stops waiting for the answer.
pub(crate) struct Outgoing {
    pub(crate) path: PathAndQuery,
    pub(crate) metadata: HeaderMap,
    pub(crate) messages: Sending,
    pub(crate) deadline: Option<Instant>,
    pub(crate) read_gate: Option<Arc<dyn ReadGate>>,
    pub(crate) one_response: bool,
    pub(crate) wait_for_ready: bool,
    /// What the compressed copy of a call's one request is counted against.
    pub(crate) compression_budget: Option<Arc<dyn CompressionBudget>>,
    /// The call's place in the registry of its channel's stats, ended by its driver with the
    /// status the call ends with, or as a cancelled call when the driver is dropped.
    pub(crate) guard: Option<CallGuard>,
}

pub(crate) async fn drive<S: ResponseSink>(
    inner: Arc<Inner>,
    mut outgoing: Outgoing,
    driving: Driving<S>,
) {
    let Driving {
        mut stop,
        sink,
        control,
    } = driving;
    let guard = outgoing.guard.take();
    let mut responding = Responding {
        sink,
        answered: Answered::default(),
        head_given: false,
        counters: control.counters.clone(),
    };

    // Held for the whole call, not released after the head. `Inner` owns the `closed` sender,
    // and a watch receiver whose senders are all gone answers like one that was told to close - so
    // a driver that let go of it would read a dropped channel handle as a cancellation.
    //
    // Contained, because a panic unwinding past the sink would drop the terminal unsent, and the
    // caller would read a driver gone with its runtime rather than a call that failed.
    let status = contained(within_deadline(
        &inner,
        outgoing,
        &mut stop,
        &mut responding,
    ))
    .await
    .unwrap_or_else(|| GrpcStatus::new(Code::Internal, "the task driving the call panicked"));
    // A refusal stops the call as a cancellation does, and is what the call ends with.
    let status = match control.refusal() {
        Some(refused) if status.code == GrpcStatusCode::Cancelled => refused,
        _ => status,
    };

    // Before the terminal, not after: a send admitted between the two would be queued for a driver
    // that has stopped, and the caller would be told it was sent.
    control.finish();
    // Before the terminal, so that a host that reads the stats on seeing it finds the call ended.
    if let Some(guard) = guard {
        guard.end(status.code);
    }

    // A head never given goes out with the end, empty: `TrailersOnly` if a response came,
    // `NoResponse` if none did.
    let head = (!responding.head_given).then(|| ResponseHead {
        metadata: Metadata::new(),
        origin: if responding.answered.marked() {
            HeadOrigin::TrailersOnly
        } else {
            HeadOrigin::NoResponse
        },
    });
    responding.sink.end(status, head).await;
}

struct Stop {
    over: watch::Receiver<bool>,
    channel_closed: watch::Receiver<bool>,
    cut: Arc<AtomicBool>,
}

impl Stop {
    /// Completes when the call is stopped this side, and marks it cut before the work it stops
    /// lets go of the request.
    async fn stopped(&mut self) {
        let Self {
            over,
            channel_closed,
            cut,
        } = self;
        tokio::select! {
            _ = over.wait_for(|over| *over) => {}
            _ = channel_closed.wait_for(|closed| *closed) => {}
        }
        cut.store(true, Ordering::Release);
    }
}

/// Marks the call cut when `run` is left with the peer's response not ended, or by a panic's
/// unwinding.
///
/// A status `run` gives then is not the peer's - a message the decoder refuses, a second one on a
/// call that answers once, a sink that refuses, a connection lost - and ends the call as a cancel
/// does. `Answered::ended` is what tells the peer's status from these: tonic's reading of the
/// response returns an error for the peer's non-OK trailers as well as for a refusal of its own,
/// but a peer's end has been read off the wire by the time it does.
struct CutOnExit {
    cut: Arc<AtomicBool>,
    answered: Answered,
}

impl Drop for CutOnExit {
    fn drop(&mut self) {
        if std::thread::panicking() || !self.answered.ended() {
            self.cut.store(true, Ordering::Release);
        }
    }
}

/// The work, unless the call or its channel is already over - which wins when both are ready.
async fn until_stopped<T>(stop: &mut Stop, work: impl Future<Output = T>) -> Option<T> {
    tokio::select! {
        biased;
        _ = stop.stopped() => None,
        value = work => Some(value),
    }
}

/// The three things a call hands a `RecvHalf`, each on its own channel.
///
/// The terminal has one of its own because it is the only one that must arrive. Sharing the
/// message queue would mean waiting for room in it, and that wait would need a bound: a channel
/// closing while one message sits unread would take the terminal with it, and a call the peer
/// answered OK would reach its reader as `Aborted`.
pub(crate) struct Delivery {
    head: Option<oneshot::Sender<ResponseHead>>,
    messages: mpsc::Sender<OwnedMessage>,
    terminal: Option<oneshot::Sender<GrpcStatus>>,
}

impl Delivery {
    pub(crate) fn new(
        head: oneshot::Sender<ResponseHead>,
        messages: mpsc::Sender<OwnedMessage>,
        terminal: oneshot::Sender<GrpcStatus>,
    ) -> Self {
        Self {
            head: Some(head),
            messages,
            terminal: Some(terminal),
        }
    }

    fn give_head(&mut self, head: ResponseHead) {
        if let Some(sender) = self.head.take() {
            let _ = sender.send(head);
        }
    }
}

impl ResponseSink for Delivery {
    async fn head(&mut self, head: ResponseHead) -> Result<(), GrpcStatus> {
        self.give_head(head);
        Ok(())
    }

    async fn message(&mut self, data: Bytes) -> Result<(), GrpcStatus> {
        self.messages
            .send(OwnedMessage { data })
            .await
            .map_err(|_| GrpcStatus::cancelled())
    }

    fn flush(&mut self) {}

    const GATHERS: bool = false;

    /// Publishes the status and lets the message queue end.
    ///
    /// Nothing to wait for: the reader takes what is queued, finds the sender gone, and reads the
    /// terminal here. `self` by value, so the queue closes when this returns even on the paths
    /// that never got a status out.
    async fn end(mut self, status: GrpcStatus, head: Option<ResponseHead>) {
        if let Some(head) = head {
            self.give_head(head);
        }
        if let Some(terminal) = self.terminal.take() {
            let _ = terminal.send(status);
        }
    }
}

/// Eight digits of hours, the largest `grpc-timeout` there is.
const LARGEST_GRPC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(99_999_999 * 3600);

/// The call, ended `DEADLINE_EXCEEDED` when its deadline passes first. Ending it drops the
/// stream, which hyper resets with `CANCEL`, as a cancellation does.
///
/// A deadline already past sends nothing: the peer would be told a timeout of zero, and the
/// answer is known without it.
async fn within_deadline<S: ResponseSink>(
    inner: &Arc<Inner>,
    outgoing: Outgoing,
    stop: &mut Stop,
    responding: &mut Responding<S>,
) -> GrpcStatus {
    let deadline = outgoing.deadline;
    if deadline.is_some_and(|deadline| deadline <= Instant::now()) {
        return GrpcStatus::deadline_exceeded();
    }
    let cut = stop.cut.clone();
    let run = run(inner, outgoing, stop, responding);
    tokio::pin!(run);
    match deadline {
        None => run.await,
        Some(deadline) => tokio::select! {
            status = &mut run => status,
            () = tokio::time::sleep_until(deadline) => {
                // Marked while `run` still holds the request: dropping it lets the request go.
                cut.store(true, Ordering::Release);
                GrpcStatus::deadline_exceeded()
            }
        },
    }
}

/// The header that tells the server how many attempts went before this one.
const PREVIOUS_ATTEMPTS: &str = "grpc-previous-rpc-attempts";

/// The call's attempts: the first, then as many more as its retry policy allows while each
/// fails with a code it names and with what it sent still kept for the replay, which a head
/// reaching the reader ends.
async fn run<S: ResponseSink>(
    inner: &Arc<Inner>,
    outgoing: Outgoing,
    stop: &mut Stop,
    responding: &mut Responding<S>,
) -> GrpcStatus {
    #[cfg(feature = "test-hooks")]
    crate::hooks::run_in_driver();

    let Outgoing {
        path,
        metadata,
        messages,
        deadline,
        read_gate,
        one_response,
        wait_for_ready,
        compression_budget,
        guard: _,
    } = outgoing;

    let policy = inner.retry.as_ref();
    let replay_limit = inner.call_replay_bytes;
    // What the call sent, kept for the attempts after the first. A stream's is made now; the
    // one request is held, whole and as its caller wrote it, until the first attempt has its turn.
    let mut replay: Option<Sent> = None;
    let mut held: Option<FramedMessage> = None;
    match messages {
        Sending::Stream(messages) => {
            replay = Some(Sent::Stream(Replay::new(
                messages,
                replay_limit,
                Arc::clone(&inner.replay),
            )));
        }
        // Nothing goes out, not even the request's head, until the request is in: a call that ends
        // first has sent nothing its peer could act on.
        Sending::One(slot) => match until_stopped(stop, slot.taken()).await {
            Some(Some(request)) => {
                let len = request.len();
                if let Some(max) = inner.max_send_message_size.filter(|max| len > *max) {
                    return GrpcStatus::new(
                        GrpcStatusCode::ResourceExhausted,
                        format!("a message of {len} bytes is past the {max} the channel sends"),
                    );
                }
                held = Some(request);
            }
            None | Some(None) => return GrpcStatus::cancelled(),
        },
    }
    // What the call's messages are compressed with, and the header says: the channel's, as it
    // stands when the first attempt has taken its turn, and the call's for every attempt after.
    let mut encoding: Option<Option<Encoding>> = None;
    // After the replay, so dropped before it: the call is marked cut before its end, or a panic's
    // unwinding, lets go of the request.
    let _cut_on_exit = CutOnExit {
        cut: stop.cut.clone(),
        answered: responding.answered.clone(),
    };
    let mut bound = policy
        .map(|policy| policy.initial_backoff)
        .unwrap_or_default();
    let mut previous = 0u32;
    let mut unsent_again = false;
    let mut refused_again = false;
    // Whether the attempt about to start is the policy's retry, which takes no turn: it is sent
    // only while `retries_open` finds the server accepting what is sent.
    let mut retrying = false;
    loop {
        // Before the attempt reads what is left of the deadline, so that `grpc-timeout` states what
        // remains after the wait. A first attempt, and a resend of a request its peer never
        // processed, wait their turn; a call whose deadline passes while it waits ends
        // DEADLINE_EXCEEDED.
        if !std::mem::take(&mut retrying) {
            // A call that waits for a connection takes its turn once there is one, when it has a
            // turn to wait for, so that the calls waiting for a connection hold none, and do not
            // all start together when it opens. A connection that fails in between makes the
            // request a resend, with a turn of its own.
            if wait_for_ready
                && inner.admission.may_wait()
                && until_stopped(stop, inner.connected(&responding.counters))
                    .await
                    .is_none()
            {
                return GrpcStatus::cancelled();
            }
            if until_stopped(stop, inner.admission.first_attempt())
                .await
                .is_none()
            {
                return GrpcStatus::cancelled();
            }
        }
        let Ended {
            status,
            pushback,
            unprocessed,
            origin,
        } = {
            let first = encoding.is_none();
            let chosen = *encoding.get_or_insert_with(|| inner.send.now());
            if let Some(Sent::Stream(stream)) = &replay {
                if first {
                    stream.compress_with(chosen);
                }
            } else if let Some(request) = held.take() {
                // Compressed once, here: every attempt sends the same bytes, and the replay
                // is charged what is sent.
                let raw = request.len();
                let request = match chosen {
                    Some(encoding) => {
                        let compressing =
                            compressed(encoding, request, compression_budget.as_ref());
                        match until_stopped(stop, compressing).await {
                            Some(request) => request,
                            None => return GrpcStatus::cancelled(),
                        }
                    }
                    None => request,
                };
                responding.counters.message_sent(raw, request.len());
                replay = Some(Sent::One(OneReplay::new(
                    request.body(),
                    replay_limit,
                    Arc::clone(&inner.replay),
                )));
            }
            let sent = replay
                .as_ref()
                .expect("the replay is made by the first attempt");
            let mut headers = metadata.clone();
            if previous > 0 {
                headers.insert(PREVIOUS_ATTEMPTS, previous.into());
            }
            let ended = attempt(
                inner,
                path.clone(),
                headers,
                sent.attempt(),
                deadline,
                read_gate.as_deref(),
                one_response,
                wait_for_ready,
                chosen,
                sent,
                stop,
                responding,
            )
            .await;
            ended.report();
            inner
                .admission
                .record(&ended.origin, ended.status.code, ended.pushback);
            ended
        };
        // gRFC A6's transparent retry: a request the peer's application never saw goes again at
        // once, whatever the policy, and counts as no attempt. Once a call for each way of not
        // being seen, so that a GOAWAY and the request it leaves unsent are both covered, and a
        // peer that refuses every stream, or drops every connection, meets the policy's backoff
        // and its count rather than a loop of dials: a further GOAWAY or unsent request is the
        // connection's end, and a further refusal is a reset of `REFUSED_STREAM`, which the
        // policy's list names or does not.
        let again = match unprocessed {
            Some(Unprocessed::Unsent) => !std::mem::replace(&mut unsent_again, true),
            Some(Unprocessed::RefusedStream | Unprocessed::GoAway) => {
                !std::mem::replace(&mut refused_again, true)
            }
            None => false,
        };
        let sent = replay
            .as_ref()
            .expect("an attempt went out before one could fail");
        if again && sent.supersede().whole {
            tracing::debug!(
                method = %path,
                reason = unprocessed.map(|reason| reason.to_string()).unwrap_or_default(),
                "the request never reached the peer's application, and is sent again"
            );
            inner.metrics.resend();
            continue;
        }
        previous += 1;

        let Some(policy) = policy else {
            return status;
        };
        // What the engine refused or ended itself is not the server's to try again, whatever code it
        // carries: it would fail the same way, or the caller has ended it.
        let retryable = status.code != GrpcStatusCode::Ok
            && origin != Origin::Local
            && cause::retried(&policy.failures, &origin, status.code, pushback)
            && previous < policy.max_attempts;
        if !retryable {
            return status;
        }
        // The estimate has this attempt counted. A retry is judged worth sending while the server
        // accepts what is sent, here and again once the backoff has passed, so that a call that
        // will not be retried does not sleep first.
        if !inner.admission.retries_open() {
            inner.metrics.retry_refused();
            return status;
        }
        let wait = match pushback {
            Pushback::Refused => return status,
            Pushback::After(wait) => {
                bound = policy.initial_backoff;
                wait
            }
            Pushback::Unsaid => {
                let wait = jittered(bound);
                bound = policy.next_bound(bound);
                wait
            }
        };
        // A backoff the deadline would cut short ends the call with what it failed with, not
        // with a DEADLINE_EXCEEDED the wait would earn it. A wait past what the clock holds is
        // one no deadline outlasts.
        if deadline.is_some_and(|deadline| {
            Instant::now()
                .checked_add(wait)
                .is_none_or(|at| at >= deadline)
        }) {
            return status;
        }
        // Before the wait, so the failed attempt's stream reads nothing the host sends meanwhile;
        // under the same lock as that stream's last commit, so a call it committed is not retried.
        if !sent.supersede().retryable {
            return status;
        }
        tracing::debug!(
            method = %path,
            attempt = previous,
            code = ?status.code,
            wait_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
            "the call failed and is retried after a backoff"
        );
        if until_stopped(stop, tokio::time::sleep(wait))
            .await
            .is_none()
        {
            return GrpcStatus::cancelled();
        }
        if !inner.admission.retries_open() {
            inner.metrics.retry_refused();
            return status;
        }
        inner
            .metrics
            .retry(|| cause::retry_slot(&policy.failures, &origin, status.code));
        retrying = true;
    }
}

/// How an attempt ended, where that came from, and what its peer said of another.
struct Ended {
    status: GrpcStatus,
    pushback: Pushback,
    /// The peer's application never saw the request.
    unprocessed: Option<Unprocessed>,
    origin: Origin,
}

impl Ended {
    /// An end that is the engine's own: a cancel, a limit, a refusing sink.
    fn local(status: GrpcStatus) -> Self {
        Self {
            status,
            pushback: Pushback::Unsaid,
            unprocessed: None,
            origin: Origin::Local,
        }
    }

    /// The server's status, in trailers or in a Trailers-Only head.
    fn server(status: GrpcStatus, pushback: Pushback) -> Self {
        Self {
            status,
            pushback,
            unprocessed: None,
            origin: Origin::Server,
        }
    }

    /// A status tonic returned. What the engine marked it with says where it came from; one it
    /// did not mark is the server's when the peer stated a status that tonic read, and the engine's
    /// or tonic's own when it did not.
    fn of(status: tonic::Status, peer_stated: bool) -> Self {
        let pushback = Pushback::of(&status.metadata().clone().into_headers());
        let failure = Failure::marked(&status);
        Self {
            origin: match &failure {
                Some(failure) => failure.origin.clone(),
                None if peer_stated => Origin::Server,
                None => Origin::Local,
            },
            unprocessed: failure.and_then(|failure| failure.unprocessed),
            pushback,
            status: GrpcStatus::from(status),
        }
    }

    /// Tells the log, and a test that listens, of an attempt that went out and ended.
    fn report(&self) {
        if self.status.code != GrpcStatusCode::Ok {
            tracing::debug!(
                target: "armonik_transport",
                code = ?self.status.code,
                origin = %self.origin.describe(),
                "an attempt failed"
            );
        }
        #[cfg(feature = "test-hooks")]
        crate::hooks::attempt_ended(&self.origin, self.status.code, self.pushback);
    }
}

/// One attempt, and what its server said of a retry when it failed before its head.
#[allow(clippy::too_many_arguments)]
async fn attempt<S: ResponseSink>(
    inner: &Arc<Inner>,
    path: PathAndQuery,
    metadata: HeaderMap,
    body: RequestBody,
    deadline: Option<Instant>,
    read_gate: Option<&dyn ReadGate>,
    one_response: bool,
    wait_for_ready: bool,
    encoding: Option<Encoding>,
    replay: &Sent,
    stop: &mut Stop,
    responding: &mut Responding<S>,
) -> Ended {
    // tonic encodes no message: the body is the engine's, framed already, put below tonic's
    // client by the channel's own service.
    let mut request = tonic::Request::new(tonic::codegen::tokio_stream::empty::<Bytes>());
    *request.metadata_mut() = MetadataMap::from_headers(metadata);
    // What is left of it before any dial, which tonic writes as `grpc-timeout` in the unit that
    // keeps it within the eight digits the protocol allows. The dial makes it slightly long, and
    // this side's timer, which ends the call first, is the one that holds - as it does for a
    // deadline past the largest the header can state, which tonic would panic on.
    if let Some(deadline) = deadline {
        request.set_timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .min(LARGEST_GRPC_TIMEOUT),
        );
    }

    // Each attempt's own: whether a response came, and ended, is the last attempt's to say.
    responding.answered.reset();
    let mut client = inner.client(
        responding.answered.clone(),
        one_response,
        wait_for_ready,
        body,
        encoding,
        responding.counters.clone(),
    );
    let response = match until_stopped(stop, client.streaming(request, path, BytesCodec)).await {
        None => return Ended::local(GrpcStatus::cancelled()),
        Some(Err(status)) => return Ended::of(status, responding.answered.stated()),
        Some(Ok(response)) => response,
    };

    let (head, mut body, _) = response.into_parts();
    let head = head.into_headers();

    // A head that states a status is the Trailers-Only shape, where that one HEADERS frame is the
    // trailers and not initial metadata. tonic ends such a stream empty and leaves the status in
    // the head, so it is read from there, and nothing goes out as a head: delivering those
    // headers twice would have the reader see a head no such response has.
    if let Some(status) = tonic::Status::from_header_map(&head) {
        return Ended::server(GrpcStatus::from(status), Pushback::of(&head));
    }
    // The reader has a head: whatever follows, this call is not tried again.
    replay.commit();
    responding.head_given = true;
    let head = ResponseHead {
        metadata: Metadata::from_headers(&head),
        origin: HeadOrigin::Wire,
    };
    // Given whatever the stop says: once a call has a head, the caller hears it before the end.
    match responding.sink.head(head).await {
        Err(status) => Ended::local(status),
        Ok(()) => {
            finish(
                stop,
                &mut responding.sink,
                &responding.answered,
                &responding.counters,
                read_gate,
                one_response,
                inner.delivery_coalescing,
                &mut body,
            )
            .await
        }
    }
}

/// What the next read off the response found.
enum Read {
    Message(Bytes),
    End(Ended),
}

/// The response's messages and trailers, once its head is given.
///
/// Each read is polled once first: what is already there is read at once. What is not there yet
/// is waited for one round of the runtime while the delivery holds less than `coalescing` bytes,
/// as the connection's writes are held: the connection shares this thread, so what it decodes on
/// its next turn - a head's message, a message's trailers - joins what the sink holds rather than
/// following it in a callback of its own. A round that brings the read earns the next one; only
/// one that brings nothing tells the sink that nothing more is ready, before the wait.
#[allow(clippy::too_many_arguments)]
async fn finish<S: ResponseSink>(
    stop: &mut Stop,
    sink: &mut S,
    answered: &Answered,
    counters: &CallCounters,
    read_gate: Option<&dyn ReadGate>,
    one_response: bool,
    coalescing: usize,
    body: &mut tonic::Streaming<Bytes>,
) -> Ended {
    let coalescing = if S::GATHERS { coalescing } else { 0 };
    let mut message_read = false;
    // The bytes of the messages read since the sink was last told to deliver, or none once it
    // was: the head comes first, held with no message. More than the sink holds once a full
    // delivery window has made it deliver on its own.
    let mut held = Some(0);
    loop {
        let turn_only = one_response && message_read;
        let read = {
            let mut next = pin!(until_stopped(
                stop,
                read_next(answered, read_gate, turn_only, body)
            ));
            let mut polled = std::future::poll_fn(|cx| Poll::Ready(next.as_mut().poll(cx))).await;
            if polled.is_pending() && held.is_some_and(|bytes| bytes < coalescing) {
                #[cfg(feature = "test-hooks")]
                crate::hooks::count_delivery_round();
                tokio::task::yield_now().await;
                polled = std::future::poll_fn(|cx| Poll::Ready(next.as_mut().poll(cx))).await;
            }
            match polled {
                Poll::Ready(read) => read,
                Poll::Pending => {
                    sink.flush();
                    held = None;
                    next.await
                }
            }
        };
        let message = match read {
            None => return Ended::local(GrpcStatus::cancelled()),
            Some(Read::End(ended)) => return ended,
            Some(Read::Message(message)) => message,
        };
        message_read = true;
        counters.message_received();
        let bytes = message.len();
        match until_stopped(stop, sink.message(message)).await {
            None => return Ended::local(GrpcStatus::cancelled()),
            Some(Err(status)) => return Ended::local(status),
            Some(Ok(())) => held = Some(held.unwrap_or(0) + bytes),
        }
    }
}

async fn read_next(
    answered: &Answered,
    read_gate: Option<&dyn ReadGate>,
    turn_only: bool,
    body: &mut tonic::Streaming<Bytes>,
) -> Read {
    // Before the read, not after the message: a call the gate holds pulls nothing off the stream,
    // so flow control holds its peer and nothing is decoded that the gate refused.
    if let Some(gate) = read_gate {
        if turn_only {
            gate.turn().await;
        } else {
            gate.admitted().await;
        }
    }
    match body.message().await {
        Err(status) => Read::End(Ended::of(past_the_limit(status), answered.stated())),
        Ok(Some(message)) => Read::Message(message),
        Ok(None) => Read::End(match body.trailers().await {
            Err(status) => Ended::of(status, answered.stated()),
            Ok(trailers) => {
                let trailers = trailers.map(MetadataMap::into_headers).unwrap_or_default();
                Ended::server(GrpcStatus::ok(&trailers), Pushback::of(&trailers))
            }
        }),
    }
}

/// RESOURCE_EXHAUSTED for a message past the limit, which is the gRPC status table's code for it.
///
/// tonic's decoder answers OUT_OF_RANGE, a code that table says the library never generates, and
/// its message is the only thing telling that refusal from a status the peer sent.
/// `a_message_past_the_maximum_ends_the_call_rather_than_being_held` fails if the wording moves.
fn past_the_limit(status: tonic::Status) -> tonic::Status {
    const TONIC_SAYS: &str = "Error, decoded message length too large";

    if status.code() == Code::OutOfRange && status.message().starts_with(TONIC_SAYS) {
        let mut refused = tonic::Status::resource_exhausted(status.message());
        refused.set_source(Arc::new(Failure::of(Origin::Local)));
        refused
    } else {
        status
    }
}

/// Room for a small message and its prefix; a larger one has the buffer grown to fit it. Not 0: tonic
/// divides by it when it decompresses.
const CODEC_BUFFER: usize = 1024;

/// Unused: `BufferSettings` asks for an encoder's threshold, and the engine encodes nothing.
const YIELD_THRESHOLD: usize = 32 * 1024;

/// Messages as the caller's bytes, both ways: the engine serializes nothing of its own.
#[derive(Clone, Copy, Debug, Default)]
struct BytesCodec;

impl Codec for BytesCodec {
    type Encode = Bytes;
    type Decode = Bytes;
    type Encoder = Self;
    type Decoder = Self;

    fn encoder(&mut self) -> Self::Encoder {
        *self
    }

    fn decoder(&mut self) -> Self::Decoder {
        *self
    }
}

impl Encoder for BytesCodec {
    type Item = Bytes;
    type Error = tonic::Status;

    /// Never fed: a request's messages reach the wire framed by the engine, below tonic.
    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        dst.put(item);
        Ok(())
    }

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::new(CODEC_BUFFER, YIELD_THRESHOLD)
    }
}

impl Decoder for BytesCodec {
    type Item = Bytes;
    type Error = tonic::Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        Ok(Some(src.copy_to_bytes(src.remaining())))
    }

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::new(CODEC_BUFFER, YIELD_THRESHOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::call::{create, RecvResult};
    use super::super::status::GrpcStatusCode;

    /// The queue is full, the channel is closed, and the peer's status still reaches the reader.
    ///
    /// Both halves matter: the message the driver has already queued, and the terminal behind it.
    /// On one queue the terminal would wait for room, and the only thing able to end that wait is
    /// the channel closing - which discards a status the peer has given.
    #[tokio::test]
    async fn a_status_the_peer_gave_outlives_the_channel_that_carried_it() {
        let (closed, closed_rx) = watch::channel(false);
        let (call, _messages, driving) = create(1, None, closed_rx);
        let (_send, mut recv, _control) = call.split();

        let Driving {
            sink: mut delivery, ..
        } = driving;
        assert!(
            delivery
                .message(Bytes::from_static(b"queued"))
                .await
                .is_ok(),
            "a window of one takes the first message"
        );

        closed.send_replace(true);
        delivery
            .end(GrpcStatus::new(GrpcStatusCode::Ok, ""), None)
            .await;

        assert!(matches!(
            recv.next_message().await,
            Ok(RecvResult::Message(_))
        ));
        assert!(matches!(
            recv.next_message().await,
            Ok(RecvResult::End(status)) if status.code == GrpcStatusCode::Ok
        ));
    }
}
