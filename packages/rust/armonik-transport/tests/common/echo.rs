use std::convert::Infallible;
use std::future::Future;
use std::io::Write;
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
use armonik_transport::reexports::tonic::codec::CompressionEncoding;
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
/// Answers OK at once, its response whole, and goes on reading the request to its end.
pub const ANSWER_EARLY: &str = "/armonik_transport.test.Echo/AnswerEarly";
/// As [`ANSWER_EARLY`], with UNAVAILABLE in a Trailers-Only response.
pub const REFUSE_EARLY: &str = "/armonik_transport.test.Echo/RefuseEarly";
/// As [`ANSWER_EARLY`], with a message and then UNAVAILABLE in the trailers.
pub const FAIL_EARLY: &str = "/armonik_transport.test.Echo/FailEarly";
pub const COLLECT: &str = "/armonik_transport.test.Echo/Collect";
pub const FAN: &str = "/armonik_transport.test.Echo/Fan";
pub const CHAT: &str = "/armonik_transport.test.Echo/Chat";
/// Fails as many times as its `x-fail-times` says for its `x-flaky-key`, then echoes.
pub const FLAKY: &str = "/armonik_transport.test.Echo/Flaky";
/// A client stream that fails as [`FLAKY`] does once it has read `x-fail-after-messages`
/// messages, or all of them when unset, and past its failures collects as [`COLLECT`].
pub const FLAKY_COLLECT: &str = "/armonik_transport.test.Echo/FlakyCollect";
/// A bidi stream that fails as [`FLAKY_COLLECT`] does, answering the first message before it
/// fails when `x-answer-first` is set, and past its failures chats as [`CHAT`].
pub const FLAKY_CHAT: &str = "/armonik_transport.test.Echo/FlakyChat";
/// Echoes through tonic, which inflates a request compressed with gzip, deflate or zstd and
/// compresses its answer in the first of those the request's `grpc-accept-encoding` lists.
pub const ECHO_COMPRESSED: &str = "/armonik_transport.test.Echo/EchoCompressed";
/// Echoes through tonic, which accepts requests compressed with gzip and no other encoding, and
/// answers one in another UNIMPLEMENTED with `grpc-accept-encoding: gzip,identity`.
pub const ECHO_GZIP_ONLY: &str = "/armonik_transport.test.Echo/EchoGzipOnly";
/// Reads the request whole and answers one message that says how it arrived: its `grpc-encoding`,
/// and the flag and length on the wire of each message.
pub const FRAMES: &str = "/armonik_transport.test.Echo/Frames";

/// The `grpc-previous-rpc-attempts` each attempt of a key's flaky calls carried, in order.
static FLAKY_SEEN: std::sync::Mutex<std::collections::BTreeMap<String, Vec<Option<String>>>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// What [`FLAKY`] saw of `key`: one entry per attempt.
pub fn flaky_seen(key: &str) -> Vec<Option<String>> {
    FLAKY_SEEN
        .lock()
        .expect("the flaky record")
        .get(key)
        .cloned()
        .unwrap_or_default()
}

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
        .recv_head()
        .await
        .expect("a response head, even an empty one")
        .metadata
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

/// Answers each request message with the same bytes, as they arrive.
///
/// The reply stream is driven by the request stream, so a client that reads before it has sent
/// everything is reading answers to what it already sent - which is the whole point of the
/// cardinality, and what a fixture that answered up front would not exercise.
#[derive(Clone, Copy)]
pub struct Chatter;

impl armonik_transport::reexports::tonic::server::StreamingService<Bytes> for Chatter {
    type Response = Bytes;
    type ResponseStream = Pin<Box<dyn futures::Stream<Item = Result<Bytes, Status>> + Send>>;
    type Future =
        Pin<Box<dyn Future<Output = Result<Response<Self::ResponseStream>, Status>> + Send>>;

    fn call(&mut self, request: Request<Streaming<Bytes>>) -> Self::Future {
        Box::pin(async move {
            let asked = request.into_inner();
            let answered = futures::stream::unfold(asked, |mut asked| async move {
                match asked.message().await {
                    Ok(Some(message)) => Some((Ok(message), asked)),
                    Ok(None) => None,
                    Err(status) => Some((Err(status), asked)),
                }
            });
            Ok(Response::new(Box::pin(answered) as Self::ResponseStream))
        })
    }
}

