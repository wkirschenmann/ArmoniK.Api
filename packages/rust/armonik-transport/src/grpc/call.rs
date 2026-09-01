//! One call: what is sent on it, what comes back, and the task that drives it.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::buf::Chain;
use bytes::Bytes;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use hyper::body::{Body, Frame};
use tokio::sync::{mpsc, oneshot, watch};

use super::channel::Inner;
use super::error::{CallError, ChannelError};
use super::frame::{frame, Deframer};
use super::metadata::Metadata;
use super::status::{
    cancelled, http_status, speaks_grpc, stated_status, GrpcStatus, GrpcStatusCode,
};

/// What a call is started with.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CallStartOptions {
    /// The method path, as in `/armonik.api.grpc.v1.Sessions/CreateSession`.
    pub method: String,
    /// Request metadata, which becomes headers.
    pub metadata: Metadata,
}

impl CallStartOptions {
    /// A call to `method`, with no metadata.
    pub fn new(method: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            metadata: Metadata::new(),
        }
    }
}

/// A received message. Dropping it releases the buffer it is a view on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedMessage {
    /// The encoded message, without its gRPC frame header.
    pub data: Bytes,
}

/// What came next on a call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecvResult {
    /// A message from the peer.
    Message(OwnedMessage),
    /// The terminal. Nothing follows it.
    End(GrpcStatus),
}

/// A started call, before it is taken apart.
#[derive(Debug)]
pub struct GrpcCall {
    send: SendHalf,
    recv: RecvHalf,
    control: CallControl,
}

impl GrpcCall {
    /// Splits the call into the side that writes, the side that reads, and the one that cancels.
    ///
    /// The halves take `&mut self`, so two concurrent sends and two concurrent reads are not
    /// states this API can be asked for.
    pub fn split(self) -> (SendHalf, RecvHalf, CallControl) {
        (self.send, self.recv, self.control)
    }
}

/// The writing side of a call.
#[derive(Debug)]
pub struct SendHalf {
    messages: mpsc::Sender<Chain<Bytes, Bytes>>,
    over: watch::Receiver<bool>,
}

impl SendHalf {
    /// Sends one message, once the send window has room for it.
    ///
    /// The buffer is framed rather than copied, so what the caller passed is what goes out.
    ///
    /// A call that has reached its terminal takes no more: the abstract model guards sending on
    /// a call with no status yet, and a send after one would go out on a stream the peer has
    /// finished with. A send already waiting on a full window answers to that too, which is why
    /// the wait is against the end of the call as well as against the window.
    pub async fn send_message(&mut self, message: Bytes) -> Result<(), CallError> {
        let framed = frame(message)?;
        let Self { messages, over } = self;

        tokio::select! {
            biased;
            _ = over.wait_for(|over| *over) => Err(CallError::Ended),
            queued = messages.send(framed) => queued.map_err(|_| CallError::Ended),
        }
    }

    /// Half-closes the request. Taking `self` is what makes "no send after the end of sending" a
    /// fact about the type rather than a rule to remember.
    ///
    /// Cannot fail, and awaits nothing: the half-close is the drop, which happens whatever the
    /// call has already done. The result is the shape design.md gives this operation; why a call
    /// ended is its terminal status, which the reading half carries.
    pub async fn end_send(self) -> Result<(), CallError> {
        Ok(())
    }
}

/// The reading side of a call.
#[derive(Debug)]
pub struct RecvHalf {
    head: Head,
    messages: mpsc::Receiver<RecvResult>,
    control: CallControl,
    /// What every further read answers, once there is nothing more to read.
    ended: Option<CallError>,
}

#[derive(Debug)]
enum Head {
    Pending(oneshot::Receiver<Metadata>),
    Ready(Metadata),
    Lost,
}

impl RecvHalf {
    /// The response head, once it has arrived.
    ///
    /// A call that ends without one - a Trailers-Only response, a connection that never opened -
    /// yields empty metadata rather than an error; the reason is the terminal status, which
    /// [`Self::next_message`] carries.
    pub async fn recv_initial_metadata(&mut self) -> Result<Metadata, CallError> {
        if let Head::Pending(pending) = &mut self.head {
            self.head = pending.await.map_or(Head::Lost, Head::Ready);
        }
        match &self.head {
            Head::Ready(metadata) => Ok(metadata.clone()),
            _ => Err(CallError::Aborted),
        }
    }

    /// The next message, or the terminal status.
    ///
    /// Each call is a request for one message, which is where the backpressure comes from: the
    /// engine reads the connection no further than the reader has asked for.
    pub async fn next_message(&mut self) -> Result<RecvResult, CallError> {
        if let Some(ended) = &self.ended {
            return Err(ended.clone());
        }
        match self.messages.recv().await {
            Some(RecvResult::End(status)) => {
                self.ended = Some(CallError::Ended);
                Ok(RecvResult::End(status))
            }
            Some(message) => Ok(message),
            None => {
                self.ended = Some(CallError::Aborted);
                Err(CallError::Aborted)
            }
        }
    }
}

