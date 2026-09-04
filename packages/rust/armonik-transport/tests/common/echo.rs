//! An echo server for the engine to talk to, and the helpers a call needs.
//!
//! The gRPC methods are served by `tonic`, so what a test exercises is this engine's framing and
//! header handling against an implementation that owes it nothing. The responses `tonic` will not
//! produce - an HTTP error page, a body that is not gRPC, a compressed message - are canned by
//! hand under `/raw/`, framed by the test rather than by the engine.
//!
//! Here rather than inside one test file because none of it is about unary calls in particular:
//! the streaming cardinalities need the same server and the same call helpers.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use armonik_transport::grpc::{
    CallStartOptions, GrpcChannel, GrpcChannelConfig, GrpcStatus, GrpcStatusCode, Metadata,
    RecvHalf, RecvResult, TokioExecutor,
};
use armonik_transport::http2::TransportConfig;
use armonik_transport::reexports::hyper;
use armonik_transport::reexports::hyper_util::rt::{TokioExecutor as HyperTokio, TokioIo};
use armonik_transport::reexports::tonic::body::Body as TonicBody;
use armonik_transport::reexports::tonic::metadata::{MetadataMap, MetadataValue as TonicValue};
use armonik_transport::reexports::tonic::{Code, Request, Response, Status};
use bytes::Bytes;
use http::header::{HeaderMap, HeaderValue};
use http::{StatusCode, Uri};
use hyper::body::{Body, Frame, Incoming};
use tower_service::Service;

use super::codec::BytesCodec;

pub const ECHO: &str = "/armonik_transport.test.Echo/Echo";
pub const FAIL: &str = "/armonik_transport.test.Echo/Fail";
pub const SLOW: &str = "/armonik_transport.test.Echo/Slow";

pub fn channel(endpoint: &str) -> GrpcChannel {
    let uri = Uri::try_from(endpoint).expect("the test server's endpoint");
    let mut config = GrpcChannelConfig::new(TransportConfig::new(uri));
    config.transport.connect_timeout = Duration::from_secs(5);

    GrpcChannel::new(
        config,
        TokioExecutor::new(tokio::runtime::Handle::current()),
    )
    .expect("a plain endpoint and default options")
}

/// One unary call: send `message`, half-close, then read to the terminal.
pub async fn unary(
    channel: &GrpcChannel,
    options: CallStartOptions,
    message: Bytes,
) -> (Metadata, Vec<Bytes>, GrpcStatus) {
    let (mut send, mut recv, _control) = channel
        .start_call(options)
        .expect("the call starts")
        .split();

    // A call can reach its terminal before the request is written - a refused connection, a
    // Trailers-Only refusal - and that is not a failure of the test.
    let _ = send.send_message(message).await;
    let _ = send.end_send().await;

    read_to_terminal(&mut recv).await
}

/// One unary call to `method`, on a server started for it alone.
pub async fn call_on(method: &str, message: Bytes) -> (Metadata, Vec<Bytes>, GrpcStatus) {
    let server = TestServer::start().await;
    unary(
        &channel(&server.endpoint),
        CallStartOptions::new(method),
        message,
    )
    .await
}

/// Reads to the terminal and expects a cancellation there, within a bound a loaded machine
/// keeps.
pub async fn ends_cancelled(recv: &mut RecvHalf, why: &str) {
    let terminal = tokio::time::timeout(Duration::from_secs(5), recv.next_message())
        .await
        .unwrap_or_else(|_| panic!("{why}"))
        .expect("a terminal");

    match terminal {
        RecvResult::End(status) => assert_eq!(status.code, GrpcStatusCode::CANCELLED),
        other => panic!("{other:?}"),
    }
}

pub async fn read_to_terminal(recv: &mut RecvHalf) -> (Metadata, Vec<Bytes>, GrpcStatus) {
    let head = recv
        .recv_initial_metadata()
        .await
        .expect("a response head, even an empty one")
        .clone();

    let mut messages = Vec::new();
    loop {
        match recv.next_message().await.expect("a message or a terminal") {
            RecvResult::Message(message) => messages.push(message.data),
            RecvResult::End(status) => return (head, messages, status),
        }
    }
}

/// What a handler does with a request, once `tonic` has decoded it.
pub type Answer = Pin<Box<dyn Future<Output = Result<Response<Bytes>, Status>> + Send>>;

#[derive(Clone, Copy)]
/// One gRPC method, as the function that answers it.
pub struct Handler(fn(Request<Bytes>) -> Answer);

impl Service<Request<Bytes>> for Handler {
    type Response = Response<Bytes>;
    type Error = Status;
    type Future = Answer;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Bytes>) -> Self::Future {
        (self.0)(request)
    }
}

/// Echoes the request, and echoes back whatever `x-request` metadata came with it.
pub fn echo(request: Request<Bytes>) -> Answer {
    let text = request
        .metadata()
        .get("x-request")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| TonicValue::try_from(value).ok());
    let binary = request
        .metadata()
        .get_bin("x-request-bin")
        .and_then(|value| value.to_bytes().ok());

    Box::pin(async move {
        let mut response = Response::new(request.into_inner());
        if let Some(text) = text {
            response.metadata_mut().insert("x-echoed", text);
        }
        if let Some(binary) = binary {
            response
                .metadata_mut()
                .insert_bin("x-echoed-bin", TonicValue::from_bytes(&binary));
        }
        Ok(response)
    })
}

/// Refuses, with a reason in the trailers.
pub fn fail(_request: Request<Bytes>) -> Answer {
    Box::pin(async move {
        let mut metadata = MetadataMap::new();
        metadata.insert("x-reason", TonicValue::from_static("policy"));
        Err(Status::with_metadata(
            Code::PermissionDenied,
            "not for you",
            metadata,
        ))
    })
}

