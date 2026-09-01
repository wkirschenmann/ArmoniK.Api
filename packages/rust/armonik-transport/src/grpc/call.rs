
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

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CallStartOptions {
    pub method: String,
    pub metadata: Metadata,
}

impl CallStartOptions {
    pub fn new(method: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            metadata: Metadata::new(),
        }
    }
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
    messages: mpsc::Sender<Chain<Bytes, Bytes>>,
    over: watch::Receiver<bool>,
}

impl SendHalf {
    pub async fn send_message(&mut self, message: Bytes) -> Result<(), CallError> {
        let framed = frame(message)?;
        let Self { messages, over } = self;

        tokio::select! {
            biased;
            _ = over.wait_for(|over| *over) => Err(CallError::Ended),
            queued = messages.send(framed) => queued.map_err(|_| CallError::Ended),
        }
    }

    pub async fn end_send(self) -> Result<(), CallError> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct RecvHalf {
    head: Head,
    messages: mpsc::Receiver<RecvResult>,
    control: CallControl,
    ended: Option<CallError>,
}

#[derive(Debug)]
enum Head {
    Pending(oneshot::Receiver<Metadata>),
    Ready(Metadata),
    Lost,
}

impl RecvHalf {
    pub async fn recv_initial_metadata(&mut self) -> Result<Metadata, CallError> {
        if let Head::Pending(pending) = &mut self.head {
            self.head = pending.await.map_or(Head::Lost, Head::Ready);
        }
        match &self.head {
            Head::Ready(metadata) => Ok(metadata.clone()),
            _ => Err(CallError::Aborted),
        }
    }

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
    fn drop(&mut self) {
        self.control.cancel();
    }
}

#[derive(Clone, Debug)]
pub struct CallControl {
    over: Arc<watch::Sender<bool>>,
}

impl CallControl {
    pub fn cancel(&self) {
        self.over.send_replace(true);
    }
}

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

pub(crate) fn create(
    send_window: usize,
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
    };

    (
        call,
        RequestBody {
            messages: message_rx,
        },
        driving,
    )
}

pub(crate) struct Driving {
    stop: Stop,
    delivery: Delivery,
    control: CallControl,
}

pub(crate) async fn drive(inner: Arc<Inner>, request: Request<RequestBody>, driving: Driving) {
    let Driving {
        mut stop,
        mut delivery,
        control,
    } = driving;

    let status = run(&inner, request, &mut stop, &mut delivery).await;
    control.cancel();
    delivery.end(status).await;
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

async fn until_stopped<T>(stop: &mut Stop, work: impl Future<Output = T>) -> Option<T> {
    tokio::select! {
        biased;
        _ = stop.stopped() => None,
        value = work => Some(value),
    }
}

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

    async fn message(&self, data: Bytes) -> bool {
        self.messages
            .send(RecvResult::Message(OwnedMessage { data }))
            .await
            .is_ok()
    }

    async fn end(mut self, status: GrpcStatus) {
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

    let mut deframer = Deframer::new(inner.max_recv_message_size());
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
        let (call, _body, _driving) = create(4, watch::channel(false).1);
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
        let (call, _body, _driving) = create(1, watch::channel(false).1);
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

    #[tokio::test]
    async fn the_send_window_holds_the_next_message_until_the_last_is_taken() {
        let (call, mut body, _driving) = create(1, watch::channel(false).1);
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