impl Drop for RecvHalf {
    /// A call nobody will read is a call nobody wants; cancelling releases the stream instead of
    /// leaving the peer to fill a window that is never drained.
    fn drop(&mut self) {
        self.control.cancel();
    }
}

/// Cancels a call, from wherever the decision is taken.
///
/// What it sets is "this call is over", which a cancellation and a terminal both make true; that
/// is what stops the writing side once the call has ended.
#[derive(Clone, Debug)]
pub struct CallControl {
    over: Arc<watch::Sender<bool>>,
}

impl CallControl {
    /// Cancels the call. Idempotent, and safe to call after it has ended.
    pub fn cancel(&self) {
        self.over.send_replace(true);
    }
}

/// The request body: the messages the writer has handed over, framed.
pub(crate) struct RequestBody {
    messages: mpsc::Receiver<Chain<Bytes, Bytes>>,
}

impl Body for RequestBody {
    type Data = Chain<Bytes, Bytes>;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        self.messages
            .poll_recv(cx)
            .map(|message| message.map(|buffer| Ok(Frame::data(buffer))))
    }
}

/// Builds the two ends of a call, and everything its driving task needs.
pub(crate) fn create(
    send_window: usize,
    max_recv_message_size: usize,
    channel_closed: watch::Receiver<bool>,
) -> (GrpcCall, RequestBody, Driving) {
    let (message_tx, message_rx) = mpsc::channel(send_window);
    let (head_tx, head_rx) = oneshot::channel();
    let (recv_tx, recv_rx) = mpsc::channel(1);
    let (over_tx, over_rx) = watch::channel(false);

    let control = CallControl {
        over: Arc::new(over_tx),
    };
    let call = GrpcCall {
        send: SendHalf {
            messages: message_tx,
            over: over_rx.clone(),
        },
        recv: RecvHalf {
            head: Head::Pending(head_rx),
            messages: recv_rx,
            control: control.clone(),
            ended: None,
        },
        control: control.clone(),
    };
    let driving = Driving {
        stop: Stop {
            over: over_rx,
            channel_closed,
        },
        delivery: Delivery {
            head: Some(head_tx),
            messages: recv_tx,
        },
        control,
        max_recv_message_size,
    };

    (
        call,
        RequestBody {
            messages: message_rx,
        },
        driving,
    )
}

/// The driving task's half of a call.
pub(crate) struct Driving {
    stop: Stop,
    delivery: Delivery,
    control: CallControl,
    max_recv_message_size: usize,
}

/// Runs the call to its terminal, and delivers that terminal whatever happens.
pub(crate) async fn drive(inner: Arc<Inner>, request: Request<RequestBody>, driving: Driving) {
    let Driving {
        mut stop,
        mut delivery,
        control,
        max_recv_message_size,
    } = driving;

    let status = run(
        &inner,
        request,
        &mut stop,
        &mut delivery,
        max_recv_message_size,
    )
    .await;
    // The call is over the moment its terminal is decided, whichever way it went; the writing
    // side is told before the reading side, so a reader that has the terminal knows the writer
    // is already refusing.
    control.cancel();
    delivery.end(status).await;
}

/// What ends a call from this side: the caller cancelled it, or the channel closed.
struct Stop {
    over: watch::Receiver<bool>,
    channel_closed: watch::Receiver<bool>,
}

