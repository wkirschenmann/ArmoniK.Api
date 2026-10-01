//! Reaching the endpoint through an HTTP proxy.
//!
//! A `CONNECT` tunnel rather than an absolute-form request, so TLS stays end to end with the real
//! server: [`ProxyConnector`] sits below the TLS connector and hands back the stream a direct dial
//! would. The handshake is `hyper_util`'s [`Tunnel`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use base64::Engine;
use hyper::http::HeaderValue;
use hyper::Uri;
use hyper_util::client::legacy::connect::proxy::Tunnel;
use hyper_util::client::proxy::matcher::Matcher;
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
    /// The environment's: `ALL_PROXY`, `HTTPS_PROXY` and `HTTP_PROXY` in either case, with
    /// `NO_PROXY` matched as curl matches it, read when the channel is made. A loopback endpoint
    /// is dialled directly, so a local server stays reachable under a corporate proxy.
    System,
}

/// The URI is printed without its userinfo, which a hand-built `Explicit` may still carry.
impl std::fmt::Debug for ProxySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("Disabled"),
            Self::System => f.write_str("System"),
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
        Some(basic(&self.username, password))
    }

    /// The value for a proxy the environment names, given the one `Matcher` built from that
    /// proxy's URL: each half of the credentials set here takes the place of the URL's.
    fn merged(&self, from_env: Option<&HeaderValue>) -> Option<HeaderValue> {
        let password = self.password.expose_secret();
        if self.username.is_empty() && password.is_empty() {
            return from_env.cloned();
        }
        let (url_username, url_password) = from_env.map(unbasic).unwrap_or_default();
        let username = if self.username.is_empty() {
            &url_username
        } else {
            &self.username
        };
        let password = if password.is_empty() {
            &url_password
        } else {
            password
        };
        Some(basic(username, password))
    }
}

/// A `Basic` `Proxy-Authorization` value, marked sensitive so it is never logged.
fn basic(username: &str, password: &str) -> HeaderValue {
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
    let mut value = HeaderValue::from_str(&format!("Basic {encoded}"))
        .expect("base64 is always a valid header value");
    value.set_sensitive(true);
    value
}

/// The username and password a `Basic` value carries, split at the first `:`, which RFC 7617
/// forbids in the username: one the URL's username percent-encodes moves into the password. A
/// value `hyper_util` did not build decodes as nothing.
fn unbasic(value: &HeaderValue) -> (String, String) {
    let encoded = value
        .to_str()
        .unwrap_or_default()
        .trim_start_matches("Basic ");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap_or_default();
    let decoded = String::from_utf8_lossy(&decoded);
    match decoded.split_once(':') {
        Some((username, password)) => (username.to_owned(), password.to_owned()),
        None => (decoded.into_owned(), String::new()),
    }
}

/// Whether `target` names this machine: `localhost` and its subdomains, which RFC 6761 keeps
/// loopback, or a loopback address. The environment's proxy is never asked to reach it.
fn is_loopback(target: &Uri) -> bool {
    let Some(host) = target.host() else {
        return false;
    };
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(address)) => address.is_loopback(),
        Ok(std::net::IpAddr::V6(address)) => {
            address.is_loopback()
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|address| address.is_loopback())
        }
        Err(_) => false,
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
    route: Route,
}

#[derive(Debug, Clone)]
enum Route {
    Direct,
    Via(Uri, Option<HeaderValue>),
    /// The matcher, and the credentials that take the place of the URL's. Behind an `Arc`
    /// because a connector is cloned per dial and a `Matcher` is not `Clone`.
    Environment(Arc<(Matcher, ProxyConfig)>),
}

impl<S> ProxyConnector<S> {
    pub(crate) fn new(inner: S, proxy: &ProxyConfig) -> Self {
        let route = match &proxy.source {
            ProxySource::Disabled => Route::Direct,
            ProxySource::Explicit(uri) => Route::Via(uri.clone(), proxy.authorization()),
            ProxySource::System => {
                Route::Environment(Arc::new((Matcher::from_env(), proxy.clone())))
            }
        };
        Self { inner, route }
    }

    /// The proxy `target` is reached through, and the `Proxy-Authorization` it is shown; none
    /// for a direct dial.
    pub(crate) fn route_to(
        &self,
        target: &Uri,
    ) -> Result<Option<(Uri, Option<HeaderValue>)>, ProxyError> {
        match &self.route {
            Route::Direct => Ok(None),
            Route::Via(proxy, authorization) => Ok(Some((proxy.clone(), authorization.clone()))),
            Route::Environment(_) if is_loopback(target) => Ok(None),
            Route::Environment(environment) => {
                let (matcher, dedicated) = environment.as_ref();
                let Some(intercept) = matcher.intercept(target) else {
                    return Ok(None);
                };
                let proxy = intercept.uri().clone();
                if proxy.scheme_str() != Some("http") {
                    return Err(ProxyError::NotHttp {
                        proxy: safe_endpoint(&proxy),
                    });
                }
                Ok(Some((proxy, dedicated.merged(intercept.basic_auth()))))
            }
        }
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
        let (proxy, authorization) = match self.route_to(&target) {
            Ok(Some(route)) => route,
            Ok(None) => {
                let dialling = self.inner.call(target);
                return Box::pin(async move { dialling.await.map_err(Into::into) });
            }
            Err(refused) => return Box::pin(std::future::ready(Err(refused.into()))),
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
    #[snafu(display(
        "the proxy `{proxy}` the environment names is not an `http://` URL, the only kind this \
         connector tunnels through"
    ))]
    NotHttp { proxy: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_and_address_of_this_machine_is_loopback() {
        for endpoint in [
            "http://localhost:1",
            "http://LocalHost.:1",
            "http://node.localhost:1",
            "http://127.0.0.1:1",
            "http://127.1.2.3:1",
            "http://[::1]:1",
            "http://[::ffff:127.0.0.1]:1",
        ] {
            assert!(is_loopback(&Uri::from_static(endpoint)), "{endpoint}");
        }
        for endpoint in [
            "http://localhost.test:1",
            "http://mylocalhost:1",
            "http://10.0.0.1:1",
            "http://[::ffff:10.0.0.1]:1",
        ] {
            assert!(!is_loopback(&Uri::from_static(endpoint)), "{endpoint}");
        }
    }
}
