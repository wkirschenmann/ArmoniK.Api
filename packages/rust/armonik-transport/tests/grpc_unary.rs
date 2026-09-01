//! Unary calls, over plain HTTP/2, against a real gRPC server.
//!
//! The gRPC methods are served by `tonic`, so what these tests exercise is this engine's framing
//! and header handling against an implementation that owes it nothing. The responses `tonic` will
//! not produce - an HTTP error page, a body that is not gRPC, a compressed message - are canned by
//! hand under `/raw/`, framed by the test rather than by the engine under test.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use armonik_transport::grpc::{
    CallError, CallStartOptions, ChannelError, GrpcChannel, GrpcChannelConfig, GrpcStatus,
    GrpcStatusCode, Metadata, MetadataValue, RecvHalf, RecvResult, TokioExecutor,
};
use armonik_transport::http2::{TransportConfig, TransportErrorKind};
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

#[path = "common/codec.rs"]
mod codec;

use codec::BytesCodec;

const ECHO: &str = "/armonik_transport.test.Echo/Echo";
const FAIL: &str = "/armonik_transport.test.Echo/Fail";
const SLOW: &str = "/armonik_transport.test.Echo/Slow";

// ---------------------------------------------------------------- driving the engine

fn channel(endpoint: &str) -> GrpcChannel {
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
async fn unary(
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

async fn read_to_terminal(recv: &mut RecvHalf) -> (Metadata, Vec<Bytes>, GrpcStatus) {
    let head = recv
        .recv_initial_metadata()
        .await
        .expect("a response head, even an empty one");

    let mut messages = Vec::new();
    loop {
        match recv.next_message().await.expect("a message or a terminal") {
            RecvResult::Message(message) => messages.push(message.data),
            RecvResult::End(status) => return (head, messages, status),
        }
    }
}

// ---------------------------------------------------------------- the deliverable

#[tokio::test]
async fn a_unary_call_reaches_a_grpc_server_and_comes_back() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let mut options = CallStartOptions::new(ECHO);
    options
        .metadata
        .append_ascii("x-request", "ping")
        .expect("a plain metadata entry");

    let (head, messages, status) = unary(&channel, options, Bytes::from_static(b"hello")).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::from_static(b"hello")]);
    assert_eq!(
        head.get("x-echoed"),
        Some(&MetadataValue::Ascii("ping".to_owned())),
        "the request metadata reached the server and its answer came back"
    );
}

#[tokio::test]
async fn an_empty_message_is_a_message_and_not_an_absence() {
    let server = TestServer::start().await;
    let (_, messages, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new(ECHO),
        Bytes::new(),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages, vec![Bytes::new()]);
}

#[tokio::test]
async fn a_message_larger_than_one_http2_frame_survives_the_round_trip() {
    // Well past the 16 KiB default frame size, so the response arrives in several chunks and the
    // deframer has to put the message back together.
    const SIZE: usize = 256 * 1024;

    let server = TestServer::start().await;
    let (_, messages, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new(ECHO),
        Bytes::from(vec![0x5a; SIZE]),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].len(), SIZE);
    assert!(messages[0].iter().all(|byte| *byte == 0x5a));
}

#[tokio::test]
async fn a_binary_metadata_entry_crosses_the_wire_as_bytes() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let mut options = CallStartOptions::new(ECHO);
    options
        .metadata
        .append_binary("x-request-bin", Bytes::from_static(&[0, 1, 2, 0xff]))
        .expect("a binary entry under a binary key");

    let (head, _, status) = unary(&channel, options, Bytes::from_static(b"x")).await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    assert_eq!(
        head.get("x-echoed-bin"),
        Some(&MetadataValue::Binary(Bytes::from_static(&[0, 1, 2, 0xff])))
    );
}

