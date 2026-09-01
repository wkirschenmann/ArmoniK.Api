//! The channel: one HTTP/2 session, and the calls started on it.

use std::sync::Arc;

use http::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, TE, USER_AGENT};
use http::{Method, Request, Uri};
use hyper::client::conn::http2::SendRequest;
use tokio::sync::{watch, Mutex};
use tower_service::Service;

use crate::config::{ConfigError, IncompatibleOptionsSnafu};
use crate::http2::{TransportConfig, TransportConnector, TransportError};

use super::call::{self, CallStartOptions, GrpcCall, RequestBody};
use super::error::ChannelError;
use super::executor::{Executor, HyperExecutor};

/// What this engine says it is, when the configuration says nothing.
const DEFAULT_USER_AGENT: &str = concat!("armonik-transport/", env!("CARGO_PKG_VERSION"));

/// The only message encoding this engine reads, and the only one it asks for.
const ACCEPTED_ENCODING: &str = "identity";

/// How a channel is configured.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GrpcChannelConfig {
    /// How to reach the endpoint.
    pub transport: TransportConfig,
    /// What to send as `user-agent`.
    pub user_agent: Option<String>,
    /// How many buffers a call may have out at once before a send has to wait.
    pub max_sends_in_flight: usize,
}

impl GrpcChannelConfig {
    /// Defaults around a way to reach the endpoint.
    pub fn new(transport: TransportConfig) -> Self {
        Self {
            transport,
            user_agent: None,
            max_sends_in_flight: 1,
        }
    }
}

/// A gRPC channel: calls, over one HTTP/2 session to one endpoint.
///
/// Cloning shares the session rather than opening a second one.
#[derive(Clone)]
pub struct GrpcChannel {
    inner: Arc<Inner>,
}

impl GrpcChannel {
    /// Builds the channel. Performs no I/O, so a failure here is a configuration that could
    /// never have worked rather than an endpoint that happened to be down.
    pub fn new(config: GrpcChannelConfig, executor: impl Executor) -> Result<Self, ConfigError> {
        if config.max_sends_in_flight == 0 {
            return Err(IncompatibleOptionsSnafu {
                msg: "`max_sends_in_flight` is the number of buffers a call may have out at \
                      once, so zero would let it send nothing",
            }
            .build());
        }

        let user_agent = match &config.user_agent {
            None => HeaderValue::from_static(DEFAULT_USER_AGENT),
            Some(text) => HeaderValue::from_str(text).map_err(|_| {
                IncompatibleOptionsSnafu {
                    msg: format!("`{text}` is not a value a `user-agent` header can carry"),
                }
                .build()
            })?,
        };

        let endpoint = config.transport.endpoint.clone();
        let connector = TransportConnector::new(config.transport).map_err(|error| {
            IncompatibleOptionsSnafu {
                msg: error.to_string(),
            }
            .build()
        })?;

        Ok(Self {
            inner: Arc::new(Inner {
                endpoint,
                connector,
                executor: Arc::new(executor),
                user_agent,
                max_sends_in_flight: config.max_sends_in_flight,
                connection: Mutex::new(None),
                closed: watch::channel(false).0,
            }),
        })
    }

    /// Opens the session now, and says how it went.
    ///
    /// Optional: a call opens it otherwise, and reports a failure to open it as its own terminal
    /// status. This is for a caller that wants to know before it has a call to lose.
    pub async fn connect(&self) -> Result<(), TransportError> {
        self.inner.sender().await.map(|_| ())
    }

    /// Starts a call.
    ///
    /// Returns as soon as the call exists, which is before it has reached the network: the
    /// request travels on the task this spawns.
    pub fn start_call(&self, options: CallStartOptions) -> Result<GrpcCall, ChannelError> {
        if *self.inner.closed.borrow() {
            return Err(ChannelError::Closed);
        }

        let uri = self.inner.request_uri(&options.method)?;
        let mut headers = HeaderMap::with_capacity(4 + options.metadata.len());
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/grpc"));
        headers.insert(TE, HeaderValue::from_static("trailers"));
        headers.insert(USER_AGENT, self.inner.user_agent.clone());
        headers.insert(
            HeaderName::from_static("grpc-accept-encoding"),
            HeaderValue::from_static(ACCEPTED_ENCODING),
        );
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

        self.inner
            .executor
            .spawn(Box::pin(call::drive(self.inner.clone(), request, driving)));

        Ok(grpc_call)
    }

    /// Refuses new calls, cancels the ones under way, and lets the session go.
    pub fn close(&self) {
        if self.inner.closed.send_replace(true) {
            return;
        }

        // Releasing the session means taking the lock the calls dial under, which this method
        // has no way to await; the executor that runs the calls runs this too.
        let inner = self.inner.clone();
        self.inner.executor.spawn(Box::pin(async move {
            inner.connection.lock().await.take();
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

/// What a channel and the tasks it spawned share.
pub(crate) struct Inner {
    endpoint: Uri,
    connector: TransportConnector,
    executor: Arc<dyn Executor>,
    user_agent: HeaderValue,
    max_sends_in_flight: usize,
    connection: Mutex<Option<SendRequest<RequestBody>>>,
    closed: watch::Sender<bool>,
}

impl Inner {
    /// The session, opening one if there is none or the last one is gone.
    pub(crate) async fn sender(&self) -> Result<SendRequest<RequestBody>, TransportError> {
        let mut slot = self.connection.lock().await;

        if let Some(sender) = slot.as_ref() {
            if !sender.is_closed() {
                return Ok(sender.clone());
            }
        }

        let mut connector = self.connector.clone();
        std::future::poll_fn(|cx| connector.poll_ready(cx)).await?;
        let io = connector.call(self.endpoint.clone()).await?;

        let (sender, connection) =
            crate::http2::handshake(&self.endpoint, HyperExecutor(self.executor.clone()), io)
                .await?;

        let endpoint = self.endpoint.clone();
        self.executor.spawn(Box::pin(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%endpoint, %error, "the HTTP/2 session ended");
            }
        }));

        *slot = Some(sender.clone());
        Ok(sender)
    }

    /// The absolute URI a request for `method` is addressed to.
    fn request_uri(&self, method: &str) -> Result<Uri, ChannelError> {
        let invalid = || ChannelError::InvalidMethod {
            method: method.to_owned(),
        };

        // `/Service/Method` and nothing else: a query or a third segment is not something a gRPC
        // server has anywhere to read.
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
            connection: Mutex::new(None),
            closed: watch::channel(false).0,
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
