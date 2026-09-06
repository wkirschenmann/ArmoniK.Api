use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use armonik_transport::grpc::{
    CallStartOptions, GrpcChannel, GrpcChannelConfig, GrpcChannelConfigError, GrpcStatus,
    GrpcStatusCode, Metadata, RecvHalf, RecvResult,
};
use armonik_transport::http2::TransportConfig;
use armonik_transport::reexports::hyper;
use armonik_transport::reexports::hyper_util::rt::{TokioExecutor as HyperTokio, TokioIo};
use armonik_transport::reexports::tonic::body::Body as TonicBody;
use armonik_transport::reexports::tonic::metadata::{MetadataMap, MetadataValue as TonicValue};
use armonik_transport::reexports::tonic::{Code, Request, Response, Status, Streaming};
use bytes::Bytes;
use http::header::{HeaderMap, HeaderValue};
use http::{StatusCode, Uri};
use hyper::body::{Body, Frame, Incoming};
use tower_service::Service;

use super::codec::BytesCodec;

pub const ECHO: &str = "/armonik_transport.test.Echo/Echo";
pub const FAIL: &str = "/armonik_transport.test.Echo/Fail";
pub const SLOW: &str = "/armonik_transport.test.Echo/Slow";
pub const COLLECT: &str = "/armonik_transport.test.Echo/Collect";

/// A listener on an ephemeral loopback port, and the endpoint that reaches it.
pub async fn loopback() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the test server");
    let address = listener.local_addr().expect("the test server's address");
    (listener, format!("http://{address}"))
}

pub fn channel(endpoint: &str) -> GrpcChannel {
    let uri = Uri::try_from(endpoint).expect("the test server's endpoint");
    let mut config = GrpcChannelConfig::new(TransportConfig::new(uri));
    config.transport.connect_timeout = Duration::from_secs(5);

    channel_with(config).expect("a plain endpoint and default options")
}

pub fn channel_with(config: GrpcChannelConfig) -> Result<GrpcChannel, GrpcChannelConfigError> {
    GrpcChannel::new(config, tokio::runtime::Handle::current())
}

pub async fn unary(
    channel: &GrpcChannel,
    options: CallStartOptions,
    message: Bytes,
) -> (Metadata, Vec<Bytes>, GrpcStatus) {
    let (mut send, mut recv, _control) = channel
        .start_call(options)
        .expect("the call starts")
        .split();

    let _ = send.send_message(message).await;
    let _ = send.end_send().await;

    read_to_terminal(&mut recv).await
}

pub async fn call_on(method: &str, message: Bytes) -> (Metadata, Vec<Bytes>, GrpcStatus) {
    let server = TestServer::start().await;
    unary(
        &channel(&server.endpoint),
        CallStartOptions::new(method),
        message,
    )
    .await
}

