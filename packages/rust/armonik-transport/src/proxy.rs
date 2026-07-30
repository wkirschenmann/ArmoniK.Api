//! Reaching the endpoint through an HTTP proxy.
//!
//! Proxying uses an HTTP `CONNECT` tunnel rather than an absolute-form request, so TLS — including
//! mutual TLS — is negotiated end to end with the real server and the proxy only ever forwards
//! opaque bytes. That matters here: the whole point of this transport is to control the TLS stack,
//! which would be defeated by terminating it at the proxy.
//!
//! [`ProxyConnector`] sits between the TCP connector and the TLS connector: it dials the proxy,
//! establishes the tunnel, and hands back the very same stream type the plain TCP connector
//! returns, so the TLS layer above it is unaffected.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::http::uri::{Authority, Scheme};
use hyper::Uri;
use hyper_util::rt::TokioIo;
use snafu::{IntoError, ResultExt, Snafu};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower_service::Service;

use super::tcp::TcpConnector;
use super::{ProxyConfig, ProxySource};

/// Upper bound on the response head a proxy may send, to stop a hostile or broken proxy from
/// making us buffer without end.
const MAX_RESPONSE_HEAD: usize = 8 * 1024;

/// Upper bound on the whole tunnel handshake, so a proxy that accepts the connection and then goes
/// quiet fails instead of hanging.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// A TCP connector that tunnels through an HTTP proxy when one is configured.
///
/// Requests that must not be proxied — proxying disabled, or a host matched by `NO_PROXY` — are
/// passed straight to the inner connector, so the non-proxied path is exactly what it was before.
#[derive(Debug, Clone)]
pub struct ProxyConnector {
    inner: TcpConnector,
    proxy: ProxyConfig,
}

impl ProxyConnector {
    /// Wrap a TCP connector with the given proxy configuration.
    pub(crate) fn new(inner: TcpConnector, proxy: ProxyConfig) -> Self {
        Self { inner, proxy }
    }
}

impl Service<Uri> for ProxyConnector {
    type Response = TokioIo<TcpStream>;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, target: Uri) -> Self::Future {
        let proxy_uri = match resolve_proxy(&self.proxy, &target) {
            Ok(proxy_uri) => proxy_uri,
            Err(error) => return Box::pin(std::future::ready(Err(error.into()))),
        };

        // Nothing to tunnel through: keep the original behaviour untouched.
        let Some(proxy_uri) = proxy_uri else {
            let future = self.inner.call(target);
            return Box::pin(future);
        };

        let authority = match target_authority(&target) {
            Ok(authority) => authority,
            Err(error) => return Box::pin(std::future::ready(Err(error.into()))),
        };
        let credentials = self
            .proxy
            .credentials()
            .map(|(user, password)| basic_auth(user, password));

        let connect_to_proxy = self.inner.call(proxy_uri.clone());

        Box::pin(async move {
            // The TCP connector already yields a boxed error, so it needs no further wrapping.
            let stream = connect_to_proxy.await.map_err(|source| {
                ConnectSnafu {
                    proxy: proxy_uri.clone(),
                }
                .into_error(source)
            })?;
            let mut stream = stream.into_inner();

            let handshake = tunnel(&mut stream, &authority, credentials.as_deref());
            tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake)
                .await
                .map_err(|_| {
                    HandshakeTimeoutSnafu {
                        proxy: proxy_uri.clone(),
                        timeout: HANDSHAKE_TIMEOUT,
                    }
                    .build()
                })??;

            tracing::debug!(proxy = %proxy_uri, target = %authority, "Established proxy tunnel");

            Ok(TokioIo::new(stream))
        })
    }
}

/// Decide which proxy, if any, should be used to reach `target`.
fn resolve_proxy(proxy: &ProxyConfig, target: &Uri) -> Result<Option<Uri>, ProxyError> {
    match &proxy.source {
        ProxySource::Disabled => Ok(None),
        // `NO_PROXY` deliberately does *not* apply here. It is part of the same environment
        // convention as `HTTPS_PROXY`, so it belongs to `System`; ArmoniK's configuration gives an explicit
        // proxy as `new WebProxy(url, false, Array.Empty<string>(), …)` — an empty bypass list that
        // ignores `NO_PROXY` entirely. Honouring it for an explicitly-configured proxy would mean a
        // request bypassing the proxy here while going through it there.
        ProxySource::Explicit(uri) => Ok(Some(uri.clone())),
        ProxySource::System => {
            let Some(uri) = system_proxy(target) else {
                return Ok(None);
            };

            if let Some(host) = target.host() {
                if no_proxy_matches(&read_env_first(&["NO_PROXY", "no_proxy"]), host) {
                    tracing::debug!(host, "Bypassing the proxy, host matched by NO_PROXY");
                    return Ok(None);
                }
            }

            Ok(Some(uri))
        }
    }
}

