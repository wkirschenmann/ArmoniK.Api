//! One call: what is sent on it, what comes back, and the task that drives it.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::buf::Chain;
use bytes::Bytes;
use http::header::HeaderMap;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use hyper::body::{Body, Frame};
use tokio::sync::{mpsc, oneshot, watch};

use super::channel::Inner;
use super::error::CallError;
use super::frame::{frame, Deframer};
use super::metadata::{Metadata, GRPC_MESSAGE, GRPC_STATUS};
use super::status::{decode_message, GrpcStatus, GrpcStatusCode};

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
}

impl SendHalf {
    /// Sends one message, once the send window has room for it.
    ///
    /// The buffer is framed rather than copied, so what the caller passed is what goes out.
    pub async fn send_message(&mut self, message: Bytes) -> Result<(), CallError> {
        let framed = frame(message)?;
        self.messages
            .send(framed)
            .await
            .map_err(|_| CallError::Ended)
    }

    /// Half-closes the request. Taking `self` is what makes "no send after the end of sending" a
    /// fact about the type rather than a rule to remember.
    pub async fn end_send(self) -> Result<(), CallError> {
        // Dropping the sender is the half-close; whether the peer is still there to see it is
        // what the answer says.
        let ended = self.messages.is_closed();
        drop(self.messages);
        if ended {
            Err(CallError::Ended)
        } else {
            Ok(())
        }
    }
}

