//! A minimal, hand-rolled gRPC service to point this library at.
//!
//! gRPC has no wire-level notion of a call *shape*: whether a method is unary, client-streaming,
//! server-streaming or bidirectional is a service-definition convention, not something HTTP/2
//! enforces. One server-side handler implementing `tonic::server::StreamingService` - the fully
//! general shape - therefore serves all four, which is why these tests need no generated protobuf
//! types at all: they speak the same opaque bytes this library does.
//!
//! The service runs on its **own** runtime, separate from the one inside the library under test, and
//! the test bodies are plain synchronous code. That is not incidental: the ABI is driven by posting
//! commands and waiting for events, so a test that drove it from inside an `async` block would be
//! occupying the very thread the server needs in order to answer.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use armonik_transport::reexports::http;
use bytes::{Buf, BufMut, Bytes};
use futures::Stream;
use tokio::sync::mpsc;
use tonic::body::Body;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::metadata::MetadataMap;
use tonic::server::{Grpc, NamedService};
use tonic::{Code, Request, Response, Status, Streaming};
use tower_service::Service;

/// The one method path every [`TestService`] answers to.
pub(crate) const METHOD_PATH: &str = "/armonik_transport_ffi.test.Raw/Call";

/// How long [`Reply::AnswerThenAbandon`] leaves the call open before resetting it.
///
/// Long enough for a caller to fill the flow-control window and be left with a write it cannot get
/// admitted, which is the state the reset has to land in.
pub(crate) const ANSWER_THEN_ABANDON_DELAY: Duration = Duration::from_millis(500);

/// Message-size ceiling for the test services.
///
/// `tonic` defaults to 4 MiB in each direction. The tests deliberately send more than that, so what
/// a large message is measured against stays the HTTP/2 flow-control window rather than a framing
/// policy neither side of this ABI has any say in.
pub(crate) const MAX_MESSAGE: usize = 64 * 1024 * 1024;

/// The response stream a handler returns. Spelled out rather than borrowed from `tonic`, which keeps
/// its own alias private.
pub(crate) type ResponseStream = Pin<Box<dyn Stream<Item = Result<Bytes, Status>> + Send>>;

/// The raw-bytes codec: the service moves whole gRPC messages as opaque bytes, exactly as the ABI
/// under test does.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BytesCodec;

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
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        dst.reserve(item.len());
        dst.put_slice(&item);
        Ok(())
    }
}

impl Decoder for BytesCodec {
    type Item = Bytes;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        let len = src.remaining();
        Ok(Some(src.copy_to_bytes(len)))
    }
}

/// What the service answers with.
#[derive(Clone)]
enum Reply {
    /// These items, in order, regardless of what was received - the request stream is drained first,
    /// as a unary or client-streaming handler would.
    Canned(Vec<Result<Bytes, Status>>),
    /// One reply per request message, emitted **as each arrives** rather than after the request
    /// stream ends. This is what makes a bidirectional test meaningful: the two directions really do
    /// interleave, so the ABI has to be able to receive while it is still sending.
    EchoEach(Bytes),
    /// Never answer, and never finish. Drains the request stream first, so the client's request side
    /// completes normally and only the response is missing - which is what a timeout has to cope
    /// with.
    Hang,
    /// Never answer, and never read the request stream either. The HTTP/2 flow-control window fills
    /// and stays shut, so a chunk handed to the ABI is never admitted - which is what a cancellation
    /// before the response headers has to cope with.
    HangWithoutReading,
    /// Answer with headers straight away, never read the request stream, and end the call a moment
    /// later by dropping both halves of it.
    ///
    /// A peer that resets a stream the caller is still writing to. The window stays shut, so a chunk
    /// handed to the ABI sits there armed; then the request stream is dropped, which resets it, and
    /// the caller's request body dies under an outstanding write.
    AnswerThenAbandon,
}

/// A `tonic`-servable, single-method gRPC service, driven entirely by a [`Reply`] plus optional
/// failure injection.
#[derive(Clone)]
pub(crate) struct TestService {
    reply: Reply,
    /// Calls still to be rejected before the service starts succeeding, and the status to reject
    /// them with.
    fail_remaining: Arc<AtomicU32>,
    failure_code: Code,
    /// Trailers to attach to an injected failure, so a test can assert they reach the terminal
    /// event.
    failure_trailers: MetadataMap,
    /// The request messages of every call, in arrival order.
    received: Arc<Mutex<Vec<Vec<Bytes>>>>,
    /// The request headers of every call, in arrival order.
    headers: Arc<Mutex<Vec<MetadataMap>>>,
    /// How each request stream of an [`Reply::EchoEach`] call ended: `None` for a clean end of
    /// stream, `Some(code)` for one the peer broke.
    ///
    /// The one thing a handler can say about a client that went away. `tonic` turns a client
    /// RST_STREAM(CANCEL) on a request stream into a clean end of stream on purpose, so a
    /// cancellation is not observable as a cancellation - only as an end.
    stream_ends: Arc<Mutex<Vec<Option<Code>>>>,
}

