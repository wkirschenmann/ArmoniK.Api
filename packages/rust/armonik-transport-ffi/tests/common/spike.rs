//! The service the C# harness calls: every checklist behaviour on one endpoint.
//!
//! [`super::server`] builds one service per behaviour and points a request at it, which is all a
//! Rust test needs. `Grpc.Net.Client` cannot work that way: a channel reaches a whole service, and
//! the go/no-go checklist needs all four call shapes plus a hang and a failure against that one
//! endpoint. So here the behaviour is keyed by method name instead.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll};

use armonik_transport::reexports::http;
use armonik_transport::reexports::tonic::body::Body;
use armonik_transport::reexports::tonic::server::{Grpc, NamedService};
use armonik_transport::reexports::tonic::{Code, Request, Response, Status, Streaming};
use bytes::Bytes;
use tokio::sync::mpsc;
use tower_service::Service;

use super::server::{
    server_runtime, BytesCodec, ChannelStream, Incoming, ResponseStream, MAX_MESSAGE,
};

/// What the spike service does for a given method.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    /// One message in, one message out, the payload unchanged.
    Unary,
    /// One message in, [`SERVER_STREAM_REPLIES`] messages out, each payload suffixed with its index.
    ServerStream,
    /// Any number of messages in, one message out carrying every payload concatenated.
    ClientStream,
    /// One message out per message in, emitted as each arrives - strict alternation, so nothing on
    /// either side may buffer.
    Bidi,
    /// Read everything, then never answer.
    Hang,
    /// Fail before any header is written: a trailers-only response.
    Fail,
    /// Like [`Behaviour::Bidi`], but counts the request streams that end.
    ///
    /// The client of this method never half-closes, so an end can only mean the peer reset the
    /// stream. That indirection is forced: `tonic` turns a client RST_STREAM(CANCEL) on a request
    /// stream into a clean end of stream on purpose (`tonic::codec::decode`, the
    /// `direction == Request && code == Cancelled` arm), so a handler cannot see the cancellation
    /// as a cancellation. What it can see, and what actually matters, is that it stops waiting.
    CancelWatch,
    /// Report how many request streams [`Behaviour::CancelWatch`] has seen end.
    CancelsObserved,
}

/// How many replies [`Behaviour::ServerStream`] produces.
pub(crate) const SERVER_STREAM_REPLIES: usize = 5;

/// `CancelWatch` request streams that have ended, process-wide.
static CANCELS_OBSERVED: AtomicU32 = AtomicU32::new(0);

/// The method names the spike service answers to, under `/armonik_transport_ffi.test.Raw/`.
fn behaviour_of(path: &str) -> Option<Behaviour> {
    match path.rsplit('/').next()? {
        "Unary" => Some(Behaviour::Unary),
        "ServerStream" => Some(Behaviour::ServerStream),
        "ClientStream" => Some(Behaviour::ClientStream),
        "Bidi" => Some(Behaviour::Bidi),
        "CancelWatch" => Some(Behaviour::CancelWatch),
        "Hang" => Some(Behaviour::Hang),
        "Fail" => Some(Behaviour::Fail),
        "CancelsObserved" => Some(Behaviour::CancelsObserved),
        _ => None,
    }
}

