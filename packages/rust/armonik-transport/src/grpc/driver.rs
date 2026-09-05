use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use http::Request;
use http_body_util::BodyExt;
use tokio::sync::{mpsc, oneshot, watch};

use super::call::{CallControl, OwnedMessage, RequestBody};
use super::channel::Inner;
use super::error::ChannelError;
use super::frame::Deframer;
use super::metadata::Metadata;
use super::status::{of_response_head, stated_status, GrpcStatus};

pub(crate) struct Driving {
    stop: Stop,
    delivery: Delivery,
    control: CallControl,
}

impl Driving {
    pub(crate) fn new(
        over: watch::Receiver<bool>,
        channel_closed: watch::Receiver<bool>,
        head: oneshot::Sender<Metadata>,
        messages: mpsc::Sender<OwnedMessage>,
        terminal: oneshot::Sender<GrpcStatus>,
        control: CallControl,
    ) -> Self {
        Self {
            stop: Stop {
                over,
                channel_closed,
            },
            delivery: Delivery {
                head: Some(head),
                messages,
                terminal: Some(terminal),
            },
            control,
        }
    }
}

pub(crate) async fn drive(inner: Arc<Inner>, request: Request<RequestBody>, driving: Driving) {
    let Driving {
        mut stop,
        mut delivery,
        control,
    } = driving;

    // Held for the whole call, not released after the head. `Inner` owns the `closed` sender,
    // and a watch receiver whose senders are all gone answers like one that was told to close - so
    // a driver that let go of it would read a dropped channel handle as a cancellation.
    let status = run(&inner, request, &mut stop, &mut delivery).await;

    // Before the terminal, not after: a send admitted between the two would be queued for a driver
    // that has stopped, and the caller would be told it was sent.
    control.cancel();
    delivery.end(status);
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

/// The three things a call hands its reader, each on its own channel.
///
/// The terminal has one of its own because it is the only one that must arrive. Sharing the
/// message queue meant waiting for room in it, and the wait had to be bounded by something: a
/// channel closing while one message sat unread took the terminal with it, and a call the peer
/// answered OK reached its reader as `Aborted` - through the FFI, a host reading `Cancelled` for
/// a call it had the response to, and retrying what it must not repeat.
struct Delivery {
    head: Option<oneshot::Sender<Metadata>>,
    messages: mpsc::Sender<OwnedMessage>,
    terminal: Option<oneshot::Sender<GrpcStatus>>,
}

impl Delivery {
    fn head(&mut self, metadata: Metadata) {
        if let Some(head) = self.head.take() {
            let _ = head.send(metadata);
        }
    }

    async fn message(&self, data: Bytes) -> bool {
        self.messages.send(OwnedMessage { data }).await.is_ok()
    }

    /// Publishes the status and lets the message queue end.
    ///
    /// Nothing to wait for: the reader takes what is queued, finds the sender gone, and reads the
    /// terminal here. `self` by value, so the queue closes when this returns even on the paths
    /// that never got a status out.
    fn end(mut self, status: GrpcStatus) {
        self.head(Metadata::new());
        if let Some(terminal) = self.terminal.take() {
            let _ = terminal.send(status);
        }
    }
}

async fn run(
    inner: &Arc<Inner>,
    request: Request<RequestBody>,
    stop: &mut Stop,
    delivery: &mut Delivery,
) -> GrpcStatus {
    let mut sender = match until_stopped(stop, inner.sender()).await {
        None | Some(Err(ChannelError::Closed)) => return GrpcStatus::cancelled(),
        Some(Err(error)) => return GrpcStatus::unreachable(error),
        Some(Ok(sender)) => sender,
    };

    let response = match until_stopped(stop, sender.send_request(request)).await {
        None => return GrpcStatus::cancelled(),
        Some(Err(error)) => return GrpcStatus::request_lost(error),
        Some(Ok(response)) => response,
    };

    let (head, mut body) = response.into_parts();
    let metadata = match of_response_head(head.status, &head.headers) {
        Err(status) => return status,
        Ok(metadata) => metadata,
    };

    delivery.head(metadata);

    let mut deframer = Deframer::new(inner.max_recv_message_size());
    loop {
        if let Err(status) = deliver_ready(&mut deframer, stop, delivery).await {
            return status;
        }

        let frame = match until_stopped(stop, body.frame()).await {
            None => return GrpcStatus::cancelled(),
            Some(None) => return GrpcStatus::no_status(),
            Some(Some(Err(error))) => return GrpcStatus::stream_broke(error),
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
            return GrpcStatus::ended_mid_message();
        }
        return stated_status(&trailers).unwrap_or_else(GrpcStatus::no_trailing_status);
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
            Err(error) => return Err(error.status()),
            Ok(Some(message)) => match until_stopped(stop, delivery.message(message)).await {
                Some(true) => {}
                _ => return Err(GrpcStatus::cancelled()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::call::{create, RecvResult};
    use super::super::status::GrpcStatusCode;

    /// The queue is full, the channel is closed, and the peer's status still reaches the reader.
    ///
    /// Both halves matter: the message the driver had already queued, and the terminal behind it.
    /// Sharing one queue meant the terminal waited for room, and the only thing that could end
    /// that wait was the channel closing - which discarded a status the peer had given.
    #[tokio::test]
    async fn a_status_the_peer_gave_outlives_the_channel_that_carried_it() {
        let (closed, closed_rx) = watch::channel(false);
        let (call, _body, driving) = create(1, closed_rx);
        let (_send, mut recv, _control) = call.split();

        let Driving { delivery, .. } = driving;
        assert!(
            delivery.message(Bytes::from_static(b"queued")).await,
            "a window of one takes the first message"
        );

        closed.send_replace(true);
        delivery.end(GrpcStatus::new(GrpcStatusCode::Ok, ""));

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
