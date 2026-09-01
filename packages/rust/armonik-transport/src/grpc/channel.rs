
use std::sync::Arc;

use http::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, TE, USER_AGENT};
use http::{Method, Request, Uri};
use hyper::client::conn::http2::SendRequest;
use tokio::sync::{watch, Mutex};
use tower_service::Service;

use crate::config::{ConfigError, IncompatibleOptionsSnafu};
use crate::http2::{TransportConfig, TransportConnector};

use super::call::{self, CallStartOptions, GrpcCall, RequestBody};
use super::error::ChannelError;
use super::executor::{Executor, HyperExecutor};

const DEFAULT_USER_AGENT: &str = concat!("armonik-transport/", env!("CARGO_PKG_VERSION"));

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
    pub fn new(config: GrpcChannelConfig, executor: impl Executor) -> Result<Self, ConfigError> {
        if config.max_sends_in_flight == 0 {
            return Err(IncompatibleOptionsSnafu {
                msg: "`max_sends_in_flight` is the number of buffers a call may have out at \
                      once, so zero would let it send nothing",
            }
            .build());
        }

        if config.max_recv_message_size == 0 {
            return Err(IncompatibleOptionsSnafu {
                msg: "`max_recv_message_size` of zero admits only empty messages, which is not                       what a caller writing zero for `no limit` is asking for",
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
                max_recv_message_size: config.max_recv_message_size,
                connection: Mutex::new(None),
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
        let mut headers = HeaderMap::new();
        options
            .metadata
            .reserve_in(&mut headers)
            .map_err(|source| ChannelError::InvalidMetadata { source })?;
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

    pub fn close(&self) {
        if self.inner.closed.send_replace(true) {
            return;
        }

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

pub(crate) struct Inner {
    endpoint: Uri,
    connector: TransportConnector,
    executor: Arc<dyn Executor>,
    user_agent: HeaderValue,
    max_sends_in_flight: usize,
    max_recv_message_size: usize,
    connection: Mutex<Option<SendRequest<RequestBody>>>,
    closed: watch::Sender<bool>,
}

impl Inner {
    pub(crate) fn max_recv_message_size(&self) -> usize {
        self.max_recv_message_size
    }

    pub(crate) async fn sender(&self) -> Result<SendRequest<RequestBody>, ChannelError> {
        let mut slot = self.connection.lock().await;
        if *self.closed.borrow() {
            return Err(ChannelError::Closed);
        }

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

        if *self.closed.borrow() {
            return Err(ChannelError::Closed);
        }

        let endpoint = self.endpoint.clone();
        self.executor.spawn(Box::pin(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%endpoint, %error, "the HTTP/2 session ended");
            }
        }));

        *slot = Some(sender.clone());
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
