use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use bytes::Bytes;
use http::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, USER_AGENT};
use http::uri::PathAndQuery;
use http::{StatusCode, Uri};
use http_body_util::BodyExt;
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper::client::conn::http2::SendRequest;
use tokio::sync::{broadcast, watch, Mutex};
use tonic::metadata::MetadataMap;
use tower_service::Service;

use super::error::GrpcChannelConfigError;
use crate::http2::{TransportConfig, TransportConnector};
use crate::options::LARGEST_WINDOW;

use super::call::{self, CallStartOptions, GrpcCall};
use super::driver::{self, Outgoing};
use super::error::ChannelError;
use super::executor::Spawner;
use super::status::{GrpcStatus, GrpcStatusCode};
use crate::utils::safe_endpoint;

const DEFAULT_USER_AGENT: &str = concat!("armonik-transport/", env!("CARGO_PKG_VERSION"));

/// Advertised as the only encoding because this engine decompresses nothing: a peer that reads
/// `grpc-accept-encoding` then sends what can be read rather than a body that cannot.
const ACCEPTED_ENCODING: &str = "identity";

const DEFAULT_MAX_RECV_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GrpcChannelConfig {
    pub transport: TransportConfig,
    pub user_agent: Option<String>,
    pub max_sends_in_flight: usize,
    pub max_recv_message_size: usize,
}

impl GrpcChannelConfig {
    pub fn new(transport: TransportConfig) -> Self {
        Self {
            transport,
            user_agent: None,
            max_sends_in_flight: 1,
            max_recv_message_size: DEFAULT_MAX_RECV_MESSAGE_SIZE,
        }
    }
}

#[derive(Clone)]
pub struct GrpcChannel {
    inner: Arc<Inner>,
}

impl GrpcChannel {
    pub fn new(
        config: GrpcChannelConfig,
        spawner: tokio::runtime::Handle,
    ) -> Result<Self, GrpcChannelConfigError> {
        if config.max_sends_in_flight == 0 {
            return Err(GrpcChannelConfigError::ZeroSendWindow);
        }

        // The schema's bound, checked at a door that takes a number rather than a document: the
        // window sizes a channel whose semaphore panics above its own limit instead of refusing,
        // and the first call is where that would land.
        if config.max_sends_in_flight > LARGEST_WINDOW as usize {
            return Err(GrpcChannelConfigError::SendWindowTooLarge {
                value: config.max_sends_in_flight,
            });
        }

        if config.max_recv_message_size == 0 {
            return Err(GrpcChannelConfigError::ZeroMaxRecvMessageSize);
        }

        let user_agent = match &config.user_agent {
            None => HeaderValue::from_static(DEFAULT_USER_AGENT),
            Some(text) => HeaderValue::from_str(text).map_err(|_| {
                GrpcChannelConfigError::InvalidUserAgent {
                    value: text.clone(),
                }
            })?,
        };

        let endpoint = config.transport.endpoint.clone();
        let connector = TransportConnector::new(config.transport)?;

        Ok(Self {
            inner: Arc::new(Inner {
                endpoint,
                connector,
                spawner,
                user_agent,
                max_sends_in_flight: config.max_sends_in_flight,
                max_recv_message_size: config.max_recv_message_size,
                connection: Mutex::new(Session::default()),
                closed: watch::channel(false).0,
            }),
        })
    }

    pub async fn connect(&self) -> Result<(), ChannelError> {
        self.inner.sender().await.map(|_| ())
    }

    pub fn start_call(&self, options: CallStartOptions) -> Result<GrpcCall, ChannelError> {
        if *self.inner.closed.borrow() {
            return Err(ChannelError::Closed);
        }

        let path = method_path(&options.method)?;
        let mut metadata = HeaderMap::new();
        options
            .metadata
            .write_into(&mut metadata)
            .map_err(|source| ChannelError::InvalidMetadata { source })?;

        let (grpc_call, messages, driving) = call::create(
            self.inner.max_sends_in_flight,
            self.inner.closed.subscribe(),
        );

        let outgoing = Outgoing {
            path,
            metadata,
            messages,
        };
        self.inner
            .spawner
            .spawn(driver::drive(self.inner.clone(), outgoing, driving));

        Ok(grpc_call)
    }

    pub fn close(&self) {
        if self.inner.closed.send_replace(true) {
            return;
        }

        let inner = self.inner.clone();
        self.inner.spawner.spawn(async move {
            inner.connection.lock().await.sender.take();
        });
    }
}

