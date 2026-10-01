//! Reaching the endpoint through an HTTP proxy.
//!
//! A `CONNECT` tunnel rather than an absolute-form request, so TLS stays end to end with the real
//! server: [`ProxyConnector`] sits below the TLS connector and hands back the stream a direct dial
//! would. The handshake is `hyper_util`'s [`Tunnel`].

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use base64::Engine;
use hyper::http::HeaderValue;
use hyper::Uri;
use hyper_util::client::legacy::connect::proxy::Tunnel;
use hyper_util::rt::TokioIo;
use secrecy::{ExposeSecret, SecretString};
use snafu::Snafu;
use tokio::net::TcpStream;
use tower_service::Service;

use crate::utils::safe_endpoint;

/// What a connector reports when it fails.
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Where the proxy is.
#[derive(Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProxySource {
    /// No proxy: the endpoint is dialled directly.
    #[default]
    Disabled,
    /// This proxy, an `http://` URI carrying no credentials.
    Explicit(Uri),
}

/// The URI is printed without its userinfo, which a hand-built `Explicit` may still carry.
impl std::fmt::Debug for ProxySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("Disabled"),
            Self::Explicit(uri) => f
                .debug_tuple("Explicit")
                .field(&format_args!("{}", safe_endpoint(uri)))
                .finish(),
        }
    }
}

/// The proxy a connection tunnels through, and how it authenticates to it.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct ProxyConfig {
    pub source: ProxySource,
    /// Empty for none.
    pub username: String,
    /// Empty for none; never printed.
    pub password: SecretString,
}

impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("source", &self.source)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

impl ProxyConfig {
    /// The ready `Proxy-Authorization` value, or none when no credential is set.
    fn authorization(&self) -> Option<HeaderValue> {
        let password = self.password.expose_secret();
        if self.username.is_empty() && password.is_empty() {
            return None;
        }
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{password}", self.username));
        let mut value = HeaderValue::from_str(&format!("Basic {encoded}"))
            .expect("base64 is always a valid header value");
        value.set_sensitive(true);
        Some(value)
    }
}

/// `target` with the port its scheme implies, when it names none.
fn with_default_port(target: Uri) -> Uri {
    if target.port().is_some() {
        return target;
    }
    let (Some(scheme), Some(authority)) = (target.scheme_str(), target.authority()) else {
        return target;
    };
    let port = if scheme == "https" { 443 } else { 80 };
    let authority = format!("{}:{port}", authority.as_str());
    let mut parts = target.clone().into_parts();
    match authority.parse() {
        Ok(authority) => {
            parts.authority = Some(authority);
            Uri::from_parts(parts).unwrap_or(target)
        }
        Err(_) => target,
    }
}

/// A TCP connector that tunnels through the configured proxy, or dials directly when there is none.
///
/// `Tunnel` bounds no handshake of its own; the connect timeout bounds the whole of a dial, proxy
/// included, one level up.
#[derive(Debug, Clone)]
pub struct ProxyConnector<S> {
    inner: S,
    route: Option<(Uri, Option<HeaderValue>)>,
}

impl<S> ProxyConnector<S> {
    pub(crate) fn new(inner: S, proxy: &ProxyConfig) -> Self {
        let route = match &proxy.source {
            ProxySource::Disabled => None,
            ProxySource::Explicit(uri) => Some((uri.clone(), proxy.authorization())),
        };
        Self { inner, route }
    }
}

impl<S> Service<Uri> for ProxyConnector<S>
where
    S: Service<Uri, Response = TokioIo<TcpStream>> + Clone + Send + 'static,
    S::Error: Into<BoxError> + Send + Sync + 'static,
    S::Future: Send + 'static,
{
    type Response = TokioIo<TcpStream>;
    type Error = BoxError;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, target: Uri) -> Self::Future {
        let Some((proxy, authorization)) = self.route.clone() else {
            let dialling = self.inner.call(target);
            return Box::pin(async move { dialling.await.map_err(Into::into) });
        };

        // `Tunnel` asks for port 443 when the target names none, whatever its scheme, where a
        // direct dial of `http://` takes 80.
        let target = with_default_port(target);

        // The proxy is dialled by the inner connector, so failing to reach it and failing the
        // handshake over it are told apart by where they happen; only the handshake's 407 is told
        // apart by its text.
        let dialling = self.inner.call(proxy.clone());
        Box::pin(async move {
            let proxy_name = safe_endpoint(&proxy);
            let stream = dialling.await.map_err(|error| ProxyError::Unreachable {
                proxy: proxy_name.clone(),
                cause: crate::utils::chain(error.into().as_ref(), ": "),
            })?;

            let mut tunnel = Tunnel::new(proxy, Connected(Some(stream)));
            if let Some(authorization) = authorization {
                tunnel = tunnel.with_auth(authorization);
            }
            tunnel
                .call(target)
                .await
                .map_err(|error| refused(proxy_name, &error).into())
        })
    }
}

/// What `Tunnel` reports, named by the proxy.
///
/// The 407 is told apart by its text: `hyper_util`'s tunnel error has no public path to match it
/// on. `a_proxy_demanding_credentials_it_was_not_given_says_so` is what notices if the wording
/// changes.
fn refused(proxy: String, error: &(dyn std::error::Error + 'static)) -> ProxyError {
    let cause = crate::utils::chain(error, ": ");
    if cause.contains("proxy authorization required") {
        return ProxyError::AuthenticationRequired { proxy };
    }
    ProxyError::TunnelRefused { proxy, cause }
}

/// The one stream the proxy was dialled into, which `Tunnel` handshakes over.
struct Connected(Option<TokioIo<TcpStream>>);

impl Service<Uri> for Connected {
    type Response = TokioIo<TcpStream>;
    type Error = BoxError;
    type Future = std::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _proxy: Uri) -> Self::Future {
        std::future::ready(
            self.0
                .take()
                .ok_or_else(|| BoxError::from("a tunnel is opened once per connection")),
        )
    }
}

/// Why the endpoint could not be reached through the proxy. No variant holds a credential.
#[derive(Clone, Debug, Eq, PartialEq, Snafu)]
#[non_exhaustive]
pub enum ProxyError {
    #[snafu(display("the proxy `{proxy}` could not be reached: {cause}"))]
    Unreachable { proxy: String, cause: String },
    #[snafu(display(
        "the proxy `{proxy}` asks for credentials, which it was not given or refused"
    ))]
    AuthenticationRequired { proxy: String },
    #[snafu(display("the proxy `{proxy}` did not open the tunnel: {cause}"))]
    TunnelRefused { proxy: String, cause: String },
}