#[tokio::test]
async fn the_request_carries_the_headers_grpc_asks_for() {
    let server = TestServer::start().await;
    let (_, messages, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new("/raw/EchoHeaders"),
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
    let seen = String::from_utf8(messages.concat().to_vec()).expect("the headers as text");
    for expected in [
        "content-type=application/grpc",
        "te=trailers",
        "grpc-accept-encoding=identity",
        "user-agent=armonik-transport/",
    ] {
        assert!(seen.contains(expected), "{expected} missing from {seen}");
    }
}

#[tokio::test]
async fn a_method_the_server_refuses_comes_back_as_its_status_and_its_trailers() {
    let server = TestServer::start().await;
    let (head, messages, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new(FAIL),
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::PermissionDenied);
    assert_eq!(status.message, "not for you");
    assert_eq!(
        status.trailing_metadata.get("x-reason"),
        Some(&MetadataValue::Ascii("policy".to_owned()))
    );
    assert!(messages.is_empty());
    // A refusal is a Trailers-Only response: there is no response head, and the contract is that
    // this reads as empty metadata rather than as a failure.
    assert!(head.is_empty(), "{head:?}");
}

#[tokio::test]
async fn a_method_the_server_does_not_have_is_unimplemented() {
    let server = TestServer::start().await;
    let (_, _, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new("/armonik_transport.test.Echo/Absent"),
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Unimplemented, "{status}");
    // The server's own words, so this is the gRPC status and not the HTTP 404 mapping, which
    // happens to produce the same code.
    assert_eq!(status.message, "no such method");
}

#[tokio::test]
async fn calls_on_one_channel_share_one_session() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let mut running = Vec::new();
    for _ in 0..8 {
        let channel = channel.clone();
        running.push(tokio::spawn(async move {
            unary(
                &channel,
                CallStartOptions::new(ECHO),
                Bytes::from_static(b"concurrent"),
            )
            .await
        }));
    }

    for call in running {
        let (_, messages, status) = call.await.expect("the call ran to its end");
        assert_eq!(status.code, GrpcStatusCode::Ok, "{status}");
        assert_eq!(messages, vec![Bytes::from_static(b"concurrent")]);
    }

    assert_eq!(
        server.connections(),
        1,
        "eight calls, one HTTP/2 connection"
    );
}

// ---------------------------------------------------------------- the end of a call

#[tokio::test]
async fn nothing_can_be_sent_once_the_call_has_reached_its_terminal() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    // A canned response answers without reading the request, so the writing side is still open
    // when the terminal lands - which is the state under test. The reading half stays bound too:
    // dropping it would end the call by itself and prove nothing about the terminal.
    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new("/raw/HeadThenError"))
        .expect("the call starts")
        .split();

    let (_, _, status) = read_to_terminal(&mut recv).await;
    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");

    // The contract, not the mechanism: two things enforce it here, the driver declaring the call
    // over and hyper tearing the stream down. Which one wins is the unit test's question.
    assert_eq!(
        send.send_message(Bytes::from_static(b"too late")).await,
        Err(CallError::Ended),
        "the abstract model guards a send on a call with no status yet"
    );
}

#[tokio::test]
async fn dropping_the_reading_half_ends_the_call() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, recv, _control) = channel
        .start_call(CallStartOptions::new(SLOW))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"x"))
        .await
        .expect("the message is accepted");

    drop(recv);

    // Nothing about the server, which is still asleep, is involved: a call nobody will read is
    // over, and the writing side says so.
    assert_eq!(
        send.send_message(Bytes::from_static(b"more")).await,
        Err(CallError::Ended)
    );
}

#[tokio::test]
async fn a_cancelled_call_ends_as_cancelled_without_waiting_for_the_server() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, mut recv, control) = channel
        .start_call(CallStartOptions::new(SLOW))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"x"))
        .await
        .expect("the message is accepted");
    send.end_send().await.expect("the request half-closes");

    control.cancel();
    // Idempotent: a second cancellation is not a second decision.
    control.cancel();

    let terminal = tokio::time::timeout(Duration::from_secs(5), recv.next_message())
        .await
        .expect("cancelling does not wait for the server's own answer")
        .expect("a terminal");

    match terminal {
        RecvResult::End(status) => assert_eq!(status.code, GrpcStatusCode::Cancelled),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        recv.next_message().await,
        Err(CallError::Ended),
        "nothing follows the terminal"
    );
}

