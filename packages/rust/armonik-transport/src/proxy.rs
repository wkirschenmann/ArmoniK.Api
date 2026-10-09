//! Reaching the endpoint through an HTTP proxy.
//!
//! A `CONNECT` tunnel rather than an absolute-form request, so TLS stays end to end with the real
//! server: [`ProxyConnector`] sits below the TLS connector and hands back the stream a direct dial
//! would. The handshake is `hyper_util`'s [`Tunnel`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

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
#[cfg(windows)]
use crate::windows_proxy::{Settings, WindowsProxy};

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
    /// `NO_PROXY` matched as curl matches it, read when the channel is made. On Windows, when the
    /// environment names none, the current user's network settings: a PAC script WinHTTP finds
    /// and runs, else the manual proxy and its bypass list. A loopback endpoint is dialled
    /// directly, so a local server stays reachable under a corporate proxy.
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

/// The username and password a proxy is authenticated to with, by `Basic`. They are one pair: an
/// empty half is an empty string, sent as such and never filled from another source.
#[derive(Clone)]
#[non_exhaustive]
pub struct BasicCredentials {
    pub username: String,
    /// Never printed.
    pub password: SecretString,
}

impl BasicCredentials {
    /// The pair as stated, an empty half included.
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into().into(),
        }
    }
}

impl std::fmt::Debug for BasicCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BasicCredentials")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// The proxy a connection tunnels through, and how it authenticates to it.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ProxyConfig {
    pub source: ProxySource,
    /// The credentials stated for the proxy, or none, in which case the URL of a proxy the
    /// environment names applies.
    pub credentials: Option<BasicCredentials>,
}

impl ProxyConfig {
    /// The ready `Proxy-Authorization` value, or none when no credentials are stated.
    fn authorization(&self) -> Option<HeaderValue> {
        self.credentials
            .as_ref()
            .map(|pair| basic(&pair.username, pair.password.expose_secret()))
    }

    /// The value for a proxy the environment names, given the one `Matcher` built from that
    /// proxy's URL: the credentials stated here, whole, else the URL's own.
    fn merged(&self, from_env: Option<&HeaderValue>) -> Option<HeaderValue> {
        self.authorization().or_else(|| from_env.cloned())
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
    /// The matcher, and the credentials that take the place of the URL's, when stated. Behind an
    /// `Arc` because a connector is cloned per dial and a `Matcher` is not `Clone`.
    Environment(Arc<(Matcher, ProxyConfig)>),
    /// The user's network settings, and the credentials the proxy they name is shown.
    #[cfg(windows)]
    Windows(Arc<(WindowsProxy, ProxyConfig)>),
}

/// The proxy a dial goes through, and the `Proxy-Authorization` it is shown; none for a direct
/// dial.
type Routed = Option<(Uri, Option<HeaderValue>)>;

impl<S> ProxyConnector<S> {
    /// `timeout` bounds each step of fetching a PAC script the system's settings name.
    pub(crate) fn new(inner: S, proxy: &ProxyConfig, timeout: Duration) -> Self {
        let route = match &proxy.source {
            ProxySource::Disabled => Route::Direct,
            ProxySource::Explicit(uri) => Route::Via(uri.clone(), proxy.authorization()),
            ProxySource::System => system_route(proxy, timeout),
        };
        Self { inner, route }
    }

    /// Where a dial of `target` goes. The Windows settings answer nothing here: finding a PAC
    /// script blocks, so `call` asks them off the runtime's threads.
    pub(crate) fn route_to(&self, target: &Uri) -> Result<Routed, ProxyError> {
        match &self.route {
            Route::Direct => Ok(None),
            Route::Via(proxy, authorization) => Ok(Some((proxy.clone(), authorization.clone()))),
            #[cfg(windows)]
            Route::Windows(_) => Ok(None),
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
                        named_by: Origin::Environment,
                    });
                }
                Ok(Some((proxy, dedicated.merged(intercept.basic_auth()))))
            }
        }
    }
}

