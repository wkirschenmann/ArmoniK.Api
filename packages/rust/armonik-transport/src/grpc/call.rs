//! One call: what is sent on it, what comes back, and the task that drives it.

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::buf::Chain;
use bytes::Bytes;
use hyper::body::{Body, Frame};
use tokio::sync::{mpsc, oneshot, watch};

use super::driver::Driving;
use super::error::CallError;
use super::frame::frame;
use super::metadata::Metadata;
use super::status::GrpcStatus;

#[derive(Clone, Debug)]
#[non_exhaustive]
/// What a call is started with.
pub struct CallStartOptions {
    /// The method path, as in `/armonik.api.grpc.v1.Sessions/CreateSession`.
    pub method: String,
    /// Request metadata, which becomes headers.
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
/// A received message. Dropping it releases the buffer it is a view on.
pub struct OwnedMessage {
    /// The encoded message, without its gRPC frame header.
    pub data: Bytes,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// What came next on a call.
pub enum RecvResult {
    /// A message from the peer.
    Message(OwnedMessage),
    /// The terminal. Nothing follows it.
    End(GrpcStatus),
}

#[derive(Debug)]
pub struct GrpcCall {
    send: SendHalf,
    recv: RecvHalf,
    control: CallControl,
}

impl GrpcCall {
    /// Splits the call into the side that writes, the side that reads, and the one that
    /// cancels. The halves take `&mut self`, so two concurrent sends and two concurrent
    /// reads are not states this API can be asked for.
    pub fn split(self) -> (SendHalf, RecvHalf, CallControl) {
        (self.send, self.recv, self.control)
    }
}

#[derive(Debug)]
/// The writing side of a call.
pub struct SendHalf {
    messages: mpsc::Sender<Chain<Bytes, Bytes>>,
    over: watch::Receiver<bool>,
}

impl SendHalf {
    /// Sends one message, once the send window has room for it. The buffer is framed rather
    /// than copied, so what the caller passed is what goes out.
    ///
    /// A call that has reached its terminal takes no more: the abstract model guards
    /// sending on a call with no status yet, and a send after one would go out on a stream
    /// the peer has finished with. A send already waiting on a full window answers to that
    /// too, which is why the wait is against the end of the call as well as the window.
    pub async fn send_message(&mut self, message: Bytes) -> Result<(), CallError> {
        let framed = frame(message)?;
        let Self { messages, over } = self;

        // The guard first, as the model has it, and then the window: with room, the send is two
        // reads and a push. The `select!` below registers a waiter on the `over` watch and
        // deregisters it again - two locks on one of its notify shards - which is the price of
        // waiting, and there is nothing to wait for until the window is full.
        if *over.borrow() {
            return Err(CallError::Ended);
        }

        let framed = match messages.try_send(framed) {
            Ok(()) => return Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => return Err(CallError::Ended),
            Err(mpsc::error::TrySendError::Full(framed)) => framed,
        };

        tokio::select! {
            biased;
            _ = over.wait_for(|over| *over) => Err(CallError::Ended),
            queued = messages.send(framed) => queued.map_err(|_| CallError::Ended),
        }
    }

    /// Half-closes the request. Taking `self` is what makes "no send after the end of
    /// sending" a fact about the type rather than a rule to remember.
    ///
    /// Cannot fail, and awaits nothing: the half-close is the drop, which happens whatever
    /// the call has already done. The result is the shape design.md gives this operation;
    /// why a call ended is its terminal status, which the reading half carries.
    pub async fn end_send(self) -> Result<(), CallError> {
        Ok(())
    }
}

#[derive(Debug)]
/// The reading side of a call.
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
    /// The response head, once it has arrived.
    ///
    /// A call that ends without one - a Trailers-Only response, a connection that never
    /// opened - yields empty metadata rather than an error; the reason is the terminal
    /// status, which [`Self::next_message`] carries.
    pub async fn recv_initial_metadata(&mut self) -> Result<&Metadata, CallError> {
        if let Head::Pending(pending) = &mut self.head {
            self.head = pending.await.map_or(Head::Lost, Head::Ready);
        }
        match &self.head {
            Head::Ready(metadata) => Ok(metadata),
            _ => Err(CallError::Aborted),
        }
    }

    /// The next message, or the terminal status.
    ///
    /// Each call is a request for one message, which is where the backpressure comes from:
    /// the engine reads the connection no further than the reader has asked for.
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
    /// A call nobody will read is a call nobody wants; cancelling releases the stream
    /// instead of leaving the peer to fill a window that is never drained.
    fn drop(&mut self) {
        self.control.cancel();
    }
}

#[derive(Clone, Debug)]
/// Cancels a call, from wherever the decision is taken.
///
/// What it sets is "this call is over", which a cancellation and a terminal both make
/// true; that is what stops the writing side once the call has ended.
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
    let driving = Driving::new(over_rx, channel_closed, head_tx, recv_tx, control);

    (
        call,
        RequestBody {
            messages: message_rx,
        },
        driving,
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn a_call_that_is_over_refuses_a_send_with_its_request_body_still_open() {
        // The body stays bound, so the channel is not closed and only the flag can refuse.
        // End to end hyper also tears the stream down, which is why this is asked here.
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
        let (call, mut body, _driving) = create(1, watch::channel(false).1);
        // The reading half stays bound: dropping it cancels the call, which is the very
        // thing that would let the second send through for the wrong reason.
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