impl std::fmt::Debug for GrpcChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrpcChannel")
            .field("endpoint", &self.inner.endpoint)
            .field("closed", &*self.inner.closed.borrow())
            .finish()
    }
}

/// What the engine adds to every request, beside the `te` and `content-type` tonic's client writes.
fn engine_headers(user_agent: &HeaderValue) -> HeaderMap {
    let mut headers = HeaderMap::with_capacity(2);
    headers.insert(USER_AGENT, user_agent.clone());
    headers.insert(
        HeaderName::from_static("grpc-accept-encoding"),
        HeaderValue::from_static(ACCEPTED_ENCODING),
    );
    headers
}

pub(crate) struct Inner {
    endpoint: Uri,
    connector: TransportConnector,
    spawner: tokio::runtime::Handle,
    user_agent: HeaderValue,
    max_sends_in_flight: usize,
    max_recv_message_size: usize,
    connection: Mutex<Session>,
    closed: watch::Sender<bool>,
}

#[derive(Default)]
struct Session {
    sender: Option<SendRequest<tonic::body::Body>>,
    /// The dial in flight, if one is, and how its outcome reaches whoever waits for it.
    ///
    /// A dial opens the channel's connection, so it belongs to the channel and not to whichever
    /// call reached it first. Run inside that call's future it would be the call's: cancelling
    /// the call - a deadline, `ak_call_cancel`, its channel closing - would drop the future and
    /// the dial with it, and the calls queued behind the lock would start again from nothing.
    /// Under a stream of calls whose deadline is shorter than a dial, none of them would ever
    /// complete one, though a single call left alone would.
    dialling: Option<broadcast::Sender<Result<SendRequest<tonic::body::Body>, ChannelError>>>,
}

impl Inner {
    /// A tonic client over this channel's session. One per call, and cheap: it holds the
    /// session's handle and its configuration, not a connection of its own.
    pub(crate) fn client(self: &Arc<Self>) -> tonic::client::Grpc<Http2> {
        tonic::client::Grpc::with_origin(Http2(Arc::clone(self)), self.endpoint.clone())
            .max_decoding_message_size(addressable(self.max_recv_message_size))
    }

    /// The channel's connection, dialling it if there is none.
    ///
    /// A caller either takes the cached session, joins the dial already in flight, or starts one -
    /// and starting one means spawning it, not running it here. Going away then detaches this
    /// caller from the dial instead of cancelling it for everyone waiting.
    async fn sender(self: &Arc<Self>) -> Result<SendRequest<tonic::body::Body>, ChannelError> {
        let mut waiting = {
            let mut slot = self.connection.lock().await;
            if *self.closed.borrow() {
                return Err(ChannelError::Closed);
            }

            if let Some(sender) = slot.sender.as_ref() {
                if !sender.is_closed() {
                    return Ok(sender.clone());
                }
            }

            match slot.dialling.as_ref() {
                Some(dialling) => dialling.subscribe(),
                None => {
                    // One, because one outcome is sent and every waiter subscribed before it was.
                    let (outcome, waiting) = broadcast::channel(1);
                    slot.dialling = Some(outcome);
                    let inner = Arc::clone(self);
                    self.spawner.spawn(async move { inner.dial().await });
                    waiting
                }
            }
        };

        // The lock is released, so the dial is free to take it when it is done. A caller dropped
        // here drops only its receiver.
        match waiting.recv().await {
            Ok(outcome) => outcome,
            // The dial task went away without an outcome, which happens when the runtime it was
            // spawned on is shutting down.
            Err(_) => Err(ChannelError::Closed),
        }
    }

    /// Opens the connection and tells whoever waited.
    ///
    /// Its own task, so no caller owns it. The order at the end matters: the slot is updated and
    /// the dial cleared before the outcome goes out, so a caller that arrives after the send finds
    /// the session rather than a dial that is no longer running.
    async fn dial(&self) {
        let dialled = crate::http2::open(
            &self.connector,
            &self.endpoint,
            Spawner(self.spawner.clone()),
        )
        .await;

        let mut slot = self.connection.lock().await;
        let outcome = slot.dialling.take();

        let outcome = match outcome {
            Some(outcome) => outcome,
            None => return,
        };

        let told = |result| {
            // Every waiter may have gone; the slot above is what the next caller reads.
            let _ = outcome.send(result);
        };

        let (sender, connection) = match dialled {
            Ok(session) => session,
            Err(error) => return told(Err(ChannelError::from(error))),
        };

        if *self.closed.borrow() {
            return told(Err(ChannelError::Closed));
        }

        let endpoint = safe_endpoint(&self.endpoint);
        self.spawner.spawn(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%endpoint, %error, "the HTTP/2 session ended");
            }
        });

        slot.sender = Some(sender.clone());
        told(Ok(sender));
    }
}

