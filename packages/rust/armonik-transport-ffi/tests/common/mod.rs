//! A minimal, hand-rolled gRPC service for `tests/calls.rs`.
//!
//! gRPC has no distinct wire-level "shape": whether a call is unary, client-streaming,
//! server-streaming or bidirectional is a client-side and service-definition convention, not
//! something HTTP/2 itself enforces. That means one server-side handler implementing
//! `tonic::server::StreamingService` (the fully general shape) can serve all four: it just reads
//! however many request messages arrive before responding with however many response messages the
//! test configures. To keep it simple, this handler always reads the *entire* request stream
//! before producing any response — real bidirectional streaming would interleave the two, but nothing
//! under test here depends on that; only the FFI's client-side plumbing is being exercised.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use armonik_transport::reexports::http;
use armonik_transport::reexports::tonic::body::Body;
use armonik_transport::reexports::tonic::codec::{
    BoxStream, Codec, DecodeBuf, Decoder, EncodeBuf, Encoder,
};
use armonik_transport::reexports::tonic::server::{Grpc, NamedService, StreamingService};
use armonik_transport::reexports::tonic::{Code, Request, Response, Status, Streaming};
use bytes::{Buf, BufMut, Bytes};
use tower_service::Service;

/// The raw-bytes codec, mirroring `armonik_transport_ffi`'s own `BytesCodec` on the server side.
#[derive(Debug, Clone, Copy, Default)]
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

/// What the test service does with a call, once it has read every request message.
#[derive(Clone)]
pub(crate) enum Reply {
    /// Answer with these messages, regardless of what was received. Covers unary (one message)
    /// and server-streaming (several).
    Canned(Vec<Bytes>),
    /// Answer with one message per message received, each with this suffix appended. Covers
    /// client-streaming (one reply) and bidirectional (several).
    EchoWithSuffix(Bytes),
}

/// A `tonic`-servable, single-method gRPC service driven entirely by [`Reply`] and an optional
/// failure countdown, for exercising the FFI's call machinery without any generated proto types.
#[derive(Clone)]
pub(crate) struct TestService {
    reply: Reply,
    /// Requests still to be rejected before the service starts succeeding, and the status to
    /// reject them with — the server side of a retry test.
    fail_remaining: Arc<AtomicU32>,
    failure_code: Code,
    /// Every call that reached the handler, successful or failed — lets a test assert a retried
    /// call really did retry the expected number of times, not just report the last status.
    attempts: Arc<AtomicU32>,
    /// Every metadata map seen so far, in call order — lets a test assert on what the FFI actually
    /// sent.
    seen_metadata:
        Arc<std::sync::Mutex<Vec<armonik_transport::reexports::tonic::metadata::MetadataMap>>>,
}

impl TestService {
    pub(crate) fn canned(responses: impl IntoIterator<Item = Bytes>) -> Self {
        Self::new(Reply::Canned(responses.into_iter().collect()))
    }

    pub(crate) fn echo(suffix: impl Into<Bytes>) -> Self {
        Self::new(Reply::EchoWithSuffix(suffix.into()))
    }

    fn new(reply: Reply) -> Self {
        Self {
            reply,
            fail_remaining: Arc::new(AtomicU32::new(0)),
            failure_code: Code::Unavailable,
            attempts: Arc::new(AtomicU32::new(0)),
            seen_metadata: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Reject the first `count` calls with `code`, then start succeeding — the server side of a
    /// retry test.
    pub(crate) fn failing_first(mut self, count: u32, code: Code) -> Self {
        self.fail_remaining = Arc::new(AtomicU32::new(count));
        self.failure_code = code;
        self
    }

    /// How many calls have reached the handler so far, successful or failed.
    pub(crate) fn attempts(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.attempts)
    }

    pub(crate) fn metadata_seen(
        &self,
    ) -> Vec<armonik_transport::reexports::tonic::metadata::MetadataMap> {
        self.seen_metadata.lock().unwrap().clone()
    }
}

impl NamedService for TestService {
    // Only used for `tonic::transport::Server`'s routing table; these tests reach it through a
    // single opaque path regardless of the method_kind under test.
    const NAME: &'static str = "armonik_transport_ffi.test.Raw";
}

impl StreamingService<Bytes> for TestService {
    type Response = Bytes;
    type ResponseStream = BoxStream<Bytes>;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Self::ResponseStream>, Status>> + Send>>;

    fn call(&mut self, request: Request<Streaming<Bytes>>) -> Self::Future {
        let reply = self.reply.clone();
        let fail_remaining = Arc::clone(&self.fail_remaining);
        let failure_code = self.failure_code;
        let attempts = Arc::clone(&self.attempts);
        let seen_metadata = Arc::clone(&self.seen_metadata);

        Box::pin(async move {
            attempts.fetch_add(1, Ordering::SeqCst);
            seen_metadata.lock().unwrap().push(request.metadata().clone());

            // Drain the whole request stream before deciding anything: a failed call must still
            // consume what the client sent, the same as a real handler would.
            let mut stream = request.into_inner();
            let mut received = Vec::new();
            while let Some(message) = stream.message().await? {
                received.push(message);
            }

            // Fetch-then-decrement without underflowing, so a test can request more failures than
            // it actually triggers without this panicking.
            let should_fail = fail_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    (remaining > 0).then_some(remaining - 1)
                })
                .is_ok();
            if should_fail {
                return Err(Status::new(failure_code, "injected failure"));
            }

            let responses: Vec<Result<Bytes, Status>> = match reply {
                Reply::Canned(items) => items.into_iter().map(Ok).collect(),
                Reply::EchoWithSuffix(suffix) => received
                    .into_iter()
                    .map(|message| {
                        let mut buffer = message.to_vec();
                        buffer.extend_from_slice(&suffix);
                        Ok(Bytes::from(buffer))
                    })
                    .collect(),
            };

            let stream: BoxStream<Bytes> =
                Box::pin(armonik_transport::reexports::tokio_stream::iter(responses));
            Ok(Response::new(stream))
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
        Box::pin(async move { Ok(Grpc::new(BytesCodec).streaming(&mut handler, request).await) })
    }
}

/// Serve `service` on an ephemeral loopback port, returning the address it is listening on.
pub(crate) async fn spawn_server(service: TestService) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind server");
    let address = listener.local_addr().expect("server address");

    tokio::spawn(async move {
        let incoming =
            armonik_transport::reexports::tokio_stream::wrappers::TcpListenerStream::new(listener);
        armonik_transport::reexports::tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(incoming)
            .await
            .expect("serve");
    });

    address
}

/// The one method path every [`TestService`] answers to.
pub(crate) const METHOD_PATH: &str = "/armonik_transport_ffi.test.Raw/Call";
