use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use http::Request;
use http_body_util::BodyExt;
use tokio::sync::{mpsc, oneshot, watch};

use super::call::{CallControl, OwnedMessage, RecvResult, RequestBody};
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
        messages: mpsc::Sender<RecvResult>,
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
    delivery.end(&mut stop, status).await;
}

struct Stop {
    over: watch::Receiver<bool>,
    channel_closed: watch::Receiver<bool>,
}

impl Stop {
    async fn channel_closed(&mut self) {
        let _ = self.channel_closed.wait_for(|closed| *closed).await;
    }

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

    async fn end(mut self, stop: &mut Stop, status: GrpcStatus) {
        self.head(Metadata::new());
        tokio::select! {
            biased;
            _ = self.messages.send(RecvResult::End(status)) => {}
            _ = stop.channel_closed() => {}
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