/// The channel's HTTP/2 session, in the shape tonic's client sends a call through.
///
/// Ready at once: the session is dialled or joined inside `call`, where a call that goes away
/// detaches from the dial instead of cancelling it for every call waiting on it.
#[derive(Clone)]
pub(crate) struct Http2(Arc<Inner>);

/// Removed from every head and every trailer before tonic reads them. tonic decodes it with an
/// `expect` wherever it reads a status, so a peer that sent it malformed would panic the call's
/// task - and nothing on this side reads it: a `grpc-` key never reaches a caller's metadata.
const GRPC_STATUS_DETAILS: &str = "grpc-status-details-bin";

/// The limit tonic's decoder is given: the configured one, within what this target can address.
///
/// tonic reserves an announced length before reading it, and a reserve past what an address can
/// span panics instead of refusing. Held under half the address space, such a message meets the
/// limit first and is refused as RESOURCE_EXHAUSTED. Only a 32-bit target is ever near it: a
/// four-byte length does not come close on 64 bits.
fn addressable(limit: usize) -> usize {
    limit.min(isize::MAX as usize / 2)
}

impl Service<http::Request<tonic::body::Body>> for Http2 {
    type Response = http::Response<ResponseBody>;
    type Error = tonic::Status;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: http::Request<tonic::body::Body>) -> Self::Future {
        let inner = Arc::clone(&self.0);
        Box::pin(async move {
            request
                .headers_mut()
                .extend(engine_headers(&inner.user_agent));

            let mut sender = inner.sender().await.map_err(|error| match error {
                ChannelError::Closed => worded(GrpcStatus::cancelled()),
                error => worded(GrpcStatus::unreachable(error)),
            })?;
            let mut response = sender
                .send_request(request)
                .await
                .map_err(|error| worded(GrpcStatus::request_lost(&error)))?;
            response.headers_mut().remove(GRPC_STATUS_DETAILS);
            refuse_what_is_not_grpc(&response)?;
            let response = refuse_a_message_behind_a_stated_status(response).await?;

            Ok(response.map(ResponseBody::new))
        })
    }
}

/// A status this engine words, in the type tonic's client carries it in.
///
/// tonic hands back a `Status` it is given, which is what keeps these codes: its own reading of a
/// transport error answers UNKNOWN for a reset, its reset table being behind its `server`
/// feature.
fn worded(status: GrpcStatus) -> tonic::Status {
    tonic::Status::new(status.code, status.message)
}

fn broke(error: hyper::Error) -> tonic::Status {
    worded(GrpcStatus::stream_broke(&error))
}

/// The response body as tonic's decoder reads it, with the one check tonic does not make.
///
/// tonic takes trailers that arrive in the middle of a message for the end of the call and reports
/// their status, so a message the peer cut short is dropped under an OK. This follows the length
/// prefixes - each message's five-byte header, and how much of its body is still owed - without
/// holding any of the bytes, and answers such trailers with INTERNAL.
pub(crate) struct ResponseBody {
    inner: Incoming,
    framing: Framing,
}

impl ResponseBody {
    fn new(inner: Incoming) -> Self {
        Self {
            inner,
            framing: Framing::default(),
        }
    }
}

/// Where a stream of length-prefixed messages stands, from the bytes that went by.
#[derive(Default)]
struct Framing {
    header: [u8; 5],
    header_read: usize,
    owed: usize,
}

impl Framing {
    fn follow(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            if self.owed > 0 {
                let taken = self.owed.min(data.len());
                self.owed -= taken;
                data = &data[taken..];
                continue;
            }

            let taken = (self.header.len() - self.header_read).min(data.len());
            self.header[self.header_read..self.header_read + taken].copy_from_slice(&data[..taken]);
            self.header_read += taken;
            data = &data[taken..];
            if self.header_read == self.header.len() {
                let [_, length @ ..] = self.header;
                self.owed = u32::from_be_bytes(length) as usize;
                self.header_read = 0;
            }
        }
    }

    fn between_messages(&self) -> bool {
        self.header_read == 0 && self.owed == 0
    }
}

