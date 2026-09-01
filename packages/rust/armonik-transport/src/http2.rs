//! Layer 1: the network, up to an HTTP/2 session.
//!
//! [`TransportConnector`] is a [`tower_service::Service<Uri>`] yielding connected streams; it
//! knows neither HTTP/2 nor gRPC and is a network dial and nothing more. [`handshake`] turns one
//! of those streams into a session. What travels on the session is [`crate::grpc`]'s business.
//!
//! This connector dials plain TCP. Transport security is [`crate::connect`], which hands out a
//! `tonic` channel instead of a stream.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::client::conn::http2::{Connection, SendRequest};
use hyper::rt::bounds::Http2ClientConnExec;
use hyper::Uri;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tower_service::Service;

#[derive(Clone, Debug)]
#[non_exhaustive]
/// What it takes to reach an endpoint.
pub struct TransportConfig {
    /// The endpoint to dial. Only the `http` scheme is accepted.
    pub endpoint: Uri,
    /// Bounds the whole sequence - resolution and connection - not each step.
    pub connect_timeout: Duration,
}

impl TransportConfig {
    pub fn new(endpoint: Uri) -> Self {
        Self {
            endpoint,
            connect_timeout: Duration::from_secs(60),
        }
    }

    /// The endpoint this connector can reach, or why it cannot.
    fn target(&self) -> Result<(&str, u16), TransportError> {
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

        let host = self.endpoint.host().ok_or_else(|| {
            TransportError::configuration(format!("the endpoint `{}` names no host", self.endpoint))
        })?;

        // Brackets delimit an IPv6 literal in an authority and are not part of the address.
        let host = host
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(host);

        Ok((host, self.endpoint.port_u16().unwrap_or(80)))
    }
}

/// A connected stream, in the shape hyper reads and writes.
pub type TransportConnection = TokioIo<TcpStream>;

#[derive(Clone, Debug)]
/// The network dial, as a service over the URI to reach.
pub struct TransportConnector {
    config: TransportConfig,
}

impl TransportConnector {
    /// Reads no configuration it cannot use: an endpoint this connector cannot dial is
    /// refused here rather than at the first call.
    pub fn new(config: TransportConfig) -> Result<Self, TransportError> {
        config.target()?;
        Ok(Self { config })
    }

    async fn dial(config: TransportConfig) -> Result<TransportConnection, TransportError> {
        let (host, port) = config.target()?;

        let connect = async {
            let addresses = tokio::net::lookup_host((host, port))
                .await
                .map_err(|error| TransportError::dns(host, port, &error))?;

            let mut last = None;
            for address in addresses {
                match TcpStream::connect(address).await {
                    Ok(stream) => {
                        // Nagle batches small writes, which is the opposite of what a
                        // request stream wants.
                        let _ = stream.set_nodelay(true);
                        return Ok(TokioIo::new(stream));
                    }
                    Err(error) => last = Some(error),
                }
            }

            Err(match last {
                Some(error) => TransportError::tcp(host, port, &error),
                None => TransportError::dns_empty(host, port),
            })
        };

        match tokio::time::timeout(config.connect_timeout, connect).await {
            Ok(result) => result,
            Err(_) => Err(TransportError::timeout(host, port, config.connect_timeout)),
        }
    }
}

/// Establishes an HTTP/2 session over a connected stream.
///
/// The dial and the session on it both belong to this layer; what travels on the session
/// does not. Naming the body and the executor concretely would be naming layer 2's types
/// here, so they stay parameters even though exactly one of each is ever passed.
pub(crate) async fn handshake<E, B>(
    endpoint: &Uri,
    executor: E,
    io: TransportConnection,
) -> Result<(SendRequest<B>, Connection<TransportConnection, B, E>), TransportError>
where
    E: Http2ClientConnExec<B, TransportConnection> + Unpin + Clone,
    B: hyper::body::Body + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    hyper::client::conn::http2::Builder::new(executor)
        .handshake(io)
        .await
        .map_err(|error| TransportError::http2_handshake(endpoint, &error))
}

impl Service<Uri> for TransportConnector {
    type Response = TransportConnection;
    type Error = TransportError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, target: Uri) -> Self::Future {
        let mut config = self.config.clone();
        config.endpoint = target;
        Box::pin(Self::dial(config))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
/// What stood between the configuration and a connected stream.
pub enum TransportErrorKind {
    DnsResolution,
    TcpConnect,
    Http2Handshake,
    Timeout,
    Configuration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A dial that did not happen, and why.
///
/// The cause chain is rendered rather than carried: an endpoint may hold credentials, and a
/// structured source would put whatever a dependency chose to say about it in front of a
/// caller that only needs to know what failed.
pub struct TransportError {
    kind: TransportErrorKind,
    message: String,
}

impl TransportError {
    /// What kind of failure this is.
    pub fn kind(&self) -> &TransportErrorKind {
        &self.kind
    }

    fn configuration(message: String) -> Self {
        Self {
            kind: TransportErrorKind::Configuration,
            message,
        }
    }

    fn dns(host: &str, port: u16, error: &io::Error) -> Self {
        Self {
            kind: TransportErrorKind::DnsResolution,
            message: format!("`{host}:{port}` did not resolve: {error}"),
        }
    }

    fn dns_empty(host: &str, port: u16) -> Self {
        Self {
            kind: TransportErrorKind::DnsResolution,
            message: format!("`{host}:{port}` resolved to no address at all"),
        }
    }

    fn tcp(host: &str, port: u16, error: &io::Error) -> Self {
        Self {
            kind: TransportErrorKind::TcpConnect,
            message: format!("no address of `{host}:{port}` accepted a connection: {error}"),
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

    fn timeout(host: &str, port: u16, after: Duration) -> Self {
        Self {
            kind: TransportErrorKind::Timeout,
            message: format!("connecting to `{host}:{port}` outlasted {after:?}"),
        }
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TransportError {}
