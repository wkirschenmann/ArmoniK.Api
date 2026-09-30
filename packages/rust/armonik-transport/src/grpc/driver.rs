use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::task::Poll;

use bytes::{Buf, BufMut, Bytes};
use http::uri::PathAndQuery;
use http::HeaderMap;
use tokio::sync::{mpsc, oneshot, watch};
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::metadata::MetadataMap;
use tonic::Code;

use super::call::{CallControl, OwnedMessage, RequestMessages};
use super::channel::Inner;
use super::metadata::Metadata;
use super::status::GrpcStatus;

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

/// What a call sends: where, with what metadata, and the messages the caller will write.
pub(crate) struct Outgoing {
    pub(crate) path: PathAndQuery,
    pub(crate) metadata: HeaderMap,
    pub(crate) messages: RequestMessages,
}

pub(crate) async fn drive(inner: Arc<Inner>, outgoing: Outgoing, driving: Driving) {
    let Driving {
        mut stop,
        mut delivery,
        control,
    } = driving;

    // Held for the whole call, not released after the head. `Inner` owns the `closed` sender,
    // and a watch receiver whose senders are all gone answers like one that was told to close - so
    // a driver that let go of it would read a dropped channel handle as a cancellation.
    //
    // Contained, because a panic unwinding past `delivery` would drop the terminal unsent, and the
    // caller would read a driver gone with its runtime rather than a call that failed.
    let status = {
        let mut running = std::pin::pin!(run(&inner, outgoing, &mut stop, &mut delivery));
        std::future::poll_fn(|cx| {
            catch_unwind(AssertUnwindSafe(|| running.as_mut().poll(cx))).unwrap_or_else(|_| {
                Poll::Ready(GrpcStatus::new(
                    Code::Internal,
                    "the task driving the call panicked",
                ))
            })
        })
        .await
    };

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
/// message queue would mean waiting for room in it, and that wait would need a bound: a channel
/// closing while one message sits unread would take the terminal with it, and a call the peer
/// answered OK would reach its reader as `Aborted` - through the FFI, a host reading `Cancelled`
/// for a call it has the response to, and retrying what it must not repeat.
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
    outgoing: Outgoing,
    stop: &mut Stop,
    delivery: &mut Delivery,
) -> GrpcStatus {
    #[cfg(feature = "test-hooks")]
    crate::hooks::run_in_driver();

    let Outgoing {
        path,
        metadata,
        messages,
    } = outgoing;

    let mut request = tonic::Request::new(messages);
    *request.metadata_mut() = MetadataMap::from_headers(metadata);

    let mut client = inner.client();
    let response = match until_stopped(stop, client.streaming(request, path, BytesCodec)).await {
        None => return GrpcStatus::cancelled(),
        Some(Err(status)) => return GrpcStatus::from(status),
        Some(Ok(response)) => response,
    };

    let (head, mut body, _) = response.into_parts();
    let head = head.into_headers();

    // A head that states a status is the Trailers-Only shape, where that one HEADERS frame is the
    // trailers and not initial metadata. tonic ends such a stream empty and leaves the status in
    // the head, so it is read from there, and nothing goes out as a head: delivering those
    // headers twice would have the reader see a head no such response has.
    if let Some(status) = tonic::Status::from_header_map(&head) {
        return GrpcStatus::from(status);
    }
    delivery.head(Metadata::from_headers(&head));

    loop {
        match until_stopped(stop, body.message()).await {
            None => return GrpcStatus::cancelled(),
            Some(Err(status)) => return GrpcStatus::from(past_the_limit(status)),
            Some(Ok(None)) => break,
            Some(Ok(Some(message))) => {
                if until_stopped(stop, delivery.message(message)).await != Some(true) {
                    return GrpcStatus::cancelled();
                }
            }
        }
    }

    match until_stopped(stop, body.trailers()).await {
        None => GrpcStatus::cancelled(),
        Some(Err(status)) => GrpcStatus::from(status),
        Some(Ok(trailers)) => {
            GrpcStatus::ok(&trailers.map(MetadataMap::into_headers).unwrap_or_default())
        }
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

    /// The one copy a message makes on its way out, and the point the caller's buffer is let go.
    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        dst.put(item);
        Ok(())
    }
}

impl Decoder for BytesCodec {
    type Item = Bytes;
    type Error = tonic::Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        Ok(Some(src.copy_to_bytes(src.remaining())))
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