impl Body for ResponseBody {
    type Data = Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        let frame = match ready!(Pin::new(&mut this.inner).poll_frame(cx)) {
            None => return Poll::Ready(None),
            Some(Err(error)) => return Poll::Ready(Some(Err(broke(error)))),
            Some(Ok(frame)) => frame,
        };

        let mut trailers = match frame.into_trailers() {
            Ok(trailers) => trailers,
            Err(frame) => {
                if let Some(data) = frame.data_ref() {
                    this.framing.follow(data);
                }
                return Poll::Ready(Some(Ok(frame)));
            }
        };

        if !this.framing.between_messages() {
            return Poll::Ready(Some(Err(tonic::Status::internal(
                "the peer's trailers arrived in the middle of a message",
            ))));
        }
        trailers.remove(GRPC_STATUS_DETAILS);
        Poll::Ready(Some(Ok(Frame::trailers(trailers))))
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// An HTTP answer that is not gRPC, refused before tonic's client reads its body as messages.
///
/// A proxy's error page is the usual one, and tonic would take the first byte of its HTML for a
/// message flag. The gRPC table maps the HTTP status instead, and a status the peer states stands
/// even behind an HTTP error.
fn refuse_what_is_not_grpc(response: &http::Response<Incoming>) -> Result<(), tonic::Status> {
    let headers = response.headers();
    let status = response.status();
    if status != StatusCode::OK {
        return Err(tonic::Status::from_header_map(headers).unwrap_or_else(|| {
            tonic::Status::with_metadata(
                http_code(status),
                format!(
                    "the peer answered HTTP {} rather than gRPC",
                    status.as_u16()
                ),
                MetadataMap::from_headers(headers.clone()),
            )
        }));
    }
    if !speaks_grpc(headers) {
        return Err(tonic::Status::internal(
            "the peer answered HTTP 200 without a gRPC content type",
        ));
    }
    Ok(())
}

/// A status in the head, held to the Trailers-Only shape: nothing may follow it.
///
/// tonic takes any status in the head for Trailers-Only and ends the call on it without reading
/// the body, so a message behind it would be dropped under whatever that status said. The body is
/// read to its end here first. An empty DATA frame is still nothing: hyper ends a body it did not
/// end on the head with one, which is how a Trailers-Only answer from a hyper server arrives.
/// UNKNOWN, because the call's status has not been said where the protocol puts it.
async fn refuse_a_message_behind_a_stated_status(
    mut response: http::Response<Incoming>,
) -> Result<http::Response<Incoming>, tonic::Status> {
    if !response.headers().contains_key("grpc-status") {
        return Ok(response);
    }
    while let Some(frame) = response.body_mut().frame().await {
        let frame = frame.map_err(broke)?;
        if frame.data_ref().is_some_and(|data| !data.is_empty()) {
            return Err(tonic::Status::unknown(
                "the peer stated a grpc-status in the response head and then sent a message",
            ));
        }
    }
    Ok(response)
}

/// PROTOCOL-HTTP2's table from an HTTP status to the gRPC code a client reports for it.
fn http_code(status: StatusCode) -> GrpcStatusCode {
    match status.as_u16() {
        400 => GrpcStatusCode::Internal,
        401 => GrpcStatusCode::Unauthenticated,
        403 => GrpcStatusCode::PermissionDenied,
        404 => GrpcStatusCode::Unimplemented,
        429 | 502 | 503 | 504 => GrpcStatusCode::Unavailable,
        _ => GrpcStatusCode::Unknown,
    }
}

fn speaks_grpc(headers: &HeaderMap) -> bool {
    const GRPC: &str = "application/grpc";

    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            let value = value.trim();
            value.len() >= GRPC.len()
                && value[..GRPC.len()].eq_ignore_ascii_case(GRPC)
                && value[GRPC.len()..]
                    .chars()
                    .next()
                    .is_none_or(|next| next == '+' || next == ';')
        })
        .unwrap_or(false)
}