/// Answers one message per comma-separated part of the request.
///
/// Through tonic like the collector, so the framing a reader has to take apart is the reference
/// implementation's and not this crate's own.
#[derive(Clone, Copy)]
pub struct Fanner;

impl armonik_transport::reexports::tonic::server::ServerStreamingService<Bytes> for Fanner {
    type Response = Bytes;
    type ResponseStream = Pin<Box<dyn futures::Stream<Item = Result<Bytes, Status>> + Send>>;
    type Future =
        Pin<Box<dyn Future<Output = Result<Response<Self::ResponseStream>, Status>> + Send>>;

    fn call(&mut self, request: Request<Bytes>) -> Self::Future {
        Box::pin(async move {
            let asked = String::from_utf8_lossy(request.get_ref()).into_owned();
            let parts: Vec<Bytes> = if asked.is_empty() {
                Vec::new()
            } else {
                asked
                    .split(',')
                    .map(|part| Bytes::from(part.to_owned()))
                    .collect()
            };
            let stream = futures::stream::iter(parts.into_iter().map(Ok));
            Ok(Response::new(Box::pin(stream) as Self::ResponseStream))
        })
    }
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

/// Reads `after` messages, or all of them, then fails UNAVAILABLE; with `answer_first`, a bidi
/// stream answers the first before it fails.
#[derive(Clone, Copy)]
pub struct Failing {
    after: Option<usize>,
    answer_first: bool,
}

impl Failing {
    async fn read(self, mut stream: Streaming<Bytes>) -> Result<Vec<Bytes>, Status> {
        let mut read = Vec::new();
        while self.after.is_none_or(|after| read.len() < after) {
            match stream.message().await? {
                Some(message) => read.push(message),
                None => break,
            }
        }
        Ok(read)
    }
}

impl armonik_transport::reexports::tonic::server::ClientStreamingService<Bytes> for Failing {
    type Response = Bytes;
    type Future = Answer;

    fn call(&mut self, request: Request<Streaming<Bytes>>) -> Self::Future {
        let this = *self;
        Box::pin(async move {
            this.read(request.into_inner()).await?;
            Err(Status::unavailable("flaky"))
        })
    }
}

impl armonik_transport::reexports::tonic::server::StreamingService<Bytes> for Failing {
    type Response = Bytes;
    type ResponseStream = Pin<Box<dyn futures::Stream<Item = Result<Bytes, Status>> + Send>>;
    type Future =
        Pin<Box<dyn Future<Output = Result<Response<Self::ResponseStream>, Status>> + Send>>;