/// Never answers within the life of a test, so a call on it ends only because it was stopped.
pub fn slow(_request: Request<Bytes>) -> Answer {
    Box::pin(async move {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        Ok(Response::new(Bytes::new()))
    })
}

/// The gRPC methods, plus the canned responses gRPC servers do not produce.
pub async fn answer(request: hyper::Request<Incoming>) -> hyper::Response<TonicBody> {
    use armonik_transport::reexports::tonic::server::Grpc;

    let path = request.uri().path().to_owned();
    if let Some(raw) = path.strip_prefix("/raw/") {
        return canned(raw, request.headers());
    }

    let handler = match path.as_str() {
        ECHO => echo,
        FAIL => fail,
        SLOW => slow,
        _ => return Status::unimplemented("no such method").into_http(),
    };

    Grpc::new(BytesCodec)
        // The engine's own maximum is what these tests are about, so the server imposes none.
        .max_decoding_message_size(usize::MAX)
        .max_encoding_message_size(usize::MAX)
        .unary(&mut Handler(handler), request.map(TonicBody::new))
        .await
}

/// A body that hands over frames already decided on.
pub struct Canned {
    frames: std::vec::IntoIter<Frame<Bytes>>,
}

impl Body for Canned {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(self.frames.next().map(Ok))
    }
}

/// One gRPC message, framed by hand, so what these responses send owes nothing to the engine
/// under test.
pub fn grpc_message(flag: u8, payload: &[u8]) -> Bytes {
    let mut framed = Vec::with_capacity(5 + payload.len());
    framed.push(flag);
    framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    framed.extend_from_slice(payload);
    Bytes::from(framed)
}

/// A frame header announcing `declared` bytes, followed by fewer of them.
pub fn announced_message(declared: u32, payload: &[u8]) -> Bytes {
    let mut framed = Vec::with_capacity(5 + payload.len());
    framed.push(0);
    framed.extend_from_slice(&declared.to_be_bytes());
    framed.extend_from_slice(payload);
    Bytes::from(framed)
}

pub fn trailers(pairs: &[(&'static str, &'static str)]) -> Frame<Bytes> {
    let mut map = HeaderMap::new();
    for (key, value) in pairs {
        map.insert(*key, HeaderValue::from_static(value));
    }
    Frame::trailers(map)
}

pub fn canned(case: &str, request: &HeaderMap) -> hyper::Response<TonicBody> {
    let (builder, frames): (_, Vec<Frame<Bytes>>) = match case {
        "EchoHeaders" => {
            let seen: Vec<String> = ["content-type", "te", "grpc-accept-encoding", "user-agent"]
                .iter()
                .filter_map(|key| {
                    let value = request.get(*key)?.to_str().ok()?;
                    Some(format!("{key}={value}"))
                })
                .collect();
            (
                grpc_head(),
                vec![
                    Frame::data(grpc_message(0, seen.join(" ").as_bytes())),
                    trailers(&[("grpc-status", "0")]),
                ],
            )
        }
        "NotFound" => (
            hyper::Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("content-type", "text/html"),
            vec![Frame::data(Bytes::from_static(b"<h1>no</h1>"))],
        ),
        "StatusBehindError" => (
            hyper::Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("content-type", "application/grpc")
                .header("grpc-status", "8")
                .header("grpc-message", "no%20room%20left"),
            vec![],
        ),
        "NotGrpc" => (
            hyper::Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/plain"),
            vec![Frame::data(Bytes::from_static(b"an ordinary web page"))],
        ),
        "TooBig" => (
            grpc_head(),
            vec![Frame::data(announced_message(
                64 * 1024 * 1024,
                b"only a taste",
            ))],
        ),
        "Compressed" => (
            grpc_head(),
            vec![
                Frame::data(grpc_message(1, b"squeezed")),
                trailers(&[("grpc-status", "0")]),
            ],
        ),
        "HeadThenError" => (
            grpc_head().header("x-head", "present"),
            vec![
                Frame::data(grpc_message(0, b"partial")),
                trailers(&[("grpc-status", "8"), ("grpc-message", "no%20room%20left")]),
            ],
        ),
        // A well-formed message and then nothing: a stream that never says how it ended.
        "NoTrailers" => (grpc_head(), vec![Frame::data(grpc_message(0, b"orphan"))]),
        other => panic!("no canned response is named `{other}`"),
    };

    builder
        .body(TonicBody::new(Canned {
            frames: frames.into_iter(),
        }))
        .expect("a well-formed canned response")
}

pub fn grpc_head() -> hyper::http::response::Builder {
    hyper::Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/grpc")
}

/// The server the tests call, and the count of connections it has accepted.
pub struct TestServer {
    // read by the tests, which live in another module now
    pub endpoint: String,
    connections: Arc<AtomicUsize>,
}

impl TestServer {
    /// Serves on an ephemeral loopback port, for as long as the test runs.
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the test server");
        let address = listener.local_addr().expect("the test server's address");
        let connections = Arc::new(AtomicUsize::new(0));

        let accepted = connections.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                accepted.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(|request| async {
                        Ok::<_, Infallible>(answer(request).await)
                    });
                    let _ = hyper::server::conn::http2::Builder::new(HyperTokio::new())
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });

        Self {
            endpoint: format!("http://{address}"),
            connections,
        }
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }
}

/// An endpoint that was listening and is not any more.
pub async fn closed_port() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind to find a free port");
    let address = listener.local_addr().expect("the port that just closed");
    drop(listener);
    format!("http://{address}")
}