fn method_path(method: &str) -> Result<PathAndQuery, ChannelError> {
    let invalid = || ChannelError::InvalidMethod {
        method: method.to_owned(),
    };

    if method.contains(['?', '#']) {
        return Err(invalid());
    }
    let mut segments = method.split('/');
    match (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) {
        (Some(""), Some(service), Some(name), None) if !service.is_empty() && !name.is_empty() => {}
        _ => return Err(invalid()),
    }

    method.parse().map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both edges of the window, at the door that takes a number.
    ///
    /// The FFI refuses this range out of a document; nothing refused it here, and a window past
    /// the semaphore's limit panics at the call that sizes its channel rather than at the
    /// configuration that named it.
    #[tokio::test]
    async fn a_send_window_outside_what_the_options_admit_is_refused() {
        let refused = |max_sends_in_flight| {
            let mut config = GrpcChannelConfig::new(TransportConfig::new(Uri::from_static(
                "http://127.0.0.1:1234",
            )));
            config.max_sends_in_flight = max_sends_in_flight;
            GrpcChannel::new(config, tokio::runtime::Handle::current()).err()
        };

        assert!(matches!(
            refused(0),
            Some(GrpcChannelConfigError::ZeroSendWindow)
        ));
        assert!(matches!(
            refused(LARGEST_WINDOW as usize + 1),
            Some(GrpcChannelConfigError::SendWindowTooLarge { .. })
        ));
        assert!(
            refused(LARGEST_WINDOW as usize).is_none(),
            "the deepest window the options admit is one this door takes"
        );
    }

    #[test]
    fn every_header_on_the_wire_that_the_caller_did_not_write_is_one_it_may_not_set() {
        use http::header::{CONTENT_TYPE, TE};

        let engine = engine_headers(&HeaderValue::from_static("test"));
        assert_eq!(engine.len(), 2, "the set grew or shrank: {engine:?}");

        for name in engine.keys().chain([&TE, &CONTENT_TYPE]) {
            let refused = super::super::Metadata::new().append_ascii(name.as_str(), "mine");
            assert!(
                refused.is_err(),
                "`{name}` is set by the channel and a caller may set it too, so both would travel"
            );
        }
    }

    #[test]
    fn the_content_type_has_to_say_grpc() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, HeaderValue::from_str(value).expect("valid"));
            headers
        };

        for value in [
            "application/grpc",
            "application/grpc+proto",
            "application/grpc; charset=utf-8",
            "Application/gRPC",
        ] {
            assert!(speaks_grpc(&with(value)), "{value}");
        }

        assert!(!speaks_grpc(&with("text/html")));
        assert!(!speaks_grpc(&with("application/grpcweb")));
        assert!(!speaks_grpc(&HeaderMap::new()));
    }

    #[test]
    fn framing_is_followed_across_the_chunks_a_message_arrives_in() {
        let mut framing = Framing::default();
        assert!(framing.between_messages(), "before anything");

        // A header split over two chunks, then a body of three.
        framing.follow(&[0, 0]);
        assert!(!framing.between_messages(), "inside a header");
        framing.follow(&[0, 0, 3, b'a']);
        assert!(!framing.between_messages(), "two bytes owed");
        framing.follow(b"bc");
        assert!(framing.between_messages(), "the message is whole");

        // An empty message, then a whole one, in a single chunk.
        framing.follow(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 1, b'x']);
        assert!(framing.between_messages(), "both are whole");

        framing.follow(&[0, 0, 0, 0, 4, b'y']);
        assert!(!framing.between_messages(), "three bytes owed");
    }

    #[test]
    fn a_limit_is_kept_within_what_the_target_can_address() {
        assert_eq!(addressable(4 * 1024 * 1024), 4 * 1024 * 1024);
        assert_eq!(addressable(usize::MAX), isize::MAX as usize / 2);
    }

    #[test]
    fn an_http_failure_maps_to_the_code_grpc_gives_it() {
        assert_eq!(
            http_code(StatusCode::NOT_FOUND),
            GrpcStatusCode::Unimplemented
        );
        assert_eq!(
            http_code(StatusCode::SERVICE_UNAVAILABLE),
            GrpcStatusCode::Unavailable
        );
        assert_eq!(http_code(StatusCode::IM_A_TEAPOT), GrpcStatusCode::Unknown);
    }

    #[test]
    fn anything_that_is_not_service_slash_method_is_refused() {
        assert!(method_path("/armonik.api.grpc.v1.Sessions/CreateSession").is_ok());
        for method in [
            "",
            "/",
            "Service/Method",
            "/Service",
            "/Service/",
            "//Method",
            "/Service/Method/Extra",
            "/Service/Method?query",
        ] {
            assert!(
                method_path(method).is_err(),
                "`{method}` should not be a method path"
            );
        }
    }
}
