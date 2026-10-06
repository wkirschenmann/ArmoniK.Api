use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot, watch};
use tonic::codegen::tokio_stream::Stream;

use super::driver::{Delivery, Driving};
use super::error::CallError;
use super::metadata::Metadata;
use super::request::{self, FramedMessage, OneRequest, RequestSlot};
use super::status::{GrpcStatus, GrpcStatusCode};

/// Where a call's response head came from. The head's metadata is empty unless it is `Wire`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeadOrigin {
    /// The peer's response headers were delivered.
    Wire,
    /// A response arrived and no head was delivered from it: the Trailers-Only shape, or an
    /// answer this engine refuses before its body, such as one that is not gRPC. The status is
    /// the call's: the peer's, unless the call was stopped here first.
    TrailersOnly,
    /// No response reached the call: it failed or was cancelled before the peer answered.
    NoResponse,
}

/// A call's response head: its metadata, and where it came from.
#[derive(Clone, Debug)]
pub struct ResponseHead {
    pub metadata: Metadata,
    pub origin: HeadOrigin,
}

/// Marked by the call's service once the peer's HTTP response is in, which is what tells a call no
/// response reached from one whose response delivered no head.
#[derive(Clone, Debug, Default)]
pub(crate) struct Answered(Arc<AtomicBool>);