impl Stop {
    /// Resolves once the call should stop. A sender that is gone counts as stopped: nothing is
    /// left that could ask for the result.
    async fn stopped(&mut self) {
        // Destructured because `select!` puts both arms in one scope, where two `&mut self`
        // methods do not borrow-check as the disjoint fields they are.
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

/// `work`'s result, unless the call stopped first.
async fn until_stopped<T>(stop: &mut Stop, work: impl Future<Output = T>) -> Option<T> {
    tokio::select! {
        biased;
        _ = stop.stopped() => None,
        value = work => Some(value),
    }
}

/// Where a call's events go, with the head resolved exactly once.
struct Delivery {
    head: Option<oneshot::Sender<Metadata>>,
    messages: mpsc::Sender<RecvResult>,
}

impl Delivery {
    fn head(&mut self, metadata: Metadata) {
        if let Some(head) = self.head.take() {
            let _ = head.send(metadata);
        }
    }

    /// Hands over one message, or reports that nobody is reading any more.
    async fn message(&self, data: Bytes) -> bool {
        self.messages
            .send(RecvResult::Message(OwnedMessage { data }))
            .await
            .is_ok()
    }

    async fn end(mut self, status: GrpcStatus) {
        // A call that never saw a response head still answers the question, with nothing in it.
        self.head(Metadata::new());
        let _ = self.messages.send(RecvResult::End(status)).await;
    }
}

async fn run(
    inner: &Inner,
    request: Request<RequestBody>,
    stop: &mut Stop,
    delivery: &mut Delivery,
    max_recv_message_size: usize,
) -> GrpcStatus {
    let mut sender = match until_stopped(stop, inner.sender()).await {
        None | Some(Err(ChannelError::Closed)) => return cancelled(),
        Some(Err(error)) => return GrpcStatus::new(GrpcStatusCode::Unavailable, error.to_string()),
        Some(Ok(sender)) => sender,
    };

    let response = match until_stopped(stop, sender.send_request(request)).await {
        None => return cancelled(),
        Some(Err(error)) => {
            return GrpcStatus::new(
                GrpcStatusCode::Unavailable,
                format!("the request did not reach the peer: {error}"),
            )
        }
        Some(Ok(response)) => response,
    };

    let (head, mut body) = response.into_parts();

    // A peer that states a status in the response head has said how the call ended, and that
    // answer stands whatever the HTTP status is: a Trailers-Only response is this case, and so
    // is a gRPC failure served behind an HTTP error.
    if let Some(status) = stated_status(&head.headers) {
        return status;
    }
    if head.status != StatusCode::OK {
        return http_status(head.status, &head.headers);
    }
    if !speaks_grpc(&head.headers) {
        return GrpcStatus::new(
            GrpcStatusCode::Internal,
            "the peer answered HTTP 200 without a gRPC content type",
        );
    }

    delivery.head(Metadata::from_headers(&head.headers));

    let mut deframer = Deframer::new(max_recv_message_size);
    loop {
        if let Err(status) = deliver_ready(&mut deframer, stop, delivery).await {
            return status;
        }

        let frame = match until_stopped(stop, body.frame()).await {
            None => return cancelled(),
            Some(None) => {
                return GrpcStatus::new(
                    GrpcStatusCode::Internal,
                    "the peer ended the stream without a grpc-status",
                )
            }
            Some(Some(Err(error))) => {
                return GrpcStatus::new(
                    GrpcStatusCode::Unavailable,
                    format!("the response stream broke: {error}"),
                )
            }
            Some(Some(Ok(frame))) => frame,
        };

        let trailers = match frame.into_data() {
            Ok(data) => {
                deframer.push(data);
                continue;
            }
            Err(frame) => match frame.into_trailers() {
                Ok(trailers) => trailers,
                // Neither data nor trailers: a frame kind this engine has no use for.
                Err(_) => continue,
            },
        };

        if !deframer.is_at_message_boundary() {
            return GrpcStatus::new(
                GrpcStatusCode::Internal,
                "the peer ended the stream in the middle of a message",
            );
        }
        return stated_status(&trailers).unwrap_or_else(|| {
            GrpcStatus::new(
                GrpcStatusCode::Internal,
                "the peer's trailers carry no grpc-status",
            )
        });
    }
}

/// Hands over every message the deframer already holds.
///
/// The error is the terminal to end the call with: either the peer's framing is unreadable, or
/// nobody is reading any more.
async fn deliver_ready(
    deframer: &mut Deframer,
    stop: &mut Stop,
    delivery: &Delivery,
) -> Result<(), GrpcStatus> {
    loop {
        match deframer.next_message() {
            Ok(None) => return Ok(()),
            Err(error) => return Err(GrpcStatus::new(error.code(), error.to_string())),
            Ok(Some(message)) => match until_stopped(stop, delivery.message(message)).await {
                Some(true) => {}
                _ => return Err(cancelled()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn a_call_that_is_over_refuses_a_send_with_its_request_body_still_open() {
        // The body stays bound, so the mpsc is not closed and only the flag can refuse. End to
        // end hyper also tears the stream down, which is why this has to be asked here: there,
        // either mechanism would answer.
        let (call, _body, _driving) = create(4, usize::MAX, watch::channel(false).1);
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

    #[tokio::test]
    async fn a_send_waiting_on_a_full_window_is_refused_when_the_call_ends() {
        let (call, _body, _driving) = create(1, usize::MAX, watch::channel(false).1);
        let (mut send, _recv, control) = call.split();

        send.send_message(Bytes::from_static(b"first"))
            .await
            .expect("the window has room");

        // Nothing polls the body, so this one waits rather than being queued.
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

    #[tokio::test]
    async fn the_send_window_holds_the_next_message_until_the_last_is_taken() {
        let (call, mut body, _driving) = create(1, usize::MAX, watch::channel(false).1);
        // The reading half stays bound: dropping it cancels the call, which is the very thing
        // that would let the second send through for the wrong reason.
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

        std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
            .await
            .expect("the first message is there to be taken")
            .expect("a data frame");

        send.send_message(Bytes::from_static(b"second"))
            .await
            .expect("the window has room again");
    }
}