impl TestService {
    /// Answer with `responses`, whatever the request was. One item is a unary reply, several a
    /// server-streaming one.
    pub(crate) fn canned(responses: impl IntoIterator<Item = Bytes>) -> Self {
        Self::new(Reply::Canned(responses.into_iter().map(Ok).collect()))
    }

    /// Echo every request message back with `suffix` appended, one reply per message, as they
    /// arrive.
    pub(crate) fn echo_each(suffix: impl Into<Bytes>) -> Self {
        Self::new(Reply::EchoEach(suffix.into()))
    }

    /// Read the request, then never answer.
    pub(crate) fn hang() -> Self {
        Self::new(Reply::Hang)
    }

    /// Never read the request and never answer.
    pub(crate) fn hang_without_reading() -> Self {
        Self::new(Reply::HangWithoutReading)
    }

    /// Answer without reading the request, then reset the stream after `ANSWER_THEN_ABANDON_DELAY`.
    pub(crate) fn answer_then_abandon() -> Self {
        Self::new(Reply::AnswerThenAbandon)
    }

    fn new(reply: Reply) -> Self {
        Self {
            reply,
            fail_remaining: Arc::new(AtomicU32::new(0)),
            failure_code: Code::Unavailable,
            failure_trailers: MetadataMap::new(),
            received: Arc::new(Mutex::new(Vec::new())),
            headers: Arc::new(Mutex::new(Vec::new())),
            stream_ends: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Reject the first `count` calls with `code`, then start succeeding.
    pub(crate) fn failing_first(mut self, count: u32, code: Code) -> Self {
        self.fail_remaining = Arc::new(AtomicU32::new(count));
        self.failure_code = code;
        self
    }

    /// Attach a trailer to every injected failure.
    pub(crate) fn with_failure_trailer(mut self, key: &'static str, value: &str) -> Self {
        self.failure_trailers
            .insert(key, value.parse().expect("a valid trailer value"));
        self
    }

    /// The request messages of every call so far, in arrival order.
    pub(crate) fn messages_received(&self) -> Vec<Vec<Bytes>> {
        self.received.lock().expect("received lock").clone()
    }

    /// The request headers of every call so far, in arrival order.
    pub(crate) fn headers_received(&self) -> Vec<MetadataMap> {
        self.headers.lock().expect("headers lock").clone()
    }

    /// How each request stream has ended so far: `None` for a clean end, `Some(code)` otherwise.
    pub(crate) fn stream_ends(&self) -> Vec<Option<Code>> {
        self.stream_ends.lock().expect("stream ends lock").clone()
    }
}

impl NamedService for TestService {
    // Only used by `tonic::transport::Server`'s routing table. Every call kind under test reaches it
    // through the same opaque path.
    const NAME: &'static str = "armonik_transport_ffi.test.Raw";
}

/// Streams items out of a channel, so a handler can produce responses while it is still reading
/// requests.
pub(crate) struct ChannelStream(pub(crate) mpsc::Receiver<Result<Bytes, Status>>);

impl Stream for ChannelStream {
    type Item = Result<Bytes, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

/// The gRPC handler itself.
///
/// `tonic::server::StreamingService` is a blanket implementation over a `tower` service of the right
/// shape rather than a trait to implement, so this is where the handler logic goes.
impl Service<Request<Streaming<Bytes>>> for TestService {
    type Response = Response<ResponseStream>;
    type Error = Status;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Streaming<Bytes>>) -> Self::Future {
        let service = self.clone();

        Box::pin(async move {
            // Recorded before anything else, so that a reply which never reads the request stream
            // still leaves its headers behind for a test to read.
            service
                .headers
                .lock()
                .expect("headers lock")
                .push(request.metadata().clone());
            let mut stream = request.into_inner();

            // Emitted as messages arrive, so the two directions genuinely interleave.
            if let Reply::EchoEach(suffix) = &service.reply {
                let suffix = suffix.clone();
                let received = Arc::clone(&service.received);
                let stream_ends = Arc::clone(&service.stream_ends);
                let (tx, rx) = mpsc::channel(8);
                tokio::spawn(async move {
                    let mut seen = Vec::new();
                    let end = loop {
                        match stream.message().await {
                            Ok(Some(message)) => {
                                let mut buffer = message.to_vec();
                                seen.push(message);
                                buffer.extend_from_slice(&suffix);
                                if tx.send(Ok(Bytes::from(buffer))).await.is_err() {
                                    break None;
                                }
                            }
                            Ok(None) => break None,
                            Err(status) => {
                                let code = status.code();
                                let _ = tx.send(Err(status)).await;
                                break Some(code);
                            }
                        }
                    };
                    stream_ends.lock().expect("stream ends lock").push(end);
                    received.lock().expect("received lock").push(seen);
                });
                return Ok(Response::new(Box::pin(ChannelStream(rx)) as ResponseStream));
            }

            if matches!(service.reply, Reply::AnswerThenAbandon) {
                // The response is opened at once and carries nothing, so the caller gets its headers
                // and can start filling a window nobody is reading. Both halves are then dropped
                // together: the request stream, which resets it, and the reply channel, which ends
                // the response.
                let (tx, rx) = mpsc::channel(1);
                tokio::spawn(async move {
                    tokio::time::sleep(ANSWER_THEN_ABANDON_DELAY).await;
                    drop(stream);
                    drop(tx);
                });
                return Ok(Response::new(Box::pin(ChannelStream(rx)) as ResponseStream));
            }

            if matches!(service.reply, Reply::HangWithoutReading) {
                // Deliberately never touching `stream`: dropping it would let the client's send side
                // complete, which is the opposite of what this reply is for. Holding it and awaiting
                // forever is what keeps the flow-control window closed.
                std::future::pending::<()>().await;
                unreachable!("`pending` never resolves");
            }

            // Every other reply drains the request first, the way a real unary or client-streaming
            // handler does - including a failing one, which still has to consume what was sent.
            let mut seen = Vec::new();
            while let Some(message) = stream.message().await? {
                seen.push(message);
            }
            service
                .received
                .lock()
                .expect("received lock")
                .push(seen.clone());

            // `checked_sub` rather than a `> 0` test: it both decides and decrements, and returns
            // `None` at zero, so a test may ask for more failures than it actually triggers without
            // underflowing the counter.
            let should_fail = service
                .fail_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok();
            if should_fail {
                let mut status = Status::new(service.failure_code, "injected failure");
                *status.metadata_mut() = service.failure_trailers;
                return Err(status);
            }

            if matches!(service.reply, Reply::Hang) {
                std::future::pending::<()>().await;
                unreachable!("`pending` never resolves");
            }

            let items = match service.reply {
                Reply::Canned(items) => items,
                // `EchoEach`, both hangs and the abandoning reply returned above.
                _ => unreachable!("every other reply is handled above"),
            };
            Ok(Response::new(
                Box::pin(futures::stream::iter(items)) as ResponseStream
            ))
        })
    }
}

impl Service<http::Request<Body>> for TestService {
    type Response = http::Response<Body>;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let mut handler = self.clone();
        Box::pin(async move {
            Ok(Grpc::new(BytesCodec)
                .max_decoding_message_size(MAX_MESSAGE)
                .max_encoding_message_size(MAX_MESSAGE)
                .streaming(&mut handler, request)
                .await)
        })
    }
}

/// The runtime the test services run on.
///
/// Separate from the runtime inside the library under test, and multi-threaded, because a test body
/// drives the ABI synchronously from the thread it was called on: sharing one runtime would mean the
/// server could only make progress while the test was not waiting.
pub(crate) fn server_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build the test server runtime")
    })
}

/// Serve `service` on an ephemeral loopback port and return its `http://` endpoint.
///
/// The server keeps running on [`server_runtime`] for the rest of the process; these are short-lived
/// test binaries, so there is nothing to gain from shutting each one down.
pub(crate) fn serve(service: TestService) -> String {
    format!("http://{}", serve_at(service, 0))
}

/// Serve `service` on `port`, or on an ephemeral one when it is zero, and return the address.
pub(crate) fn serve_at(service: TestService, port: u16) -> SocketAddr {
    server_runtime().block_on(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .expect("bind the test server");
        let address = listener.local_addr().expect("the test server's address");

        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_incoming(tonic::transport::server::TcpIncoming::from(listener))
                .await
                .expect("serve the test service");
        });

        address
    })
}
