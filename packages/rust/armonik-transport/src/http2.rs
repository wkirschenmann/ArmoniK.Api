//! Layer 1: the network, up to an HTTP/2 session.
//!
//! [`TransportConnector`] is a [`tower_service::Service<Uri>`] yielding connected streams; it
//! knows neither HTTP/2 nor gRPC and is a network dial and nothing more. [`open`] turns one of
//! those streams into a session. What travels on the session is [`crate::grpc`]'s business.
//!
//! This connector dials plain TCP. Transport security is [`crate::connect`], which hands out a
//! `tonic` channel instead of a stream.

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

    /// Whether this endpoint is one a connector could dial, or why it is not.
    ///
    /// Scheme and host only: the port has a default and the host's shape is the resolver's
    /// business, not this crate's.
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

/// A connected stream, in the shape hyper reads and writes.
pub type TransportConnection = TokioIo<TcpStream>;

#[derive(Clone, Debug)]
/// The network dial, as a service over the URI to reach.
///
/// A shape over `hyper_util`'s own connector rather than a second dialler: resolution, the
/// per-address attempts and the socket options are its, and it races the address families where
/// a hand-rolled loop would try them in turn - so one black-holed address cannot consume the
/// whole budget while a reachable one goes untried. What stays here is the endpoint check this
/// crate makes before any I/O, and the deadline it puts on the sequence as a whole.
pub struct TransportConnector {
    http: HttpConnector,
    /// Bounds the whole dial. `HttpConnector` bounds each attempt instead, which is not what
    /// [`TransportConfig::connect_timeout`] promises.
    connect_timeout: Duration,
}

impl TransportConnector {
    /// Reads no configuration it cannot use: an endpoint this connector cannot dial is
    /// refused here rather than at the first call.
    pub fn new(config: TransportConfig) -> Result<Self, TransportError> {
        config.dialable()?;

        let mut http = HttpConnector::new();
        // Nagle batches small writes, which is the opposite of what a request stream wants.
        http.set_nodelay(true);
        // Refuses at call time what `dialable` refuses at construction, for a target that did
        // not come from the configuration.
        http.enforce_http(true);

        Ok(Self {
            http,
            connect_timeout: config.connect_timeout,
        })
    }
}

/// Dials `endpoint` and establishes an HTTP/2 session on it.
///
/// One entry point rather than three steps, because the three are one decision: what a caller
/// wants is a session, and how a connection is asked for - a `tower` service polled ready and
/// then called - is this module's business. A caller that performed those steps itself would be
/// carrying the transport's vocabulary in order to say "connect".
///
/// The body and the executor stay parameters even though exactly one of each is ever passed:
/// naming them concretely would name what travels on the session, which does not belong here.
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
    let mut connector = connector.clone();
    std::future::poll_fn(|cx| connector.poll_ready(cx)).await?;
    let io = connector.call(endpoint.clone()).await?;
    handshake(endpoint, executor, io).await
}

/// Establishes an HTTP/2 session over an already connected stream.
async fn handshake<E, B>(
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
        let dialling = self.http.call(target.clone());
        let deadline = self.connect_timeout;

        Box::pin(async move {
            match tokio::time::timeout(deadline, dialling).await {
                Ok(Ok(io)) => Ok(io),
                Ok(Err(error)) => Err(TransportError::connect(&target, &error)),
                Err(_) => Err(TransportError::timeout(&target, deadline)),
            }
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
/// What stood between the configuration and a connected stream.
///
/// `Connect` covers resolution and the connection alike: the dial is `hyper_util`'s, which races
/// address families rather than walking them, so "the name did not resolve" and "no address
/// accepted" stop being two outcomes a caller could act on differently. What failed is in the
/// message; what a caller can do about it is the same either way.
pub enum TransportErrorKind {
    Connect,
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

/// An error and everything under it, as one line.
///
/// The connector's own message names the step and nothing else - "dns error", "tcp connect
/// error" - so what actually went wrong is in the cause. Rendered here rather than carried, for
/// the reason [`TransportError`] gives.
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
