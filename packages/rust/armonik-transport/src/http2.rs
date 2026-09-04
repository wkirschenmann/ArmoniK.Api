use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::client::conn::http2::{Connection, SendRequest};
use hyper::rt::bounds::Http2ClientConnExec;
use hyper::Uri;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tower_service::Service;

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
                return Err(TransportError::configuration(format!(
                    "`{other}://` is not a scheme this connector dials; it speaks plain HTTP"
                )))
            }
            None => {
                return Err(TransportError::configuration(format!(
                    "the endpoint `{}` names no scheme; it has to be an `http://` URI",
                    self.endpoint
                )))
            }
        }

        if self.endpoint.host().is_none() {
            return Err(TransportError::configuration(format!(
                "the endpoint `{}` names no host",
                self.endpoint
            )));
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
            .map_err(|error| TransportError::http2_handshake(endpoint, &error))
    };

    match tokio::time::timeout(deadline, opening).await {
        Ok(result) => result,
        Err(_) => Err(TransportError::timeout(endpoint, deadline)),
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
            dialling
                .await
                .map_err(|error| TransportError::connect(&target, &error))
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportError {
    kind: TransportErrorKind,
    message: String,
}

impl TransportError {
    pub fn kind(&self) -> TransportErrorKind {
        self.kind
    }

    fn configuration(message: String) -> Self {
        Self {
            kind: TransportErrorKind::Configuration,
            message,
        }
    }

    fn connect(endpoint: &Uri, error: &(dyn Error + 'static)) -> Self {
        Self {
            kind: TransportErrorKind::Connect,
            message: format!("`{endpoint}` could not be reached: {}", chain(error)),
        }
    }

    fn http2_handshake(endpoint: &Uri, error: &dyn std::error::Error) -> Self {
        Self {
            kind: TransportErrorKind::Http2Handshake,
            message: format!(
                "`{endpoint}` accepted a connection but not an HTTP/2 session: {error}"
            ),
        }
    }

    fn timeout(endpoint: &Uri, after: Duration) -> Self {
        Self {
            kind: TransportErrorKind::Timeout,
            message: format!("connecting to `{endpoint}` outlasted {after:?}"),
        }
    }
}

fn chain(error: &(dyn Error + 'static)) -> String {
    let mut rendered = error.to_string();
    let mut under = error.source();
    while let Some(cause) = under {
        rendered.push_str(": ");
        rendered.push_str(&cause.to_string());
        under = cause.source();
    }
    rendered
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TransportError {}
