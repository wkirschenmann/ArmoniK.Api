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

        if *over.borrow() {
            return Err(CallError::Ended);
        }

        let framed = match messages.try_send(framed) {
            Ok(()) => return Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => return Err(CallError::Ended),
            Err(mpsc::error::TrySendError::Full(framed)) => framed,
        };

        // Biased, so a call that ended while this send waited for room answers `Ended` rather than
        // queueing into a channel the driver has stopped reading.
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
    messages: mpsc::Receiver<OwnedMessage>,
    terminal: oneshot::Receiver<GrpcStatus>,
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
    pub async fn recv_initial_metadata(&mut self) -> Result<&Metadata, CallError> {
        if let Head::Pending(pending) = &mut self.head {
            self.head = pending.await.map_or(Head::Lost, Head::Ready);
        }
        match &self.head {
            Head::Ready(metadata) => Ok(metadata),
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
    // One, because the reader is what paces the peer: anything deeper reads ahead of a consumer
    // that has not asked, and the message sits in memory this side has not accounted for.
    let (recv_tx, recv_rx) = mpsc::channel(1);
    let (terminal_tx, terminal_rx) = oneshot::channel();
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
            terminal: terminal_rx,
            control: control.clone(),
            ended: None,
        },
        control: control.clone(),
    };
    let driving = Driving::new(
        over_rx,
        channel_closed,
        head_tx,
        recv_tx,
        terminal_tx,
        control,
    );

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

    #[tokio::test(start_paused = true)]
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

    #[tokio::test(start_paused = true)]
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
