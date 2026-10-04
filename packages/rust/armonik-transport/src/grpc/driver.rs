use std::future::Future;
use std::pin::pin;
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
use super::channel::Inner;
use super::contained::contained;
use super::metadata::Metadata;
use super::retry::{jittered, AttemptMessages, Replay};
use super::status::GrpcStatusCode;
use super::status::{GrpcStatus, Unprocessed};

pub(crate) struct Driving<S> {
    stop: Stop,
    sink: S,
    control: CallControl,
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
}

/// What a call sends: where, with what metadata, and the messages the caller will write - and
/// when it stops waiting for the answer.
pub(crate) struct Outgoing {
    pub(crate) path: PathAndQuery,
    pub(crate) metadata: HeaderMap,
    pub(crate) messages: RequestMessages,
    pub(crate) deadline: Option<Instant>,
    pub(crate) read_gate: Option<Arc<dyn ReadGate>>,
}

pub(crate) async fn drive<S: ResponseSink>(
    inner: Arc<Inner>,
    outgoing: Outgoing,
    driving: Driving<S>,
) {
    let Driving {
        mut stop,
        sink,
        control,
    } = driving;
    let mut responding = Responding {
        sink,
        answered: Answered::default(),
        head_given: false,
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

    // Before the terminal, not after: a send admitted between the two would be queued for a driver
    // that has stopped, and the caller would be told it was sent.
    control.cancel();

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
}

impl Stop {
    async fn stopped(&mut self) {
        let Self {
            over,
            channel_closed,
        } = self;
        tokio::select! {
            _ = over.wait_for(|over| *over) => {}
            _ = channel_closed.wait_for(|closed| *closed) => {}
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
    let Some(deadline) = outgoing.deadline else {
        return run(inner, outgoing, stop, responding).await;
    };
    if deadline <= Instant::now() {
        return GrpcStatus::deadline_exceeded();
    }
    tokio::time::timeout_at(deadline, run(inner, outgoing, stop, responding))
        .await
        .unwrap_or_else(|_| GrpcStatus::deadline_exceeded())
}

/// The header that tells the server how many attempts went before this one.
const PREVIOUS_ATTEMPTS: &str = "grpc-previous-rpc-attempts";

/// The trailer in which the server says how long to wait before a retry, or not to retry.
const PUSHBACK: &str = "grpc-retry-pushback-ms";

/// What a failed attempt's server said of a retry.
enum Pushback {
    /// Nothing: the backoff decides.
    Unsaid,
    /// Retry after this long.
    After(std::time::Duration),
    /// Do not retry: a negative or unreadable value, which gRFC A6 reads so.
    Refused,
}

impl Pushback {
    fn of(headers: &HeaderMap) -> Self {
        let Some(value) = headers.get(PUSHBACK) else {
            return Self::Unsaid;
        };
        match value
            .to_str()
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
        {
            Some(millis) => Self::After(std::time::Duration::from_millis(millis)),
            None => Self::Refused,
        }
    }
}

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
    } = outgoing;

    let policy = inner.retry.as_ref();
    let replay = Replay::new(
        messages,
        policy.map(|policy| policy.call_replay_bytes),
        Arc::clone(&inner.replay),
    );
    let mut bound = policy
        .map(|policy| policy.initial_backoff)
        .unwrap_or_default();
    let mut previous = 0u32;
    let mut unsent_again = false;
    let mut refused_again = false;
    loop {
        let mut headers = metadata.clone();
        if previous > 0 {
            headers.insert(PREVIOUS_ATTEMPTS, previous.into());
        }
        let Ended {
            status,
            pushback,
            unprocessed,
        } = attempt(
            inner,
            path.clone(),
            headers,
            replay.attempt(),
            deadline,
            read_gate.as_deref(),
            &replay,
            stop,
            responding,
        )
        .await;
        // gRFC A6's transparent retry: a request the peer's application never saw goes again at
        // once, whatever the policy, and counts as no attempt. Once a call for each way of not
        // being seen, so that a GOAWAY and the request it leaves unsent are both covered, and a
        // peer that refuses every stream, or drops every connection, meets the policy's backoff
        // and its count rather than a loop of dials.
        let again = match unprocessed {
            Some(Unprocessed::Unsent) => !std::mem::replace(&mut unsent_again, true),
            Some(Unprocessed::Refused) => !std::mem::replace(&mut refused_again, true),
            None => false,
        };
        if again && replay.supersede().whole {
            continue;
        }
        previous += 1;

        let Some(policy) = policy else {
            return status;
        };
        let retryable = status.code != GrpcStatusCode::Ok
            && policy.retryable_codes.contains(&status.code)
            && previous < policy.max_attempts;
        if !retryable {
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
        if !replay.supersede().retryable {
            return status;
        }
        if until_stopped(stop, tokio::time::sleep(wait))
            .await
            .is_none()
        {
            return GrpcStatus::cancelled();
        }
    }
}

/// How an attempt ended, and what its peer said of another.
struct Ended {
    status: GrpcStatus,
    pushback: Pushback,
    /// The peer's application never saw the request.
    unprocessed: Option<Unprocessed>,
}

impl Ended {
    fn with(status: GrpcStatus, pushback: Pushback) -> Self {
        Self {
            status,
            pushback,
            unprocessed: None,
        }
    }
}

/// One attempt, and what its server said of a retry when it failed before its head.
#[allow(clippy::too_many_arguments)]
async fn attempt<S: ResponseSink>(
    inner: &Arc<Inner>,
    path: PathAndQuery,
    metadata: HeaderMap,
    messages: AttemptMessages,
    deadline: Option<Instant>,
    read_gate: Option<&dyn ReadGate>,
    replay: &Replay,
    stop: &mut Stop,
    responding: &mut Responding<S>,
) -> Ended {
    let mut request = tonic::Request::new(messages);
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

    // Each attempt's own: whether a response came is the last attempt's to say.
    responding.answered = Answered::default();
    let mut client = inner.client(responding.answered.clone());
    let response = match until_stopped(stop, client.streaming(request, path, BytesCodec)).await {
        None => return Ended::with(GrpcStatus::cancelled(), Pushback::Unsaid),
        Some(Err(status)) => {
            return Ended {
                pushback: Pushback::of(&status.metadata().clone().into_headers()),
                unprocessed: Unprocessed::marked(&status),
                status: GrpcStatus::from(status),
            };
        }
        Some(Ok(response)) => response,
    };

    let (head, mut body, _) = response.into_parts();
    let head = head.into_headers();

    // A head that states a status is the Trailers-Only shape, where that one HEADERS frame is the
    // trailers and not initial metadata. tonic ends such a stream empty and leaves the status in
    // the head, so it is read from there, and nothing goes out as a head: delivering those
    // headers twice would have the reader see a head no such response has.
    if let Some(status) = tonic::Status::from_header_map(&head) {
        return Ended::with(GrpcStatus::from(status), Pushback::of(&head));
    }
    // The reader has a head: whatever follows, this call is not tried again.
    replay.commit();
    responding.head_given = true;
    let head = ResponseHead {
        metadata: Metadata::from_headers(&head),
        origin: HeadOrigin::Wire,
    };
    // Given whatever the stop says: once a call has a head, the caller hears it before the end.
    let status = match responding.sink.head(head).await {
        Err(status) => status,
        Ok(()) => finish(stop, &mut responding.sink, read_gate, &mut body).await,
    };
    Ended::with(status, Pushback::Unsaid)
}

/// What the next read off the response found.
enum Read {
    Message(Bytes),
    End(GrpcStatus),
}

/// The response's messages and trailers, once its head is given.
///
/// Each read is polled once first: what is already there is read at once, and only when nothing
/// is does the sink hear that nothing more is ready, before the wait.
async fn finish<S: ResponseSink>(
    stop: &mut Stop,
    sink: &mut S,
    read_gate: Option<&dyn ReadGate>,
    body: &mut tonic::Streaming<Bytes>,
) -> GrpcStatus {
    loop {
        let read = {
            let mut next = pin!(until_stopped(stop, read_next(read_gate, body)));
            match std::future::poll_fn(|cx| Poll::Ready(next.as_mut().poll(cx))).await {
                Poll::Ready(read) => read,
                Poll::Pending => {
                    sink.flush();
                    next.await
                }
            }
        };
        let message = match read {
            None => return GrpcStatus::cancelled(),
            Some(Read::End(status)) => return status,
            Some(Read::Message(message)) => message,
        };
        match until_stopped(stop, sink.message(message)).await {
            None => return GrpcStatus::cancelled(),
            Some(Err(status)) => return status,
            Some(Ok(())) => {}
        }
    }
}

async fn read_next(read_gate: Option<&dyn ReadGate>, body: &mut tonic::Streaming<Bytes>) -> Read {
    // Before the read, not after the message: a call the gate holds pulls nothing off the stream,
    // so flow control holds its peer and nothing is decoded that the gate refused.
    if let Some(gate) = read_gate {
        gate.admitted().await;
    }
    match body.message().await {
        Err(status) => Read::End(GrpcStatus::from(past_the_limit(status))),
        Ok(Some(message)) => Read::Message(message),
        Ok(None) => Read::End(match body.trailers().await {
            Err(status) => GrpcStatus::from(status),
            Ok(trailers) => {
                GrpcStatus::ok(&trailers.map(MetadataMap::into_headers).unwrap_or_default())
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
        tonic::Status::resource_exhausted(status.message())
    } else {
        status
    }
}

/// Room for a small message and its prefix; a larger one has the buffer grown to fit it. Not 0: tonic
/// divides by it when it decompresses.
const CODEC_BUFFER: usize = 1024;

/// How much a stream's encoder gathers before it hands a batch on: tonic's default, which its
/// settings do not expose.
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

    /// The one copy a message makes on its way out.
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
        let (call, _messages, driving) = create(1, closed_rx);
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
