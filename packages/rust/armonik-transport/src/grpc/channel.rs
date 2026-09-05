use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use http::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, TE, USER_AGENT};
use http::{Method, Request, Uri};
use hyper::client::conn::http2::SendRequest;
use tokio::sync::{watch, Mutex};

use super::error::GrpcChannelConfigError;
use crate::http2::{TransportConfig, TransportConnector};

use super::call::{self, CallStartOptions, GrpcCall, RequestBody};
use super::driver;
use super::error::ChannelError;
use super::executor::{Executor, HyperExecutor};

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
        executor: impl Executor,
    ) -> Result<Self, GrpcChannelConfigError> {
        if config.max_sends_in_flight == 0 {
            return Err(GrpcChannelConfigError::ZeroSendWindow);
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
                executor: Arc::new(executor),
                user_agent,
                max_sends_in_flight: config.max_sends_in_flight,
                max_recv_message_size: config.max_recv_message_size,
                connection: Mutex::new(Session::default()),
                attempts: AtomicU64::new(0),
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

        let uri = self.inner.request_uri(&options.method)?;
        let mut headers = engine_headers(&self.inner.user_agent);
        options
            .metadata
            .write_into(&mut headers)
            .map_err(|source| ChannelError::InvalidMetadata { source })?;

        let (grpc_call, body, driving) = call::create(
            self.inner.max_sends_in_flight,
            self.inner.closed.subscribe(),
        );

        let mut request = Request::new(body);
        *request.method_mut() = Method::POST;
        *request.uri_mut() = uri;
        *request.headers_mut() = headers;

        self.inner.executor.spawn(Box::pin(driver::drive(
            self.inner.clone(),
            request,
            driving,
        )));

        Ok(grpc_call)
    }

    pub fn close(&self) {
        if self.inner.closed.send_replace(true) {
            return;
        }

        let inner = self.inner.clone();
        self.inner.executor.spawn(Box::pin(async move {
            inner.connection.lock().await.sender.take();
        }));
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

fn engine_headers(user_agent: &HeaderValue) -> HeaderMap {
    let mut headers = HeaderMap::with_capacity(4);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/grpc"));
    headers.insert(TE, HeaderValue::from_static("trailers"));
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
    executor: Arc<dyn Executor>,
    user_agent: HeaderValue,
    max_sends_in_flight: usize,
    max_recv_message_size: usize,
    connection: Mutex<Session>,
    attempts: AtomicU64,
    closed: watch::Sender<bool>,
}

#[derive(Default)]
struct Session {
    sender: Option<SendRequest<RequestBody>>,
    failed: Option<ChannelError>,
}

impl Inner {
    pub(crate) fn max_recv_message_size(&self) -> usize {
        self.max_recv_message_size
    }

    pub(crate) async fn sender(&self) -> Result<SendRequest<RequestBody>, ChannelError> {
        // Read before the lock, compared after: whoever held it may have dialled and failed while
        // this caller waited, and the failure is theirs to report rather than a second dial's.
        let asked_at = self.attempts.load(Ordering::Acquire);

        let mut slot = self.connection.lock().await;
        if *self.closed.borrow() {
            return Err(ChannelError::Closed);
        }

        if let Some(sender) = slot.sender.as_ref() {
            if !sender.is_closed() {
                return Ok(sender.clone());
            }
        }

        if self.attempts.load(Ordering::Acquire) > asked_at {
            if let Some(failed) = slot.failed.clone() {
                return Err(failed);
            }
        }

        let dialled = crate::http2::open(
            &self.connector,
            &self.endpoint,
            HyperExecutor(self.executor.clone()),
        )
        .await;

        self.attempts.fetch_add(1, Ordering::AcqRel);
        let (sender, connection) = match dialled {
            Ok(session) => {
                slot.failed = None;
                session
            }
            Err(error) => {
                let error = ChannelError::from(error);
                slot.failed = Some(error.clone());
                return Err(error);
            }
        };

        if *self.closed.borrow() {
            return Err(ChannelError::Closed);
        }

        let endpoint = self.endpoint.clone();
        self.executor.spawn(Box::pin(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%endpoint, %error, "the HTTP/2 session ended");
            }
        }));

        slot.sender = Some(sender.clone());
        Ok(sender)
    }

    fn request_uri(&self, method: &str) -> Result<Uri, ChannelError> {
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
            (Some(""), Some(service), Some(name), None)
                if !service.is_empty() && !name.is_empty() => {}
            _ => return Err(invalid()),
        }

        let parts = self.endpoint.clone().into_parts();
        let mut builder = Uri::builder();
        if let Some(scheme) = parts.scheme {
            builder = builder.scheme(scheme);
        }
        if let Some(authority) = parts.authority {
            builder = builder.authority(authority);
        }
        builder
            .path_and_query(method)
            .build()
            .map_err(|_| invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner() -> Inner {
        Inner {
            endpoint: Uri::from_static("http://127.0.0.1:1234"),
            connector: TransportConnector::new(TransportConfig::new(Uri::from_static(
                "http://127.0.0.1:1234",
            )))
            .expect("a plain endpoint"),
            executor: Arc::new(super::super::executor::TokioExecutor::new(
                tokio::runtime::Handle::try_current().expect("a runtime"),
            )),
            user_agent: HeaderValue::from_static("test"),
            max_sends_in_flight: 1,
            max_recv_message_size: DEFAULT_MAX_RECV_MESSAGE_SIZE,
            connection: Mutex::new(Session::default()),
            attempts: AtomicU64::new(0),
            closed: watch::channel(false).0,
        }
    }

    #[test]
    fn every_header_this_engine_sets_is_one_a_caller_may_not_set() {
        let engine = engine_headers(&HeaderValue::from_static("test"));
        assert_eq!(engine.len(), 4, "the set grew or shrank: {engine:?}");

        for name in engine.keys() {
            let refused = super::super::Metadata::new().append_ascii(name.as_str(), "mine");
            assert!(
                refused.is_err(),
                "`{name}` is set by the channel and a caller may set it too, so both would travel"
            );
        }
    }

    #[tokio::test]
    async fn a_method_path_becomes_an_absolute_uri_on_the_endpoint() {
        let uri = inner()
            .request_uri("/armonik.api.grpc.v1.Sessions/CreateSession")
            .expect("a well-formed method path");

        assert_eq!(
            uri.to_string(),
            "http://127.0.0.1:1234/armonik.api.grpc.v1.Sessions/CreateSession"
        );
    }

    #[tokio::test]
    async fn anything_that_is_not_service_slash_method_is_refused() {
        let inner = inner();
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
                inner.request_uri(method).is_err(),
                "`{method}` should not be a method path"
            );
        }
    }
}