    fn call(&mut self, request: Request<Streaming<Bytes>>) -> Self::Future {
        let this = *self;
        Box::pin(async move {
            let read = this.read(request.into_inner()).await?;
            let first = read.into_iter().next().filter(|_| this.answer_first);
            if first.is_none() {
                return Err(Status::unavailable("flaky"));
            }
            let answered = futures::stream::iter(
                first
                    .map(Ok)
                    .into_iter()
                    .chain([Err(Status::unavailable("flaky"))]),
            );
            Ok(Response::new(Box::pin(answered) as Self::ResponseStream))
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

    if path == ANSWER_EARLY || path == REFUSE_EARLY || path == FAIL_EARLY {
        let mut body = request.into_body();
        tokio::spawn(async move {
            while let Some(Ok(_)) =
                std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await
            {}
        });
        if path == FAIL_EARLY {
            return grpc_head()
                .body(TonicBody::new(Canned {
                    frames: vec![
                        Frame::data(grpc_message(0, b"before the failure")),
                        trailers(&[("grpc-status", "14")]),
                    ]
                    .into_iter(),
                    then_fails: false,
                    paced: false,
                    gave_way: false,
                }))
                .expect("a response");
        }
        let status = if path == REFUSE_EARLY { "14" } else { "0" };
        return grpc_head()
            .header("grpc-status", status)
            .body(TonicBody::empty())
            .expect("a response");
    }

    if path == ECHO_COMPRESSED {
        return Grpc::new(BytesCodec)
            .accept_compressed(CompressionEncoding::Gzip)
            .accept_compressed(CompressionEncoding::Deflate)
            .accept_compressed(CompressionEncoding::Zstd)
            .send_compressed(CompressionEncoding::Gzip)
            .send_compressed(CompressionEncoding::Deflate)
            .send_compressed(CompressionEncoding::Zstd)
            .max_decoding_message_size(usize::MAX)
            .max_encoding_message_size(usize::MAX)
            .unary(&mut Handler(echo), request.map(TonicBody::new))
            .await;
    }

    if path == ECHO_GZIP_ONLY {
        return Grpc::new(BytesCodec)
            .accept_compressed(CompressionEncoding::Gzip)
            .max_decoding_message_size(usize::MAX)
            .max_encoding_message_size(usize::MAX)
            .unary(&mut Handler(echo), request.map(TonicBody::new))
            .await;
    }

    if path == FRAMES {
        return frames(request).await;
    }

    if path == CHAT {
        return Grpc::new(BytesCodec)
            .max_decoding_message_size(usize::MAX)
            .max_encoding_message_size(usize::MAX)
            .streaming(Chatter, request.map(TonicBody::new))
            .await;
    }

    if path == FAN {
        return Grpc::new(BytesCodec)
            .max_decoding_message_size(usize::MAX)
            .max_encoding_message_size(usize::MAX)
            .server_streaming(Fanner, request.map(TonicBody::new))
            .await;
    }

    if path == COLLECT {
        return Grpc::new(BytesCodec)
            .max_decoding_message_size(usize::MAX)
            .max_encoding_message_size(usize::MAX)
            .client_streaming(Collector, request.map(TonicBody::new))
            .await;
    }

    if path == FLAKY {
        return flaky(request).await;
    }

    if path == FLAKY_COLLECT || path == FLAKY_CHAT {
        let headers = request.headers();
        let failing = flaky_fails(headers).then(|| Failing {
            after: header_of(headers, "x-fail-after-messages").and_then(|after| after.parse().ok()),
            answer_first: headers.contains_key("x-answer-first"),
        });
        let mut grpc = Grpc::new(BytesCodec)
            .max_decoding_message_size(usize::MAX)
            .max_encoding_message_size(usize::MAX);
        let request = request.map(TonicBody::new);
        return match (path == FLAKY_COLLECT, failing) {
            (true, Some(failing)) => grpc.client_streaming(failing, request).await,
            (true, None) => grpc.client_streaming(Collector, request).await,
            (false, Some(failing)) => grpc.streaming(failing, request).await,
            (false, None) => grpc.streaming(Chatter, request).await,
        };
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

async fn frames(request: hyper::Request<Incoming>) -> hyper::Response<TonicBody> {
    let encoding = header_of(request.headers(), "grpc-encoding").unwrap_or_else(|| "none".into());
    let mut body = request.into_body();
    let mut received = Vec::new();
    while let Some(Ok(frame)) = std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await
    {
        if let Some(data) = frame.data_ref() {
            received.extend_from_slice(data);
        }
    }

    let mut seen = Vec::new();
    let mut rest = &received[..];
    while rest.len() >= 5 {
        let len = u32::from_be_bytes(rest[1..5].try_into().expect("four bytes")) as usize;
        seen.push(format!("{}:{len}", rest[0]));
        rest = &rest[(5 + len).min(rest.len())..];
    }

    let said = format!("encoding={encoding} frames={}", seen.join(","));
    grpc_head()
        .body(TonicBody::new(Canned {
            frames: vec![
                Frame::data(grpc_message(0, said.as_bytes())),
                trailers(&[("grpc-status", "0")]),
            ]
            .into_iter(),
            then_fails: false,
            paced: false,
            gave_way: false,
        }))
        .expect("a response")
}

/// A message compressed in the encoding `name`, framed as such. The encoders are the libraries'
/// own, not the engine's, so a reply built here is what another implementation would send.
pub fn compressed_message(name: &str, payload: &[u8]) -> Bytes {
    let level = flate2::Compression::default();
    let squeezed = match name {
        "gzip" => {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), level);
            encoder.write_all(payload).expect("into a vector");
            encoder.finish().expect("a gzip stream")
        }
        "deflate" => {
            let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), level);
            encoder.write_all(payload).expect("into a vector");
            encoder.finish().expect("a zlib stream")
        }
        "zstd" => zstd::stream::encode_all(payload, 0).expect("a zstd frame"),
        other => panic!("no encoder for {other}"),
    };
    grpc_message(1, &squeezed)
}

/// Fails with `x-fail-code` (UNAVAILABLE by default) - in the head, or after one when
/// `x-fail-after-head` is set - with `x-pushback` as `grpc-retry-pushback-ms` when given, until
/// the key has failed `x-fail-times` times; then answers as [`ECHO`].
async fn flaky(request: hyper::Request<Incoming>) -> hyper::Response<TonicBody> {
    let header = |name: &str| header_of(request.headers(), name);
    let code = header("x-fail-code").unwrap_or_else(|| "14".to_owned());
    if !flaky_fails(request.headers()) {
        return armonik_transport::reexports::tonic::server::Grpc::new(BytesCodec)
            .accept_compressed(CompressionEncoding::Gzip)
            .accept_compressed(CompressionEncoding::Deflate)
            .accept_compressed(CompressionEncoding::Zstd)
            .unary(&mut Handler(echo), request.map(TonicBody::new))
            .await;
    }

    let mut status = HeaderMap::new();
    status.insert("grpc-status", HeaderValue::from_str(&code).expect("a code"));
    if let Some(pushback) = header("x-pushback") {
        status.insert(
            "grpc-retry-pushback-ms",
            HeaderValue::from_str(&pushback).expect("a pushback"),
        );
    }
    let (builder, frames) = if header("x-fail-after-head").is_some() {
        (grpc_head(), vec![Frame::trailers(status)])
    } else {
        let mut builder = grpc_head();
        for (name, value) in &status {
            builder = builder.header(name, value);
        }
        (builder, vec![])
    };
    builder
        .body(TonicBody::new(Canned {
            frames: frames.into_iter(),
            then_fails: false,
            paced: false,
            gave_way: false,
        }))
        .expect("a well-formed flaky response")
}

fn header_of(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// Records an attempt of the call's `x-flaky-key`, and whether it is one to fail.
fn flaky_fails(headers: &HeaderMap) -> bool {
    let key = header_of(headers, "x-flaky-key").expect("a flaky call names its key");
    let fails: usize = header_of(headers, "x-fail-times")
        .and_then(|times| times.parse().ok())
        .unwrap_or(0);
    let mut seen = FLAKY_SEEN.lock().expect("the flaky record");
    let attempts = seen.entry(key).or_default();
    attempts.push(header_of(headers, "grpc-previous-rpc-attempts"));
    attempts.len() <= fails
}

pub struct Canned {
    frames: std::vec::IntoIter<Frame<Bytes>>,
    /// Fails once the frames are out, rather than ending.
    ///
    /// It is how hyper's server is made to send a RST_STREAM: a body that errors resets the
    /// stream with INTERNAL_ERROR, where a body that ends finishes the response.
    then_fails: bool,
    /// Gives way once before each frame, so hyper writes what it holds first: the head and each
    /// frame then go out in writes of their own, as a server that flushes as it answers sends
    /// them.
    paced: bool,
    /// Whether the next frame has given way already.
    gave_way: bool,
}

impl Body for Canned {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if self.paced && !std::mem::replace(&mut self.gave_way, true) {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        self.gave_way = false;
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
            let seen: Vec<String> = [
                "content-type",
                "te",
                "grpc-accept-encoding",
                "grpc-encoding",
                "user-agent",
                "grpc-timeout",
                "content-length",
            ]
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
        // As many messages as `x-sizes` lists, each of that many bytes, whatever was sent, then
        // OK.
        "Sized" => {
            let sizes = header_of(request, "x-sizes").unwrap_or_default();
            let mut frames: Vec<Frame<Bytes>> = sizes
                .split(',')
                .filter_map(|size| size.trim().parse::<usize>().ok())
                .map(|size| Frame::data(grpc_message(0, &vec![b'z'; size])))
                .collect();
            frames.push(trailers(&[("grpc-status", "0")]));
            (grpc_head(), frames)
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
        // Not a gRPC response, whatever its headers say.
        "NotGrpcThatLists" => (
            hyper::Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/plain")
                .header("grpc-accept-encoding", "identity"),
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
        // Answers in the encodings of the compression document: a message left as it is under
        // the encoding the head names, one in an encoding nobody asked for, and, further down, a
        // message compressed, ones that cannot be inflated or inflate to more than a limit.
        "GzipReplyLeftPlain" => (
            grpc_head().header("grpc-encoding", "gzip"),
            vec![
                Frame::data(grpc_message(0, b"plain")),
                trailers(&[("grpc-status", "0")]),
            ],
        ),
        "BrotliReply" => (
            grpc_head().header("grpc-encoding", "br"),
            vec![
                Frame::data(grpc_message(1, b"squeezed")),
                trailers(&[("grpc-status", "0")]),
            ],
        ),
        "BrotliReplyLeftPlain" => (
            grpc_head().header("grpc-encoding", "br"),
            vec![
                Frame::data(grpc_message(0, b"plain")),
                trailers(&[("grpc-status", "0")]),
            ],
        ),
        "GzipTrailersOnlyUnavailable" => (
            grpc_head()
                .header("grpc-encoding", "gzip")
                .header("grpc-status", "14")
                .header("grpc-message", "try%20again"),
            vec![],
        ),
        // The same, in any of the encodings the engine knows: `ReplyIn:<name>` answers a message
        // compressed in it, `ReplyThatDoesNotInflate:<name>` bytes of no such stream, and
        // `ReplyOfEightMiB:<name>` eight MiB of zeros.
        case if case.starts_with("ReplyIn:") => {
            let name = &case["ReplyIn:".len()..];
            (
                grpc_head().header("grpc-encoding", name),
                vec![
                    Frame::data(compressed_message(name, &b"squeezed ".repeat(100))),
                    trailers(&[("grpc-status", "0")]),
                ],
            )
        }
        // An answer whose head states what the server accepts: `Accepts:<value>` is the
        // `grpc-accept-encoding` it carries.
        case if case.starts_with("Accepts:") => (
            grpc_head().header("grpc-accept-encoding", &case["Accepts:".len()..]),
            vec![
                Frame::data(grpc_message(0, b"ok")),
                trailers(&[("grpc-status", "0")]),
            ],
        ),
        case if case.starts_with("ReplyThatDoesNotInflate:") => {
            let name = &case["ReplyThatDoesNotInflate:".len()..];
            (
                grpc_head().header("grpc-encoding", name),
                vec![
                    Frame::data(grpc_message(1, b"not a stream of the encoding")),
                    trailers(&[("grpc-status", "0")]),
                ],
            )
        }
        case if case.starts_with("ReplyOfEightMiB:") => {
            let name = &case["ReplyOfEightMiB:".len()..];
            (
                grpc_head().header("grpc-encoding", name),
                vec![
                    Frame::data(compressed_message(name, &vec![0; 8 * 1024 * 1024])),
                    trailers(&[("grpc-status", "0")]),
                ],
            )
        }
        "HeadThenError" => (
            grpc_head().header("x-head", "present"),
            vec![
                Frame::data(grpc_message(0, b"partial")),
                trailers(&[("grpc-status", "8"), ("grpc-message", "no%20room%20left")]),
            ],
        ),
        "NoTrailers" => (grpc_head(), vec![Frame::data(grpc_message(0, b"orphan"))]),
        // A unary answer whose head, message and trailers go out in three writes.
        "Paced" => (
            grpc_head(),
            vec![
                Frame::data(grpc_message(0, b"paced")),
                trailers(&[("grpc-status", "0")]),
            ],
        ),
        // A message announced longer than what arrives before the trailers.
        "EndsMidMessage" => (
            grpc_head(),
            vec![
                Frame::data(announced_message(10, b"abc")),
                trailers(&[("grpc-status", "0")]),
            ],
        ),
        // Status details a peer mangled: not base64 at all.
        "BadStatusDetails" => (
            grpc_head(),
            vec![
                Frame::data(grpc_message(0, b"kept")),
                trailers(&[
                    ("grpc-status", "0"),
                    ("grpc-status-details-bin", "!not base64!"),
                ]),
            ],
        ),
        "TrailersOnlyBadStatusDetails" => (
            grpc_head()
                .header("grpc-status", "5")
                .header("grpc-message", "gone")
                .header("grpc-status-details-bin", "!not base64!"),
            vec![],
        ),
        // A length no 32-bit address can span.
        "Unaddressable" => (
            grpc_head(),
            vec![Frame::data(announced_message(u32::MAX, b"x"))],
        ),
        // Trailers-Only is one HEADERS frame carrying the status and nothing after it. This one
        // states the status and then sends a message, which is neither shape.
        "StatusInHeadThenMessage" => (
            grpc_head().header("grpc-status", "0"),
            vec![Frame::data(grpc_message(
                0,
                b"unread if the head is believed",
            ))],
        ),
        // And the shape it is mistaken for: the status in the head, and no body at all.
        "TrailersOnly" => (
            grpc_head()
                .header("grpc-status", "5")
                .header("grpc-message", "no%20such%20method"),
            vec![],
        ),
        "TrailersOnlyOk" => (
            grpc_head()
                .header("grpc-status", "0")
                .header("x-trailer", "present"),
            vec![],
        ),
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
            paced: case == "Paced",
            gave_way: false,
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
    open: Arc<AtomicUsize>,
}

impl TestServer {
    pub async fn start() -> Self {
        Self::serving(None, None).await
    }

    /// One whose SETTINGS let a connection open at most `streams` streams at once.
    pub async fn allowing(streams: u32) -> Self {
        Self::serving(Some(streams), None).await
    }

    /// One that closes a connection gracefully once it has taken `requests`, as nginx's
    /// `keepalive_requests` does: a GOAWAY, the streams it took answered to their end.
    pub async fn closing_after(requests: usize) -> Self {
        Self::serving(None, Some(requests)).await
    }

    /// A server listening at `endpoint`, such as the address `closed_port` returned.
    pub async fn start_at(endpoint: &str) -> Self {
        let address = endpoint
            .strip_prefix("http://")
            .expect("an http endpoint of an address");
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .expect("bind the address the endpoint names");
        Self::serving_on(listener, endpoint.to_owned(), None, None)
    }

    async fn serving(streams: Option<u32>, requests: Option<usize>) -> Self {
        let (listener, endpoint) = loopback().await;
        Self::serving_on(listener, endpoint, streams, requests)
    }

    fn serving_on(
        listener: tokio::net::TcpListener,
        endpoint: String,
        streams: Option<u32>,
        requests: Option<usize>,
    ) -> Self {
        let connections = Arc::new(AtomicUsize::new(0));

        let accepted = connections.clone();
        let open = Arc::new(AtomicUsize::new(0));
        let serving = open.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                accepted.fetch_add(1, Ordering::Relaxed);
                serving.fetch_add(1, Ordering::SeqCst);
                let serving = serving.clone();
                tokio::spawn(async move {
                    let taken = AtomicUsize::new(0);
                    let enough = tokio::sync::Notify::new();
                    let service = hyper::service::service_fn(|request| {
                        if Some(taken.fetch_add(1, Ordering::SeqCst) + 1) == requests {
                            enough.notify_one();
                        }
                        async { Ok::<_, Infallible>(answer(request).await) }
                    });
                    let mut builder = hyper::server::conn::http2::Builder::new(HyperTokio::new());
                    // Only when asked: none would lift hyper's own limit.
                    if let Some(streams) = streams {
                        builder.max_concurrent_streams(streams);
                    }
                    let mut connection =
                        std::pin::pin!(builder.serve_connection(TokioIo::new(stream), service));
                    tokio::select! {
                        _ = connection.as_mut() => {}
                        _ = enough.notified() => {
                            connection.as_mut().graceful_shutdown();
                            let _ = connection.await;
                        }
                    }
                    serving.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });

        Self {
            endpoint,
            connections,
            open,
        }
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }

    /// How many of its connections are still open.
    pub fn open(&self) -> usize {
        self.open.load(Ordering::SeqCst)
    }
}

pub async fn closed_port() -> String {
    let (listener, endpoint) = loopback().await;
    drop(listener);
    endpoint
}