/// Read the proxy for `target` from the environment, following the usual `*_PROXY` convention.
fn system_proxy(target: &Uri) -> Option<Uri> {
    let names: &[&str] = if target.scheme() == Some(&Scheme::HTTPS) {
        &["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"]
    } else {
        &["HTTP_PROXY", "http_proxy"]
    };

    let value = read_env_first(names);
    if value.is_empty() {
        return None;
    }

    let with_scheme = if value.contains("://") {
        value
    } else {
        format!("http://{value}")
    };

    Uri::try_from(with_scheme).ok()
}

/// First non-empty value among `names`.
fn read_env_first(names: &[&str]) -> String {
    names
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

/// Whether `host` is covered by a `NO_PROXY` list.
///
/// Follows the de-facto convention: `*` bypasses everything, a leading dot or a bare domain matches
/// that domain and its subdomains, and matching is case-insensitive. Any port in an entry is
/// ignored, since the decision is per host.
fn no_proxy_matches(no_proxy: &str, host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();

    no_proxy.split(',').map(str::trim).any(|entry| {
        if entry.is_empty() {
            return false;
        }
        if entry == "*" {
            return true;
        }

        let entry = entry
            .split(':')
            .next()
            .unwrap_or(entry)
            .trim_start_matches('.')
            .to_ascii_lowercase();

        host == entry || host.ends_with(&format!(".{entry}"))
    })
}

/// The `host:port` the tunnel should be opened to, defaulting the port from the scheme.
fn target_authority(target: &Uri) -> Result<Authority, ProxyError> {
    let host = target.host().ok_or_else(|| {
        UnsupportedTargetSnafu {
            target: target.clone(),
            reason: String::from("no host"),
        }
        .build()
    })?;

    let port = target.port_u16().unwrap_or_else(|| {
        if target.scheme() == Some(&Scheme::HTTPS) {
            443
        } else {
            80
        }
    });

    // Bracket IPv6 literals so the authority stays parseable.
    let authority = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };

    Authority::try_from(authority.as_str()).ok().ok_or_else(|| {
        UnsupportedTargetSnafu {
            target: target.clone(),
            reason: format!("`{authority}` is not a valid authority"),
        }
        .build()
    })
}

/// Perform the `CONNECT` handshake on an already-open connection to the proxy.
async fn tunnel(
    stream: &mut TcpStream,
    target: &Authority,
    credentials: Option<&str>,
) -> Result<(), ProxyError> {
    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(credentials) = credentials {
        request.push_str("Proxy-Authorization: Basic ");
        request.push_str(credentials);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");

    stream
        .write_all(request.as_bytes())
        .await
        .context(HandshakeIoSnafu {})?;
    stream.flush().await.context(HandshakeIoSnafu {})?;

    // Read one byte at a time and stop on the blank line that ends the head. Buffered reads could
    // consume bytes belonging to the tunnel itself; that cannot legitimately happen here because
    // both TLS and HTTP/2 have the client speak first, but a byte-wise read removes the caveat
    // entirely, and it happens once per connection over roughly a hundred bytes.
    let mut head = Vec::with_capacity(128);
    loop {
        let mut byte = [0u8; 1];
        if stream
            .read_exact(&mut byte)
            .await
            .context(HandshakeIoSnafu {})
            .is_err()
        {
            return TruncatedResponseSnafu {}.fail();
        }
        head.push(byte[0]);

        if head.ends_with(b"\r\n\r\n") {
            break;
        }
        if head.len() >= MAX_RESPONSE_HEAD {
            return ResponseTooLargeSnafu {
                limit: MAX_RESPONSE_HEAD,
            }
            .fail();
        }
    }

    match status_code(&head) {
        Some(200) => Ok(()),
        Some(407) => AuthenticationRequiredSnafu {}.fail(),
        Some(status) => RejectedSnafu { status }.fail(),
        None => MalformedResponseSnafu {}.fail(),
    }
}

/// Extract the status code from an HTTP response head.
fn status_code(head: &[u8]) -> Option<u16> {
    let line = head.split(|byte| *byte == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.split_whitespace();

    let version = parts.next()?;
    if !version.starts_with("HTTP/") {
        return None;
    }

    parts.next()?.parse().ok()
}

/// Encode credentials for the `Basic` authentication scheme.
fn basic_auth(username: &str, password: &str) -> String {
    base64_encode(format!("{username}:{password}").as_bytes())
}

/// Standard base64 with padding, as defined by RFC 4648 §4.
///
/// Written out rather than pulled in as a dependency: it is a handful of lines, and this is the
/// only place in the crate that needs it.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);

    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let packed = u32::from(bytes[0]) << 16 | u32::from(bytes[1]) << 8 | u32::from(bytes[2]);

        for offset in 0..4 {
            // Emit padding rather than data for the sextets that lie past the end of the input.
            if offset <= chunk.len() {
                let index = (packed >> (18 - 6 * offset)) & 0b11_1111;
                out.push(char::from(ALPHABET[index as usize]));
            } else {
                out.push('=');
            }
        }
    }

    out
}