/// Whether one of the variables a proxy is named by is set; `NO_PROXY` alone names none.
fn environment_names_a_proxy() -> bool {
    [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

#[cfg_attr(not(windows), allow(unused_variables))]
fn system_route(proxy: &ProxyConfig, timeout: Duration) -> Route {
    if environment_names_a_proxy() {
        return Route::Environment(Arc::new((Matcher::from_env(), proxy.clone())));
    }
    #[cfg(windows)]
    {
        let settings = Settings::current_user();
        if settings.names_a_proxy() {
            return Route::Windows(Arc::new((
                WindowsProxy::new(settings, timeout),
                proxy.clone(),
            )));
        }
    }
    Route::Direct
}

/// The route the Windows settings give `target`, asked on a blocking thread; the connect timeout
/// bounds the wait one level up.
#[cfg(windows)]
async fn through_settings(
    windows: Arc<(WindowsProxy, ProxyConfig)>,
    target: &Uri,
) -> Result<Routed, ProxyError> {
    let resolving = Arc::clone(&windows);
    let asked = target.clone();
    let entry = tokio::task::spawn_blocking(move || resolving.0.resolve(&asked))
        .await
        .map_err(|_| ProxyError::Interrupted)?;
    let Some(entry) = entry else {
        return Ok(None);
    };
    Ok(Some((settings_proxy(&entry)?, windows.1.authorization())))
}

/// The `http://` URI of an entry the settings write as `host:port`, perhaps with a scheme.
#[cfg(windows)]
fn settings_proxy(entry: &str) -> Result<Uri, ProxyError> {
    // A scheme is letters, digits, `+`, `-` and `.`: a `://` after anything else is inside a
    // userinfo, which no message quotes.
    let (scheme, rest) = match entry.split_once("://") {
        Some((scheme, rest))
            if !scheme.is_empty()
                && scheme
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"+-.".contains(&byte)) =>
        {
            (scheme, rest)
        }
        _ => ("http", entry),
    };
    // The last `@`: what precedes it is a userinfo, which no message quotes.
    let authority = rest
        .rsplit_once('@')
        .map_or(rest, |(_, host)| host)
        .trim_end_matches('/');
    if !scheme.eq_ignore_ascii_case("http") {
        return Err(ProxyError::NotHttp {
            proxy: format!("{scheme}://{authority}"),
            named_by: Origin::WindowsSettings,
        });
    }
    Uri::builder()
        .scheme("http")
        .authority(authority)
        .path_and_query("/")
        .build()
        .map_err(|_| ProxyError::Unresolved {
            cause: format!("`{authority}` is not a `host:port`"),
        })
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
        let routed = self.route_to(&target);
        #[cfg(windows)]
        let settings = match &self.route {
            Route::Windows(windows) if !is_loopback(&target) => Some(Arc::clone(windows)),
            _ => None,
        };
        let mut inner = self.inner.clone();
        Box::pin(async move {
            #[cfg(windows)]
            let routed = match settings {
                Some(windows) => through_settings(windows, &target).await,
                None => routed,
            };
            std::future::poll_fn(|cx| inner.poll_ready(cx))
                .await
                .map_err(Into::into)?;
            let Some((proxy, authorization)) = routed? else {
                return inner.call(target).await.map_err(Into::into);
            };

            // `Tunnel` asks for port 443 when the target names none, whatever its scheme, where
            // a direct dial of `http://` takes 80.
            let target = with_default_port(target);

            // The proxy is dialled by the inner connector, so failing to reach it and failing the
            // handshake over it are told apart by where they happen; only the handshake's 407 is
            // told apart by its text.
            let proxy_name = safe_endpoint(&proxy);
            let stream =
                inner
                    .call(proxy.clone())
                    .await
                    .map_err(|error| ProxyError::Unreachable {
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
        "{named_by} the proxy `{proxy}`, which is not an `http://` URL, the only kind this \
         connector tunnels through"
    ))]
    NotHttp { proxy: String, named_by: Origin },
    #[snafu(display("Windows' network settings named no usable proxy: {cause}"))]
    Unresolved { cause: String },
    #[snafu(display("resolving the proxy Windows' network settings name was interrupted"))]
    Interrupted,
}

/// What named a proxy the connector refuses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Origin {
    Environment,
    #[cfg(windows)]
    WindowsSettings,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Environment => "the environment names",
            #[cfg(windows)]
            Self::WindowsSettings => "Windows' network settings name",
        })
    }
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

    /// A proxy that answers one `CONNECT` with 200 and reports the target it was asked for.
    #[cfg(windows)]
    async fn one_tunnel() -> (String, tokio::sync::oneshot::Receiver<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("an address");
        let (asked, heard) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut client, _) = listener.accept().await.expect("a client");
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0u8; 1];
                client.read_exact(&mut byte).await.expect("the request");
                head.push(byte[0]);
            }
            let line = String::from_utf8_lossy(&head)
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned();
            let _ = asked.send(line);
            client
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .expect("the answer");
            let mut rest = Vec::new();
            let _ = client.read_to_end(&mut rest).await;
        });
        (address.to_string(), heard)
    }

    #[cfg(windows)]
    fn through(
        proxy: &str,
        bypass: Option<&str>,
    ) -> ProxyConnector<hyper_util::client::legacy::connect::HttpConnector> {
        let mut http = hyper_util::client::legacy::connect::HttpConnector::new();
        http.enforce_http(false);
        let settings = Settings {
            proxy: Some(proxy.to_owned()),
            bypass: bypass.map(str::to_owned),
            ..Settings::default()
        };
        ProxyConnector {
            inner: http,
            route: Route::Windows(Arc::new((
                WindowsProxy::new(settings, Duration::from_secs(5)),
                ProxyConfig::default(),
            ))),
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn the_proxy_windows_settings_name_is_tunnelled_through() {
        let (proxy, heard) = one_tunnel().await;
        let mut connector = through(&proxy, None);
        connector
            .call(Uri::from_static("http://server.test:5001"))
            .await
            .expect("a tunnel");
        assert_eq!(
            heard.await.expect("asked"),
            "CONNECT server.test:5001 HTTP/1.1"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_host_the_settings_bypass_or_a_loopback_one_is_dialled_directly() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("an address").port();
        let mut connector = through("127.0.0.1:9", Some("*.corp.test"));
        let target = Uri::try_from(format!("http://127.0.0.1:{port}")).expect("a uri");
        connector.call(target).await.expect("a direct dial");
        let refused = connector
            .call(Uri::from_static("http://server.corp.test:1"))
            .await
            .expect_err("no such host");
        assert!(refused.downcast_ref::<ProxyError>().is_none(), "{refused}");
    }

    #[cfg(windows)]
    #[test]
    fn a_settings_entry_is_read_without_quoting_its_userinfo() {
        assert_eq!(
            settings_proxy("proxy.test:3128")
                .expect("an entry")
                .to_string(),
            "http://proxy.test:3128/"
        );
        // A `://` inside the userinfo is not a scheme.
        assert_eq!(
            settings_proxy("alice:a://s3cret@proxy.test:443")
                .expect("an entry")
                .to_string(),
            "http://proxy.test:443/"
        );
        let refused = settings_proxy("socks://alice:s3cret@socks.test:1080")
            .expect_err("not http")
            .to_string();
        assert!(refused.contains("`socks://socks.test:1080`"), "{refused}");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn an_https_proxy_the_settings_name_is_refused_without_its_userinfo() {
        let mut connector = through("https://alice:s3cret@proxy.test:443", None);
        let refused = connector
            .call(Uri::from_static("http://server.test:1"))
            .await
            .expect_err("not http");
        let message = refused.to_string();
        assert!(message.contains("Windows' network settings"), "{message}");
        assert!(message.contains("not an `http://` URL"), "{message}");
        assert!(!message.contains("s3cret"), "{message}");
    }
}