/// Encode an `EchoMsg { bytes payload = 1 }`.
///
/// By hand rather than through `prost`: this crate has no protobuf codegen, and acquiring some for
/// one field would put `protoc` back in a build that is deliberately without it. Field 1, wire type
/// 2, is the single byte `0x0A`, followed by the length as a varint.
pub(crate) fn encode_echo(payload: &[u8]) -> Bytes {
    let mut out = Vec::with_capacity(payload.len() + 6);
    out.push(0x0A);
    let mut len = payload.len() as u64;
    loop {
        let byte = (len & 0x7F) as u8;
        len >>= 7;
        if len == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
    out.extend_from_slice(payload);
    Bytes::from(out)
}

/// Read the payload back out of an `EchoMsg`.
///
/// An empty message is an empty payload: a protobuf encoder omits a field holding its default.
pub(crate) fn decode_echo(message: &[u8]) -> Result<Vec<u8>, Status> {
    if message.is_empty() {
        return Ok(Vec::new());
    }
    if message[0] != 0x0A {
        return Err(Status::invalid_argument("not an EchoMsg"));
    }
    let mut len = 0u64;
    let mut shift = 0;
    let mut index = 1;
    loop {
        let byte = *message
            .get(index)
            .ok_or_else(|| Status::invalid_argument("truncated EchoMsg length"))?;
        len |= u64::from(byte & 0x7F) << shift;
        index += 1;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    let end = index + len as usize;
    message
        .get(index..end)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| Status::invalid_argument("truncated EchoMsg payload"))
}

/// The gRPC code of the last `CancelWatch` request stream that ended, for diagnosis. `CLEAN_END`
/// means it ended without an error, which is what an absorbed RST_STREAM(CANCEL) looks like;
/// `u32::MAX` means none has ended yet.
static LAST_STREAM_ERROR: AtomicU32 = AtomicU32::new(u32::MAX);

/// What [`LAST_STREAM_ERROR`] holds when a request stream ended without an error.
const CLEAN_END: u32 = 1000;

/// The handler behind one method of the spike service.
#[derive(Clone, Copy)]
struct SpikeHandler(Behaviour);

impl Service<Request<Streaming<Bytes>>> for SpikeHandler {
    type Response = Response<ResponseStream>;
    type Error = Status;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Streaming<Bytes>>) -> Self::Future {
        let behaviour = self.0;
        Box::pin(async move {
            let mut stream = request.into_inner();

            if behaviour == Behaviour::Fail {
                return Err(Status::new(Code::FailedPrecondition, "refused on purpose"));
            }

            if behaviour == Behaviour::CancelsObserved {
                let count = format!(
                    "{}:{}",
                    CANCELS_OBSERVED.load(Ordering::SeqCst),
                    LAST_STREAM_ERROR.load(Ordering::SeqCst)
                );
                let items = vec![Ok(encode_echo(count.as_bytes()))];
                return Ok(Response::new(
                    Box::pin(armonik_transport::reexports::tokio_stream::iter(items))
                        as ResponseStream,
                ));
            }

            // Answers while it is still reading, which is the only shape that proves the two
            // directions really interleave. Capacity one, so a reply that nobody reads stops the
            // handler rather than letting it run ahead.
            if behaviour == Behaviour::Bidi || behaviour == Behaviour::CancelWatch {
                let watching = behaviour == Behaviour::CancelWatch;
                let (tx, rx) = mpsc::channel(1);
                tokio::spawn(async move {
                    loop {
                        match stream.message().await {
                            Ok(Some(message)) => {
                                let payload = match decode_echo(&message) {
                                    Ok(payload) => payload,
                                    Err(status) => {
                                        let _ = tx.send(Err(status)).await;
                                        break;
                                    }
                                };
                                if tx.send(Ok(encode_echo(&payload))).await.is_err() {
                                    break;
                                }
                            }
                            Ok(None) => {
                                if watching {
                                    LAST_STREAM_ERROR.store(CLEAN_END, Ordering::SeqCst);
                                    CANCELS_OBSERVED.fetch_add(1, Ordering::SeqCst);
                                }
                                break;
                            }
                            Err(status) => {
                                if watching {
                                    LAST_STREAM_ERROR.store(status.code() as u32, Ordering::SeqCst);
                                    CANCELS_OBSERVED.fetch_add(1, Ordering::SeqCst);
                                }
                                let _ = tx.send(Err(status)).await;
                                break;
                            }
                        }
                    }
                });
                return Ok(Response::new(Box::pin(ChannelStream(rx)) as ResponseStream));
            }

            // Every other shape reads the whole request first.
            let mut payloads = Vec::new();
            loop {
                match stream.message().await {
                    Ok(Some(message)) => payloads.push(decode_echo(&message)?),
                    Ok(None) => break,
                    Err(status) => return Err(status),
                }
            }

            if behaviour == Behaviour::Hang {
                std::future::pending::<()>().await;
                unreachable!("`pending` never resolves");
            }

            let first = payloads.first().cloned().unwrap_or_default();
            let items: Vec<Result<Bytes, Status>> = match behaviour {
                Behaviour::Unary => vec![Ok(encode_echo(&first))],
                Behaviour::ServerStream => (0..SERVER_STREAM_REPLIES)
                    .map(|index| {
                        let mut payload = first.clone();
                        payload.extend_from_slice(format!("#{index}").as_bytes());
                        Ok(encode_echo(&payload))
                    })
                    .collect(),
                Behaviour::ClientStream => vec![Ok(encode_echo(&payloads.concat()))],
                Behaviour::Bidi
                | Behaviour::CancelWatch
                | Behaviour::Hang
                | Behaviour::Fail
                | Behaviour::CancelsObserved => unreachable!("handled above"),
            };
            Ok(Response::new(
                Box::pin(armonik_transport::reexports::tokio_stream::iter(items)) as ResponseStream,
            ))
        })
    }
}

/// Answers anything the spike service does not know about, so an unknown path comes back as a gRPC
/// status rather than as a connection that goes quiet.
#[derive(Clone, Copy)]
struct UnknownMethod;

impl Service<Request<Streaming<Bytes>>> for UnknownMethod {
    type Response = Response<ResponseStream>;
    type Error = Status;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _request: Request<Streaming<Bytes>>) -> Self::Future {
        Box::pin(async { Err(Status::unimplemented("no such method")) })
    }
}

/// The service the `spike_server` example serves.
#[derive(Clone, Copy, Default)]
pub(crate) struct SpikeService;

impl NamedService for SpikeService {
    const NAME: &'static str = "armonik_transport_ffi.test.Raw";
}

impl Service<http::Request<Body>> for SpikeService {
    type Response = http::Response<Body>;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let behaviour = behaviour_of(request.uri().path());
        Box::pin(async move {
            match behaviour {
                Some(behaviour) => {
                    let mut handler = SpikeHandler(behaviour);
                    Ok(grpc().streaming(&mut handler, request).await)
                }
                None => Ok(grpc().streaming(&mut UnknownMethod, request).await),
            }
        })
    }
}

/// The codec and the message-size ceiling every method here is served with.
fn grpc() -> Grpc<BytesCodec> {
    Grpc::new(BytesCodec)
        .max_decoding_message_size(MAX_MESSAGE)
        .max_encoding_message_size(MAX_MESSAGE)
}

/// Serve [`SpikeService`] on `port`, or on an ephemeral one when it is zero, and return the address.
pub(crate) fn serve_spike(port: u16) -> SocketAddr {
    server_runtime().block_on(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .expect("bind the spike server");
        let address = listener.local_addr().expect("the spike server's address");

        tokio::spawn(async move {
            armonik_transport::reexports::tonic::transport::Server::builder()
                .add_service(SpikeService)
                .serve_with_incoming(Incoming(listener))
                .await
                .expect("serve the spike service");
        });

        address
    })
}