/// The reading side of a call.
#[derive(Debug)]
pub struct RecvHalf {
    head: Head,
    messages: mpsc::Receiver<RecvResult>,
    control: CallControl,
    ended: bool,
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
            self.head = match pending.await {
                Ok(metadata) => Head::Ready(metadata),
                Err(_) => Head::Lost,
            };
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
        if self.ended {
            return Err(CallError::Ended);
        }
        match self.messages.recv().await {
            Some(RecvResult::End(status)) => {
                self.ended = true;
                Ok(RecvResult::End(status))
            }
            Some(message) => Ok(message),
            None => {
                self.ended = true;
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
#[derive(Clone, Debug)]
pub struct CallControl {
    state: Arc<CancelState>,
}

#[derive(Debug)]
struct CancelState {
    cancel: watch::Sender<bool>,
}

impl CallControl {
    /// Cancels the call. Idempotent, and safe to call after it has ended.
    pub fn cancel(&self) {
        self.state.cancel.send_replace(true);
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
    channel_closed: watch::Receiver<bool>,
) -> (GrpcCall, RequestBody, Driving) {
    let (message_tx, message_rx) = mpsc::channel(send_window);
    let (head_tx, head_rx) = oneshot::channel();
    let (recv_tx, recv_rx) = mpsc::channel(1);
    let (cancel_tx, cancel_rx) = watch::channel(false);

    let control = CallControl {
        state: Arc::new(CancelState { cancel: cancel_tx }),
    };
    let call = GrpcCall {
        send: SendHalf {
            messages: message_tx,
        },
        recv: RecvHalf {
            head: Head::Pending(head_rx),
            messages: recv_rx,
            control: control.clone(),
            ended: false,
        },
        control,
    };
    let driving = Driving {
        stop: Stop {
            cancel: cancel_rx,
            channel_closed,
        },
        delivery: Delivery {
            head: Some(head_tx),
            messages: recv_tx,
        },
    };

    (call, RequestBody { messages: message_rx }, driving)
}

/// The driving task's half of a call.
pub(crate) struct Driving {
    stop: Stop,
    delivery: Delivery,
}

/// Runs the call to its terminal, and delivers that terminal whatever happens.
pub(crate) async fn drive(inner: Arc<Inner>, request: Request<RequestBody>, driving: Driving) {
    let Driving {
        mut stop,
        mut delivery,
    } = driving;
    let status = run(&inner, request, &mut stop, &mut delivery).await;
    delivery.end(status).await;
}

/// What ends a call from this side: the caller cancelled it, or the channel closed.
struct Stop {
    cancel: watch::Receiver<bool>,
    channel_closed: watch::Receiver<bool>,
}

impl Stop {
    /// Resolves once the call should stop. A sender that is gone counts as stopped: nothing is
    /// left that could ask for the result.
    async fn stopped(&mut self) {
        let Self {
            cancel,
            channel_closed,
        } = self;
        tokio::select! {
            _ = cancel.wait_for(|stopped| *stopped) => {}
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
) -> GrpcStatus {
    let mut sender = match until_stopped(stop, inner.sender()).await {
        None => return cancelled(),
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

    if head.status != StatusCode::OK {
        return http_status(head.status, &head.headers);
    }
    // A Trailers-Only response carries the whole outcome in the headers and has no body.
    if head.headers.contains_key(GRPC_STATUS) {
        return status_from(&head.headers);
    }
    if !speaks_grpc(&head.headers) {
        return GrpcStatus::new(
            GrpcStatusCode::Internal,
            "the peer answered HTTP 200 without a gRPC content type",
        );
    }

    delivery.head(Metadata::from_headers(&head.headers));

    let mut deframer = Deframer::default();
    loop {
        match deliver_ready(&mut deframer, stop, delivery).await {
            Ok(true) => {}
            Ok(false) => return cancelled(),
            Err(status) => return status,
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

        match deliver_ready(&mut deframer, stop, delivery).await {
            Ok(true) => {}
            Ok(false) => return cancelled(),
            Err(status) => return status,
        }
        if !deframer.is_at_message_boundary() {
            return GrpcStatus::new(
                GrpcStatusCode::Internal,
                "the peer ended the stream in the middle of a message",
            );
        }
        return status_from(&trailers);
    }
}

/// Hands over every message the deframer already holds.
///
/// `Ok(false)` says nobody is reading any more, `Err` that the peer's framing is not something
/// this engine can read.
async fn deliver_ready(
    deframer: &mut Deframer,
    stop: &mut Stop,
    delivery: &Delivery,
) -> Result<bool, GrpcStatus> {
    loop {
        match deframer.next_message() {
            Ok(None) => return Ok(true),
            Err(error) => {
                return Err(GrpcStatus::new(
                    GrpcStatusCode::Internal,
                    error.to_string(),
                ))
            }
            Ok(Some(message)) => match until_stopped(stop, delivery.message(message)).await {
                Some(true) => {}
                _ => return Ok(false),
            },
        }
    }
}

fn cancelled() -> GrpcStatus {
    GrpcStatus::new(GrpcStatusCode::Cancelled, "the call was cancelled")
}

/// The terminal of a response that never became gRPC.
fn http_status(status: StatusCode, headers: &HeaderMap) -> GrpcStatus {
    GrpcStatus {
        code: GrpcStatusCode::from_http_status(status.as_u16()),
        message: format!("the peer answered HTTP {} rather than gRPC", status.as_u16()),
        trailing_metadata: Metadata::from_headers(headers),
    }
}

/// The terminal a set of headers or trailers states.
fn status_from(headers: &HeaderMap) -> GrpcStatus {
    let code = headers
        .get(GRPC_STATUS)
        .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
        .and_then(|text| text.trim().parse::<i32>().ok());

    let Some(code) = code else {
        return GrpcStatus::new(
            GrpcStatusCode::Internal,
            "the peer's grpc-status is missing or is not a number",
        );
    };

    GrpcStatus {
        code: GrpcStatusCode::from_wire(code),
        message: headers
            .get(GRPC_MESSAGE)
            .map(|value| decode_message(value.as_bytes()))
            .unwrap_or_default(),
        trailing_metadata: Metadata::from_headers(headers),
    }
}

/// Whether the content type says the body is gRPC. The subtype after `+` names the message
/// encoding, which is the caller's business rather than this engine's.
fn speaks_grpc(headers: &HeaderMap) -> bool {
    headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            let value = value.trim();
            value == "application/grpc"
                || value.starts_with("application/grpc+")
                || value.starts_with("application/grpc;")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (key, value) in pairs {
            map.append(*key, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn a_status_carries_its_code_its_reason_and_the_rest_of_the_trailers() {
        let status = status_from(&headers(&[
            ("grpc-status", "9"),
            ("grpc-message", "not%20now"),
            ("x-trailer", "kept"),
        ]));

        assert_eq!(status.code, GrpcStatusCode::FailedPrecondition);
        assert_eq!(status.message, "not now");
        assert_eq!(status.trailing_metadata.len(), 1);
    }

    #[test]
    fn trailers_without_a_readable_status_are_an_internal_failure() {
        assert_eq!(
            status_from(&headers(&[("grpc-message", "orphan")])).code,
            GrpcStatusCode::Internal
        );
        assert_eq!(
            status_from(&headers(&[("grpc-status", "not a number")])).code,
            GrpcStatusCode::Internal
        );
    }

    #[test]
    fn a_status_without_a_message_is_still_a_status() {
        let status = status_from(&headers(&[("grpc-status", "0")]));
        assert_eq!(status.code, GrpcStatusCode::Ok);
        assert!(status.message.is_empty());
    }

    #[test]
    fn the_content_type_has_to_say_grpc() {
        assert!(speaks_grpc(&headers(&[(
            "content-type",
            "application/grpc"
        )])));
        assert!(speaks_grpc(&headers(&[(
            "content-type",
            "application/grpc+proto"
        )])));
        assert!(!speaks_grpc(&headers(&[("content-type", "text/html")])));
        // A prefix is not a type: `application/grpcweb` is a different protocol.
        assert!(!speaks_grpc(&headers(&[(
            "content-type",
            "application/grpcweb"
        )])));
        assert!(!speaks_grpc(&HeaderMap::new()));
    }

    #[test]
    fn an_http_failure_maps_to_the_code_grpc_gives_it() {
        assert_eq!(
            http_status(StatusCode::NOT_FOUND, &HeaderMap::new()).code,
            GrpcStatusCode::Unimplemented
        );
        assert_eq!(
            http_status(StatusCode::SERVICE_UNAVAILABLE, &HeaderMap::new()).code,
            GrpcStatusCode::Unavailable
        );
    }
}