pub async fn ends_cancelled(recv: &mut RecvHalf, why: &str) {
    let terminal = tokio::time::timeout(Duration::from_secs(5), recv.next_message())
        .await
        .unwrap_or_else(|_| panic!("{why}"))
        .expect("a terminal");

    match terminal {
        RecvResult::End(status) => assert_eq!(status.code, GrpcStatusCode::Cancelled),
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

pub type Answer = Pin<Box<dyn Future<Output = Result<Response<Bytes>, Status>> + Send>>;

#[derive(Clone, Copy)]
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

/// Reads every request message and answers once, naming how many it saw and their contents.
///
/// Through tonic rather than a canned body, because this is the direction where the server has to
/// decode what the client framed, and tonic is the reference for that.
#[derive(Clone, Copy)]
pub struct Collector;

impl armonik_transport::reexports::tonic::server::ClientStreamingService<Bytes> for Collector {
    type Response = Bytes;
    type Future = Answer;

    fn call(&mut self, request: Request<Streaming<Bytes>>) -> Self::Future {
        Box::pin(async move {
            let mut stream = request.into_inner();
            let mut seen: Vec<String> = Vec::new();
            while let Some(message) = stream.message().await? {
                seen.push(String::from_utf8_lossy(&message).into_owned());
            }
            Ok(Response::new(Bytes::from(format!(
                "{}:{}",
                seen.len(),
                seen.join(",")
            ))))
        })
    }
}

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

pub fn slow(_request: Request<Bytes>) -> Answer {
    Box::pin(async move {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        Ok(Response::new(Bytes::new()))
    })
}

pub async fn answer(request: hyper::Request<Incoming>) -> hyper::Response<TonicBody> {
    use armonik_transport::reexports::tonic::server::Grpc;

    let path = request.uri().path().to_owned();
    if let Some(raw) = path.strip_prefix("/raw/") {
        return canned(raw, request.headers());
    }

    if path == COLLECT {
        return Grpc::new(BytesCodec)
            .max_decoding_message_size(usize::MAX)
            .max_encoding_message_size(usize::MAX)
            .client_streaming(Collector, request.map(TonicBody::new))
            .await;
    }

    let handler = match path.as_str() {
        ECHO => echo,
        FAIL => fail,
        SLOW => slow,
        _ => return Status::unimplemented("no such method").into_http(),
    };

    Grpc::new(BytesCodec)
        .max_decoding_message_size(usize::MAX)
        .max_encoding_message_size(usize::MAX)
        .unary(&mut Handler(handler), request.map(TonicBody::new))
        .await
}

pub struct Canned {
    frames: std::vec::IntoIter<Frame<Bytes>>,
    /// Fails once the frames are out, rather than ending.
    ///
    /// It is how hyper's server is made to send a RST_STREAM: a body that errors resets the
    /// stream with INTERNAL_ERROR, where a body that ends finishes the response.
    then_fails: bool,
}

impl Body for Canned {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if let Some(frame) = self.frames.next() {
            return Poll::Ready(Some(Ok(frame)));
        }
        if std::mem::replace(&mut self.then_fails, false) {
            return Poll::Ready(Some(Err(std::io::Error::other(
                "the canned body fails here",
            ))));
        }
        Poll::Ready(None)
    }
}

fn framed(flag: u8, declared: u32, payload: &[u8]) -> Bytes {
    let mut message = Vec::with_capacity(5 + payload.len());
    message.push(flag);
    message.extend_from_slice(&declared.to_be_bytes());
    message.extend_from_slice(payload);
    Bytes::from(message)
}

pub fn grpc_message(flag: u8, payload: &[u8]) -> Bytes {
    framed(flag, payload.len() as u32, payload)
}

/// A length the payload does not honour, which is what a receiver's limit is checked against.
pub fn announced_message(declared: u32, payload: &[u8]) -> Bytes {
    framed(0, declared, payload)
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
        "NoTrailers" => (grpc_head(), vec![Frame::data(grpc_message(0, b"orphan"))]),
        // A body that fails mid-stream, which is how hyper's server is made to send a
        // RST_STREAM: it resets with INTERNAL_ERROR rather than finishing the response.
        "ResetsMidBody" => (
            grpc_head(),
            vec![Frame::data(grpc_message(0, b"before the reset"))],
        ),
        other => panic!("no canned response is named `{other}`"),
    };

    builder
        .body(TonicBody::new(Canned {
            frames: frames.into_iter(),
            then_fails: case == "ResetsMidBody",
        }))
        .expect("a well-formed canned response")
}

pub fn grpc_head() -> hyper::http::response::Builder {
    hyper::Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/grpc")
}

pub struct TestServer {
    pub endpoint: String,
    connections: Arc<AtomicUsize>,
}

impl TestServer {
    pub async fn start() -> Self {
        let (listener, endpoint) = loopback().await;
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
            endpoint,
            connections,
        }
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }
}

pub async fn closed_port() -> String {
    let (listener, endpoint) = loopback().await;
    drop(listener);
    endpoint
}