impl Answered {
    pub(crate) fn mark(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(crate) fn marked(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// When a call stops waiting for its answer, and ends `DEADLINE_EXCEEDED`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Deadline {
    /// This instant.
    Absolute(Instant),
    /// This long after the call is started.
    Timeout(Duration),
}

/// When a call may read its next message off the network: the caller's rule, the engine's wait.
///
/// The engine waits on it before each message it reads, so a call it holds reads nothing and
/// HTTP/2 flow control stops its peer, while the call's deadline, its cancellation and its
/// channel closing still end it as they end any other wait. A status the peer sends after its
/// messages is read in the same place, so it too waits for the gate - for its turn alone on a
/// call that declared one response, where nothing but the status can follow the message.
pub trait ReadGate: std::fmt::Debug + Send + Sync {
    /// Completes once the call may read its next message. Asked once per read, from the first
    /// one after the response head, which is not gated; the future is dropped unfinished when the
    /// call ends first.
    fn admitted(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// Completes once a call that declared one response may read what follows its message, which
    /// can only be its status and so takes nothing the gate's admission accounts for. Asked
    /// instead of `admitted` for that one read; a gate with no turn of its own admits it.
    fn turn(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.admitted()
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CallStartOptions {
    pub method: String,
    pub metadata: Metadata,
    /// None takes the channel's default deadline, which may be none.
    pub deadline: Option<Deadline>,
    /// None reads as far ahead as the reader's queue lets it: a message off the stream while the
    /// one before waits there to be taken.
    pub read_gate: Option<Arc<dyn ReadGate>>,
    /// The response is at most one message: a byte of a second one ends the call `INTERNAL`
    /// before anything decodes it, and the read after the message asks the gate for its turn
    /// alone.
    pub one_response: bool,
}

impl CallStartOptions {
    pub fn new(method: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            metadata: Metadata::new(),
            deadline: None,
            read_gate: None,
            one_response: false,
        }
    }
}

/// Where a call's response goes when its caller takes it as the driver reads it, rather than
/// through a [`RecvHalf`].
///
/// The driver calls it from the task that polls it, in order: the head, the messages, then the
/// end. Between the head and the end it calls [`ResponseSink::flush`] whenever it has nothing more
/// ready to read, so a sink may hold what it was given and hand it on together.
pub trait ResponseSink: Send + 'static {
    /// The peer's response head. An error ends the call with that status. The driver gives it
    /// whatever the call's end says, so that a call with a head hands it on before its end; a sink
    /// answers at once rather than wait.
    fn head(&mut self, head: ResponseHead) -> impl Future<Output = Result<(), GrpcStatus>> + Send;

    /// A message read off the stream. An error ends the call with that status.
    fn message(&mut self, data: Bytes) -> impl Future<Output = Result<(), GrpcStatus>> + Send;

    /// Nothing more is ready to be read.
    fn flush(&mut self);

    /// Whether `flush` is what hands on what the calls before it gave, so that the driver gathers
    /// more before it. A sink that hands each part on as it is given says no.
    const GATHERS: bool = true;

    /// The call's end: its status, and the head it was never given when `head` was not called,
    /// which says whether a response came.
    fn end(self, status: GrpcStatus, head: Option<ResponseHead>)
        -> impl Future<Output = ()> + Send;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedMessage {
    pub data: Bytes,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecvResult {
    Message(OwnedMessage),
    End(GrpcStatus),
}

#[derive(Debug)]
pub struct GrpcCall {
    send: SendHalf,
    recv: RecvHalf,
    control: CallControl,
}

impl GrpcCall {
    pub fn split(self) -> (SendHalf, RecvHalf, CallControl) {
        (self.send, self.recv, self.control)
    }
}

#[derive(Debug)]
pub struct SendHalf {
    messages: mpsc::Sender<FramedMessage>,
    over: watch::Receiver<bool>,
    max_message_size: Option<usize>,
    control: CallControl,
}

impl SendHalf {
    /// Queues `message`, framed by a copy into a buffer of its own. [`SendHalf::send_framed`]
    /// sends a message framed in place, with none.
    pub async fn send_message(&mut self, message: Bytes) -> Result<(), CallError> {
        self.admit(message.len())?;
        let framed = FramedMessage::copy_of(&message).expect("a length a four-byte prefix carries");
        self.queue(framed).await
    }

    /// Queues `message` as it is framed. A call that may be retried keeps it for a replay,
    /// charging its length to the replay's budget: a buffer larger than its message stays alive
    /// whole while the budget counts the message.
    pub async fn send_framed(&mut self, message: FramedMessage) -> Result<(), CallError> {
        self.admit(message.len())?;
        self.queue(message).await
    }

    /// Refuses a message of `len` bytes before any of it is queued: the call ends with the
    /// refusal, as the gRPC status table has a message past the configured limit end.
    fn admit(&self, len: usize) -> Result<(), CallError> {
        // Here and not later, which would fail the whole call far from the send that caused it.
        if u32::try_from(len).is_err() {
            return Err(CallError::MessageTooLong { len });
        }
        if *self.over.borrow() {
            return Err(CallError::Ended);
        }
        if let Some(max) = self.max_message_size.filter(|max| len > *max) {
            self.control.refuse(GrpcStatus::new(
                GrpcStatusCode::ResourceExhausted,
                format!("a message of {len} bytes is past the {max} the channel sends"),
            ));
            return Err(CallError::MessageTooLarge { len, max });
        }
        Ok(())
    }

    async fn queue(&mut self, message: FramedMessage) -> Result<(), CallError> {
        let Self { messages, over, .. } = self;
        let message = match messages.try_send(message) {
            Ok(()) => return Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => return Err(CallError::Ended),
            Err(mpsc::error::TrySendError::Full(message)) => message,
        };

        // Biased, so a call that ended while this send waited for room answers `Ended` rather than
        // queueing into a channel the driver has stopped reading.
        tokio::select! {
            biased;
            _ = over.wait_for(|over| *over) => Err(CallError::Ended),
            queued = messages.send(message) => queued.map_err(|_| CallError::Ended),
        }
    }

    pub async fn end_send(self) -> Result<(), CallError> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct RecvHalf {
    head: Head,
    messages: mpsc::Receiver<OwnedMessage>,
    terminal: oneshot::Receiver<GrpcStatus>,
    control: CallControl,
    ended: Option<CallError>,
}

#[derive(Debug)]
enum Head {
    Pending(oneshot::Receiver<ResponseHead>),
    Ready(ResponseHead),
    Lost,
}

impl RecvHalf {
    pub async fn recv_head(&mut self) -> Result<&ResponseHead, CallError> {
        if let Head::Pending(pending) = &mut self.head {
            self.head = pending.await.map_or(Head::Lost, Head::Ready);
        }
        match &self.head {
            Head::Ready(head) => Ok(head),
            _ => Err(CallError::Aborted),
        }
    }

    pub async fn next_message(&mut self) -> Result<RecvResult, CallError> {
        if let Some(ended) = &self.ended {
            return Err(ended.clone());
        }
        // The queue first, to its end, and only then the terminal: what the driver put in the
        // queue before it published the status is still the call's, and a status read early would
        // drop it.
        if let Some(message) = self.messages.recv().await {
            return Ok(RecvResult::Message(message));
        }
        match (&mut self.terminal).await {
            Ok(status) => {
                self.ended = Some(CallError::Ended);
                Ok(RecvResult::End(status))
            }
            // The driver task went away without publishing one, which is not something it does on
            // any path of its own: it was dropped with the runtime.
            Err(_) => {
                self.ended = Some(CallError::Aborted);
                Err(CallError::Aborted)
            }
        }
    }
}

impl Drop for RecvHalf {
    fn drop(&mut self) {
        self.control.cancel();
    }
}

#[derive(Clone, Debug)]
pub struct CallControl {
    over: Arc<watch::Sender<bool>>,
    /// The same news, on a channel the request's messages can poll.
    ///
    /// A `watch` is read, not awaited, so messages parked on their queue would never learn the
    /// call ended - and hyper keeps a reference to the HTTP/2 stream for as long as the body they
    /// feed is unfinished. This is what makes the stream go back.
    body_over: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    /// Why the call stopped when it is this side that refused what it was given, which is the
    /// status it ends with rather than `CANCELLED`.
    refused: Arc<OnceLock<GrpcStatus>>,
}

impl CallControl {
    /// Ends the call with `status`; the first refusal is the one that stands.
    pub(crate) fn refuse(&self, status: GrpcStatus) {
        let _ = self.refused.set(status);
        self.cancel();
    }

    pub(crate) fn refusal(&self) -> Option<GrpcStatus> {
        self.refused.get().cloned()
    }

    pub fn cancel(&self) {
        self.over.send_replace(true);
        if let Some(told) = self
            .body_over
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = told.send(());
        }
    }
}

/// The messages a call sends, framed, as the stream the request body is made of.
pub(crate) struct RequestMessages {
    messages: mpsc::Receiver<FramedMessage>,
    over: oneshot::Receiver<()>,
    ended: bool,
}

impl Stream for RequestMessages {
    type Item = FramedMessage;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }

        // The call ending ends the messages, not only the send half being dropped. hyper pipes an
        // unfinished body into the h2 stream and holds a stream reference while it does, so a
        // call cancelled after the response head left the stream open with no RST_STREAM: the
        // peer went on producing into a reader that discards every frame, and the stream kept its
        // place against MAX_CONCURRENT_STREAMS until the send half happened to drop. A caller
        // that holds one for the life of its client never drops it.
        //
        // Either answer ends it: the control fired, or every control is gone, which is the same
        // news later.
        if Pin::new(&mut this.over).poll(cx).is_ready() {
            this.ended = true;
            return Poll::Ready(None);
        }

        this.messages.poll_recv(cx)
    }
}

/// A call's send half, its control, the messages it sends, and what its driver needs but the sink
/// its response goes to.
pub(crate) fn create_with(
    send_window: usize,
    max_message_size: Option<usize>,
    channel_closed: watch::Receiver<bool>,
) -> (SendHalf, CallControl, RequestMessages, Driving<()>) {
    let (message_tx, message_rx) = mpsc::channel(send_window);
    let (over_tx, over_rx) = watch::channel(false);
    let (body_over_tx, body_over_rx) = oneshot::channel();

    let control = CallControl {
        over: Arc::new(over_tx),
        body_over: Arc::new(Mutex::new(Some(body_over_tx))),
        refused: Arc::default(),
    };
    let send = SendHalf {
        messages: message_tx,
        over: over_rx.clone(),
        max_message_size,
        control: control.clone(),
    };
    let driving = Driving::new(over_rx, channel_closed, control.clone());

    (
        send,
        control,
        RequestMessages {
            messages: message_rx,
            over: body_over_rx,
            ended: false,
        },
        driving,
    )
}

/// A call that sends one request: where the request goes, the call's control, the slot its driver
/// takes the request from, and what its driver needs but the sink.
pub(crate) fn create_one(
    channel_closed: watch::Receiver<bool>,
) -> (OneRequest, CallControl, RequestSlot, Driving<()>) {
    let (request, slot) = request::one_request();
    let (over_tx, over_rx) = watch::channel(false);
    let control = CallControl {
        over: Arc::new(over_tx),
        body_over: Arc::new(Mutex::new(None)),
        refused: Arc::default(),
    };
    let driving = Driving::new(over_rx, channel_closed, control.clone());
    (request, control, slot, driving)
}

pub(crate) fn create(
    send_window: usize,
    max_message_size: Option<usize>,
    channel_closed: watch::Receiver<bool>,
) -> (GrpcCall, RequestMessages, Driving<Delivery>) {
    let (head_tx, head_rx) = oneshot::channel();
    // One, because the reader is what paces the peer: anything deeper reads ahead of a consumer
    // that has not asked, and the message sits in memory this side has not accounted for.
    let (recv_tx, recv_rx) = mpsc::channel(1);
    let (terminal_tx, terminal_rx) = oneshot::channel();

    let (send, control, messages, driving) =
        create_with(send_window, max_message_size, channel_closed);
    let driving = driving.with_sink(Delivery::new(head_tx, recv_tx, terminal_tx));
    let call = GrpcCall {
        send,
        recv: RecvHalf {
            head: Head::Pending(head_rx),
            messages: recv_rx,
            terminal: terminal_rx,
            control: control.clone(),
            ended: None,
        },
        control,
    };

    (call, messages, driving)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// The queue has room and the send is still refused: it is the call's state that decides,
    /// not whether the transport happens to be able to take the bytes.
    #[tokio::test]
    async fn a_call_that_is_over_refuses_a_send_though_its_queue_has_room() {
        let (call, _messages, _driving) = create(4, None, watch::channel(false).1);
        let (mut send, _recv, control) = call.split();

        send.send_message(Bytes::from_static(b"first"))
            .await
            .expect("the call is open");

        control.cancel();

        assert_eq!(
            send.send_message(Bytes::from_static(b"second")).await,
            Err(CallError::Ended)
        );
    }

    /// Messages parked on their queue are woken by the call ending, with the send half still held.
    ///
    /// Held is the case: hyper keeps a reference to the HTTP/2 stream while the body is
    /// unfinished, so messages that end only when their sender drops leave the stream open for as
    /// long as the caller keeps the send half - and a caller that keeps it for the life of its
    /// client keeps the stream for the life of its client.
    #[tokio::test]
    async fn the_request_messages_end_when_the_call_does_and_not_when_their_sender_drops() {
        let (call, mut messages, _driving) = create(1, None, watch::channel(false).1);
        let (_send, _recv, control) = call.split();

        let parked = tokio::spawn(async move {
            std::future::poll_fn(|cx| Pin::new(&mut messages).poll_next(cx)).await
        });
        tokio::task::yield_now().await;

        control.cancel();

        let ended = tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("the call ending wakes the body")
            .expect("the task did not panic");
        assert!(ended.is_none(), "and ends it");
    }

    #[tokio::test(start_paused = true)]
    async fn a_send_waiting_on_a_full_window_is_refused_when_the_call_ends() {
        let (call, _messages, _driving) = create(1, None, watch::channel(false).1);
        let (mut send, _recv, control) = call.split();

        send.send_message(Bytes::from_static(b"first"))
            .await
            .expect("the window has room");

        let waiting =
            tokio::spawn(async move { send.send_message(Bytes::from_static(b"second")).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !waiting.is_finished(),
            "the second send is on a full window"
        );

        control.cancel();

        let resolved = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("ending the call releases a send waiting on the window")
            .expect("the send resolved");
        assert_eq!(resolved, Err(CallError::Ended));
    }

    #[tokio::test(start_paused = true)]
    async fn the_send_window_holds_the_next_message_until_the_last_is_taken() {
        let (call, mut messages, _driving) = create(1, None, watch::channel(false).1);
        let (mut send, _recv, _control) = call.split();

        send.send_message(Bytes::from_static(b"first"))
            .await
            .expect("room for the first");

        let held = tokio::time::timeout(
            Duration::from_millis(50),
            send.send_message(Bytes::from_static(b"second")),
        )
        .await;
        assert!(held.is_err(), "a window of one holds the second message");

        std::future::poll_fn(|cx| Pin::new(&mut messages).poll_next(cx))
            .await
            .expect("the first message is there to be taken");

        send.send_message(Bytes::from_static(b"second"))
            .await
            .expect("the window has room again");
    }
}