/// Failure to reach the endpoint through the proxy.
#[derive(Debug, Snafu)]
#[non_exhaustive]
pub enum ProxyError {
    #[snafu(display("Could not connect to the proxy {proxy} [{location}]"))]
    #[non_exhaustive]
    Connect {
        proxy: Uri,
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display(
        "The proxy {proxy} did not complete the tunnel within {timeout:?} [{location}]"
    ))]
    #[non_exhaustive]
    HandshakeTimeout {
        proxy: Uri,
        timeout: Duration,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("Could not exchange the CONNECT handshake with the proxy [{location}]"))]
    #[non_exhaustive]
    HandshakeIo {
        source: std::io::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("The proxy closed the connection during the CONNECT handshake [{location}]"))]
    #[non_exhaustive]
    TruncatedResponse {
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display(
        "The proxy sent more than {limit} bytes of response head to CONNECT [{location}]"
    ))]
    #[non_exhaustive]
    ResponseTooLarge {
        limit: usize,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("The proxy sent a malformed response to CONNECT [{location}]"))]
    #[non_exhaustive]
    MalformedResponse {
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display(
        "The proxy requires authentication; set `GrpcClient__ProxyUsername` and \
         `GrpcClient__ProxyPassword` [{location}]"
    ))]
    #[non_exhaustive]
    AuthenticationRequired {
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("The proxy refused to open the tunnel, HTTP status {status} [{location}]"))]
    #[non_exhaustive]
    Rejected {
        status: u16,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("Cannot tunnel to {target}: {reason} [{location}]"))]
    #[non_exhaustive]
    UnsupportedTarget {
        target: Uri,
        reason: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        // RFC 4648 section 10, which exercises every padding case.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_covers_the_whole_alphabet() {
        // Bytes 0..=255 exercise every sextet value, including the `+` and `/` at the end of the
        // alphabet that a hand-written table is easy to get wrong.
        let all = (0..=255u8).collect::<Vec<_>>();
        let encoded = base64_encode(&all);
        assert!(encoded.contains('+'), "missing `+`: {encoded}");
        assert!(encoded.contains('/'), "missing `/`: {encoded}");
        assert_eq!(encoded.len(), 344);
    }

    #[test]
    fn basic_auth_encodes_user_and_password() {
        // The canonical example from RFC 7617.
        assert_eq!(
            basic_auth("Aladdin", "open sesame"),
            "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }

    #[test]
    fn status_code_is_read_from_the_first_line() {
        assert_eq!(
            status_code(b"HTTP/1.1 200 Connection established\r\n\r\n"),
            Some(200)
        );
        assert_eq!(
            status_code(b"HTTP/1.0 407 Proxy Authentication Required\r\n\r\n"),
            Some(407)
        );
        // No reason phrase is still valid.
        assert_eq!(status_code(b"HTTP/1.1 502\r\n\r\n"), Some(502));
    }

    #[test]
    fn status_code_rejects_non_http_responses() {
        assert_eq!(status_code(b"\r\n\r\n"), None);
        assert_eq!(status_code(b"NOTHTTP 200 OK\r\n\r\n"), None);
        assert_eq!(status_code(b"HTTP/1.1 nonsense\r\n\r\n"), None);
        // A raw TLS record must not be mistaken for a response.
        assert_eq!(status_code(&[0x16, 0x03, 0x01, 0x00, 0x05]), None);
    }

    #[test]
    fn target_authority_defaults_the_port_from_the_scheme() {
        let authority = |uri: &str| {
            target_authority(&Uri::try_from(uri).unwrap())
                .unwrap()
                .to_string()
        };

        assert_eq!(
            authority("https://armonik.example.com/"),
            "armonik.example.com:443"
        );
        assert_eq!(
            authority("http://armonik.example.com/"),
            "armonik.example.com:80"
        );
        assert_eq!(
            authority("https://armonik.example.com:5001/"),
            "armonik.example.com:5001"
        );
    }

    #[test]
    fn target_authority_brackets_ipv6_literals() {
        let uri = Uri::try_from("https://[::1]:5001/").unwrap();
        assert_eq!(target_authority(&uri).unwrap().to_string(), "[::1]:5001");
    }

    #[test]
    fn no_proxy_matches_domains_and_subdomains() {
        let list = "localhost, .corp.example.com, other.com";

        assert!(no_proxy_matches(list, "localhost"));
        assert!(no_proxy_matches(list, "corp.example.com"));
        assert!(no_proxy_matches(list, "api.corp.example.com"));
        assert!(no_proxy_matches(list, "other.com"));
        assert!(no_proxy_matches(list, "deep.nested.other.com"));

        assert!(!no_proxy_matches(list, "example.com"));
        assert!(!no_proxy_matches(list, "notlocalhost"));
        // A suffix that is not on a label boundary must not match.
        assert!(!no_proxy_matches(list, "evilother.com"));
    }

    #[test]
    fn no_proxy_is_case_insensitive_and_ignores_ports() {
        assert!(no_proxy_matches("CORP.EXAMPLE.COM", "api.corp.example.com"));
        assert!(no_proxy_matches(
            "corp.example.com:8080",
            "corp.example.com"
        ));
    }

    #[test]
    fn no_proxy_wildcard_bypasses_everything() {
        assert!(no_proxy_matches("*", "anything.example.com"));
    }

    #[test]
    fn empty_no_proxy_matches_nothing() {
        assert!(!no_proxy_matches("", "armonik.example.com"));
        assert!(!no_proxy_matches(",  ,", "armonik.example.com"));
    }

    #[test]
    fn disabled_proxy_is_never_resolved() {
        let target = Uri::try_from("https://armonik.example.com:5001/").unwrap();
        let resolved = resolve_proxy(&ProxyConfig::default(), &target).unwrap();
        assert_eq!(resolved, None);
    }

    #[test]
    fn explicit_proxy_is_used_as_is() {
        let target = Uri::try_from("https://armonik.example.com:5001/").unwrap();
        let proxy = ProxyConfig::explicit(Uri::try_from("http://proxy.corp:3128").unwrap());

        let resolved = resolve_proxy(&proxy, &target).unwrap().expect("a proxy");
        assert_eq!(resolved.host(), Some("proxy.corp"));
        assert_eq!(resolved.port_u16(), Some(3128));
    }

    #[test]
    #[serial_test::serial(env)]
    fn no_proxy_does_not_apply_to_an_explicitly_configured_proxy() {
        // `NO_PROXY` belongs to the same environment convention as `HTTPS_PROXY`, so it governs
        // `System` only. ArmoniK's configuration gives an explicit proxy an empty bypass list, and diverging
        // here would mean a request skipping the proxy in this transport while using it in that one.
        //
        // Set for the duration of this test only; `resolve_proxy` reads the variable itself, so
        // there is no way to inject it. Marked `serial` for that reason.
        let target = Uri::try_from("https://armonik.example.com:5001/").unwrap();
        let proxy = ProxyConfig::explicit(Uri::try_from("http://proxy.corp:3128").unwrap());

        let restore = std::env::var("NO_PROXY").ok();
        // SAFETY: single-threaded within this test, which is `serial` for exactly this reason.
        unsafe { std::env::set_var("NO_PROXY", "armonik.example.com") };

        let resolved = resolve_proxy(&proxy, &target).unwrap();

        match restore {
            // SAFETY: as above.
            Some(previous) => unsafe { std::env::set_var("NO_PROXY", previous) },
            None => unsafe { std::env::remove_var("NO_PROXY") },
        }

        assert!(
            resolved.is_some(),
            "an explicit proxy must be used even when NO_PROXY names the target"
        );
    }
}