#[tokio::test]
async fn closing_a_channel_refuses_new_calls_and_ends_the_ones_under_way() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    let (mut send, mut recv, _control) = channel
        .start_call(CallStartOptions::new(SLOW))
        .expect("the call starts")
        .split();
    send.send_message(Bytes::from_static(b"x"))
        .await
        .expect("the message is accepted");
    send.end_send().await.expect("the request half-closes");

    channel.close();

    assert_eq!(
        channel.start_call(CallStartOptions::new(ECHO)).err(),
        Some(ChannelError::Closed)
    );

    let terminal = tokio::time::timeout(Duration::from_secs(5), recv.next_message())
        .await
        .expect("closing ends the calls under way")
        .expect("a terminal");
    match terminal {
        RecvResult::End(status) => assert_eq!(status.code, GrpcStatusCode::Cancelled),
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------- what is not a gRPC answer

#[tokio::test]
async fn an_endpoint_nobody_answers_ends_the_call_rather_than_failing_to_start_it() {
    let endpoint = closed_port().await;
    let (_, messages, status) = unary(
        &channel(&endpoint),
        CallStartOptions::new(ECHO),
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Unavailable, "{status}");
    assert!(messages.is_empty());
}

#[tokio::test]
async fn connecting_up_front_reports_what_a_call_would_have_reported() {
    let server = TestServer::start().await;
    channel(&server.endpoint)
        .connect()
        .await
        .expect("the test server is listening");

    let error = channel(&closed_port().await)
        .connect()
        .await
        .expect_err("nothing is listening there");
    match error {
        ChannelError::Transport { source } => {
            assert_eq!(source.kind(), &TransportErrorKind::TcpConnect, "{source}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_closed_channel_opens_no_session() {
    let server = TestServer::start().await;
    let channel = channel(&server.endpoint);

    channel.close();

    assert_eq!(channel.connect().await, Err(ChannelError::Closed));
    // Nothing was dialled, so nothing is left holding a socket the channel will never release.
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn an_https_endpoint_is_refused_rather_than_dialled_in_the_clear() {
    let config = GrpcChannelConfig::new(TransportConfig::new(Uri::from_static(
        "https://127.0.0.1:443",
    )));

    let error = GrpcChannel::new(
        config,
        TokioExecutor::new(tokio::runtime::Handle::current()),
    )
    .expect_err("this connector speaks plain HTTP and says so");
    assert!(error.to_string().contains("https://"), "{error}");
}

#[tokio::test]
async fn a_send_window_of_nothing_is_refused() {
    let mut config =
        GrpcChannelConfig::new(TransportConfig::new(Uri::from_static("http://127.0.0.1:1")));
    config.max_sends_in_flight = 0;

    GrpcChannel::new(
        config,
        TokioExecutor::new(tokio::runtime::Handle::current()),
    )
    .expect_err("a window of zero would let a call send nothing");
}

#[tokio::test]
async fn an_http_error_page_is_reported_as_the_code_grpc_gives_it() {
    let server = TestServer::start().await;
    let (_, _, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new("/raw/NotFound"),
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Unimplemented, "{status}");
    assert!(status.message.contains("HTTP 404"), "{status}");
}

#[tokio::test]
async fn a_status_the_peer_states_stands_even_behind_an_http_error() {
    let server = TestServer::start().await;
    let (_, _, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new("/raw/StatusBehindError"),
        Bytes::from_static(b"x"),
    )
    .await;

    // A peer that answered in gRPC has said how the call ended; the HTTP status is not a better
    // account of it than its own.
    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");
    assert_eq!(status.message, "no room left");
}

#[tokio::test]
async fn a_two_hundred_that_is_not_grpc_is_an_internal_failure() {
    let server = TestServer::start().await;
    let (_, _, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new("/raw/NotGrpc"),
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Internal, "{status}");
    assert!(status.message.contains("content type"), "{status}");
}

#[tokio::test]
async fn a_compressed_message_ends_the_call_rather_than_being_read_as_bytes() {
    let server = TestServer::start().await;
    let (_, messages, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new("/raw/Compressed"),
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Internal, "{status}");
    assert!(status.message.contains("compressed"), "{status}");
    assert!(messages.is_empty());
}

#[tokio::test]
async fn a_status_behind_a_response_head_is_read_off_the_trailers() {
    let server = TestServer::start().await;
    let (head, messages, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new("/raw/HeadThenError"),
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::ResourceExhausted, "{status}");
    assert_eq!(status.message, "no room left");
    assert_eq!(messages, vec![Bytes::from_static(b"partial")]);
    assert_eq!(
        head.get("x-head"),
        Some(&MetadataValue::Ascii("present".to_owned()))
    );
}

#[tokio::test]
async fn a_stream_that_ends_without_a_status_is_an_internal_failure() {
    let server = TestServer::start().await;
    let (_, messages, status) = unary(
        &channel(&server.endpoint),
        CallStartOptions::new("/raw/NoTrailers"),
        Bytes::from_static(b"x"),
    )
    .await;

    assert_eq!(status.code, GrpcStatusCode::Internal, "{status}");
    assert!(status.message.contains("grpc-status"), "{status}");
    // What arrived before the stream stopped is still delivered.
    assert_eq!(messages, vec![Bytes::from_static(b"orphan")]);
}

// ---------------------------------------------------------------- the test server

/// What a handler does with a request, once `tonic` has decoded it.
type Answer = Pin<Box<dyn Future<Output = Result<Response<Bytes>, Status>> + Send>>;

/// One gRPC method, as the function that answers it.
#[derive(Clone, Copy)]
struct Handler(fn(Request<Bytes>) -> Answer);

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
fn echo(request: Request<Bytes>) -> Answer {
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
fn fail(_request: Request<Bytes>) -> Answer {
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
fn slow(_request: Request<Bytes>) -> Answer {
    Box::pin(async move {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        Ok(Response::new(Bytes::new()))
    })
}

/// The gRPC methods, plus the canned responses gRPC servers do not produce.
async fn answer(request: hyper::Request<Incoming>) -> hyper::Response<TonicBody> {
    use armonik_transport::reexports::tonic::server::Grpc;

    let path = request.uri().path().to_owned();
    if let Some(raw) = path.strip_prefix("/raw/") {
        return canned(raw, request.headers());
    }

    let request = request.map(TonicBody::new);
    match path.as_str() {
        ECHO => {
            Grpc::new(BytesCodec)
                .unary(&mut Handler(echo), request)
                .await
        }
        FAIL => {
            Grpc::new(BytesCodec)
                .unary(&mut Handler(fail), request)
                .await
        }
        SLOW => {
            Grpc::new(BytesCodec)
                .unary(&mut Handler(slow), request)
                .await
        }
        _ => Status::unimplemented("no such method").into_http(),
    }
}

/// A body that hands over frames already decided on.
struct Canned {
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

/// One gRPC message, framed by hand, so what these responses send owes nothing to this engine.
fn grpc_message(flag: u8, payload: &[u8]) -> Bytes {
    let mut framed = Vec::with_capacity(5 + payload.len());
    framed.push(flag);
    framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    framed.extend_from_slice(payload);
    Bytes::from(framed)
}

fn trailers(pairs: &[(&'static str, &'static str)]) -> Frame<Bytes> {
    let mut map = HeaderMap::new();
    for (key, value) in pairs {
        map.insert(*key, HeaderValue::from_static(value));
    }
    Frame::trailers(map)
}

fn canned(case: &str, request: &HeaderMap) -> hyper::Response<TonicBody> {
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

/// The head of an ordinary gRPC response.
fn grpc_head() -> hyper::http::response::Builder {
    hyper::Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/grpc")
}

/// The server the tests call, and the count of connections it has accepted.
struct TestServer {
    endpoint: String,
    connections: Arc<AtomicUsize>,
}

impl TestServer {
    /// Serves on an ephemeral loopback port, for as long as the test runs.
    async fn start() -> Self {
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

    fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }
}

/// An endpoint that was listening and is not any more.
async fn closed_port() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind to find a free port");
    let address = listener.local_addr().expect("the port that just closed");
    drop(listener);
    format!("http://{address}")
}
