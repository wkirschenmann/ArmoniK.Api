use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::client::conn::http2::{Connection, SendRequest};
use hyper::rt::bounds::Http2ClientConnExec;
use hyper::Uri;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioIo;
use snafu::Snafu;
use tokio::net::TcpStream;
use tower_service::Service;

use crate::utils::chain;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TransportConfig {
    pub endpoint: Uri,
    pub connect_timeout: Duration,
}

impl TransportConfig {
    pub fn new(endpoint: Uri) -> Self {
        Self {
            endpoint,
            connect_timeout: Duration::from_secs(60),
        }
    }

    fn dialable(&self) -> Result<(), TransportError> {
        match self.endpoint.scheme_str() {
            Some("http") => {}
            Some(other) => {
                return ConfigurationSnafu {
                    message: format!(
                        "`{other}://` is not a scheme this connector dials; it speaks plain HTTP"
                    ),
                }
                .fail()
            }
            None => {
                return ConfigurationSnafu {
                    message: format!(
                        "the endpoint `{}` names no scheme; it has to be an `http://` URI",
                        self.endpoint
                    ),
                }
                .fail()
            }
        }

        if self.endpoint.host().is_none() {
            return ConfigurationSnafu {
                message: format!("the endpoint `{}` names no host", self.endpoint),
            }
            .fail();
        }

        Ok(())
    }
}

pub type TransportConnection = TokioIo<TcpStream>;

#[derive(Clone, Debug)]
pub struct TransportConnector {
    http: HttpConnector,
    connect_timeout: Duration,
}

impl TransportConnector {
    pub fn new(config: TransportConfig) -> Result<Self, TransportError> {
        config.dialable()?;

        let mut http = HttpConnector::new();
        http.set_nodelay(true);
        http.enforce_http(true);

        Ok(Self {
            http,
            connect_timeout: config.connect_timeout,
        })
    }
}

pub(crate) async fn open<E, B>(
    connector: &TransportConnector,
    endpoint: &Uri,
    executor: E,
) -> Result<(SendRequest<B>, Connection<TransportConnection, B, E>), TransportError>
where
    E: Http2ClientConnExec<B, TransportConnection> + Unpin + Clone,
    B: hyper::body::Body + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let deadline = connector.connect_timeout;
    let opening = async {
        let mut connector = connector.clone();
        std::future::poll_fn(|cx| connector.poll_ready(cx)).await?;
        let io = connector.call(endpoint.clone()).await?;
        hyper::client::conn::http2::Builder::new(executor)
            .handshake(io)
            .await
            .map_err(|error| {
                Http2HandshakeSnafu {
                    endpoint: endpoint.clone(),
                    cause: error.to_string(),
                }
                .build()
            })
    };

    match tokio::time::timeout(deadline, opening).await {
        Ok(result) => result,
        Err(_) => TimeoutSnafu {
            endpoint: endpoint.clone(),
            after: deadline,
        }
        .fail(),
    }
}

impl Service<Uri> for TransportConnector {
    type Response = TransportConnection;
    type Error = TransportError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, target: Uri) -> Self::Future {
        let dialling = self.http.call(target.clone());

        Box::pin(async move {
            dialling.await.map_err(|error| {
                ConnectSnafu {
                    endpoint: target.clone(),
                    cause: chain(&error, ": "),
                }
                .build()
            })
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TransportErrorKind {
    Connect,
    Http2Handshake,
    Timeout,
    Configuration,
}

#[derive(Clone, Debug, Eq, PartialEq, Snafu)]
#[snafu(visibility(pub(crate)))]
#[non_exhaustive]
pub enum TransportError {
    #[snafu(display("{message}"))]
    Configuration { message: String },
    #[snafu(display("`{endpoint}` could not be reached: {cause}"))]
    Connect { endpoint: Uri, cause: String },
    #[snafu(display("`{endpoint}` accepted a connection but not an HTTP/2 session: {cause}"))]
    Http2Handshake { endpoint: Uri, cause: String },
    #[snafu(display("connecting to `{endpoint}` outlasted {after:?}"))]
    Timeout { endpoint: Uri, after: Duration },
}

impl TransportError {
    pub fn kind(&self) -> TransportErrorKind {
        match self {
            Self::Configuration { .. } => TransportErrorKind::Configuration,
            Self::Connect { .. } => TransportErrorKind::Connect,
            Self::Http2Handshake { .. } => TransportErrorKind::Http2Handshake,
            Self::Timeout { .. } => TransportErrorKind::Timeout,
        }
    }
}
