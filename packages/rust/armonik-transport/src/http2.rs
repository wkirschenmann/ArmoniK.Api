//! HTTP/2 to one endpoint: in cleartext for `http://`, over TLS for `https://`.
//!
//! The dial and the connection are hyper's, and the TLS is rustls under hyper-rustls; what this
//! module adds is the endpoint check and the connector shape the gRPC layer drives.
//! [`crate::ClientConfig`] configures the other path and never reaches this one.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::client::conn::http2::{Connection, SendRequest};
use hyper::rt::bounds::Http2ClientConnExec;
use hyper::Uri;
use hyper_rustls::{FixedServerNameResolver, HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use snafu::Snafu;
use tokio::net::TcpStream;
use tower_service::Service;

use crate::tls::{Refused, Trust};
use crate::utils::{chain, safe_endpoint};

/// What this connector needs in order to dial: where, how long to wait, and how to secure it.
///
/// Not [`crate::ClientConfig`], which configures [`crate::connect`] and carries the keepalives
/// tonic reads. The two are separate types because they drive separate engines, and nothing
/// converts between them.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TransportConfig {
    pub endpoint: Uri,
    pub connect_timeout: Duration,
    /// Read for an `https://` endpoint, and refused for an `http://` one unless left default.
    pub tls: TlsConfig,
}

impl TransportConfig {
    pub fn new(endpoint: Uri) -> Self {
        Self {
            endpoint,
            connect_timeout: Duration::from_secs(60),
            tls: TlsConfig::default(),
        }
    }

    /// Everything this connector will not dial, refused before it can be dialled.
    ///
    /// No message prints the endpoint whole. What is refused here includes an endpoint carrying a
    /// password, and a `Configuration` error is rendered into the status a caller reads and into
    /// the log, so printing the thing being refused is how the secret would travel.
    fn dialable(&self) -> Result<(), TransportError> {
        let refuse = |message: String| ConfigurationSnafu { message }.fail();

        match self.endpoint.scheme_str() {
            Some("http" | "https") => {}
            Some(other) => {
                return refuse(format!(
                    "`{other}://` is not a scheme this connector dials; it dials `http://` and \
                     `https://`"
                ))
            }
            None => {
                return refuse(
                    "the endpoint names no scheme; it has to be an `http://` or an `https://` URI"
                        .to_owned(),
                )
            }
        }

        let Some(authority) = self.endpoint.authority() else {
            return refuse("the endpoint names no host".to_owned());
        };

        // First, so that nothing below splits a string that still holds a password, and so that
        // `host()` is a prefix of `as_str()` for the port check.
        if authority.as_str().contains('@') {
            return refuse(
                "the endpoint carries `user:password@`, which HTTP/2 forbids in `:authority` and \
                 which this connector would otherwise put on the wire and in its errors"
                    .to_owned(),
            );
        }

        let host = authority.host();
        if host.is_empty() {
            return refuse("the endpoint names no host".to_owned());
        }

        // A port that does not parse, or a colon with none after it, leaves `port_u16` empty and
        // the connector dials 80, so a typo reaches the wrong service rather than being refused.
        // A bracketed host ends at its `]`, and the parser admits more text after it than a port.
        let after_host = &authority.as_str()[host.len()..];
        if !after_host.is_empty() {
            let Some(port) = after_host.strip_prefix(':') else {
                return refuse(
                    "the endpoint's host is followed by something other than `:` and a port"
                        .to_owned(),
                );
            };
            if port.is_empty() {
                return refuse("the endpoint has a `:` after its host and no port".to_owned());
            }
            // Digits only, because `u16` parsing also takes a leading `+`; and quoted only then,
            // because anything else may be a password that lost its `@`.
            if !port.bytes().all(|byte| byte.is_ascii_digit()) {
                return refuse(
                    "the endpoint's port is not a number; it has to be 1 to 65535".to_owned(),
                );
            }
            if authority.port_u16().is_none_or(|port| port == 0) {
                return refuse(format!("`{port}` is not a port; it has to be 1 to 65535"));
            }
        }

        // Neither reaches the wire: a call is addressed by its method path, which replaces both.
        if self.endpoint.query().is_some() || !matches!(self.endpoint.path(), "" | "/") {
            return refuse(
                "the endpoint carries a path or a query, and a call is addressed by its method, \
                 so neither would be sent"
                    .to_owned(),
            );
        }

        if self.connect_timeout.is_zero() {
            return refuse(
                "a `connect_timeout` of zero elapses before a dial can finish, so every call \
                 would report a timeout"
                    .to_owned(),
            );
        }

        if self.endpoint.scheme_str() == Some("http") && !self.tls.is_default() {
            return refuse(
                "the endpoint is `http://`, which is dialled in the clear, so the TLS settings \
                 given with it would never be used"
                    .to_owned(),
            );
        }

        if self.tls.accept_any_server && !self.tls.roots.is_empty() {
            return refuse(
                "accepting any server certificate and verifying it against the roots given \
                 contradict each other"
                    .to_owned(),
            );
        }

        Ok(())
    }
}

/// How a connection to an `https://` endpoint is secured.
///
/// The default verifies the server against the operating system's roots, under the endpoint's
/// host, and presents no client certificate.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct TlsConfig {
    /// The roots a server certificate is verified against, in place of the system's.
    pub roots: Vec<CertificateDer<'static>>,
    /// Accepts any server certificate. The connection is still encrypted, to whoever answers.
    pub accept_any_server: bool,
    /// The chain and key this client authenticates with.
    pub identity: Option<ClientIdentity>,
    /// The host the server certificate is verified against, and sent as SNI, in place of the
    /// endpoint's: a DNS name or an IP address, with an optional port that is not read.
    pub server_name: Option<String>,
}

impl TlsConfig {
    fn is_default(&self) -> bool {
        self.roots.is_empty()
            && !self.accept_any_server
            && self.identity.is_none()
            && self.server_name.is_none()
    }
}

impl std::fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsConfig")
            .field("roots", &self.roots.len())
            .field("accept_any_server", &self.accept_any_server)
            .field("identity", &self.identity)
            .field("server_name", &self.server_name)
            .finish()
    }
}

/// A client certificate, the chain that leads from it towards a root, and its key.
pub struct ClientIdentity {
    /// The client's certificate first, then each issuer the server may not hold.
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

impl Clone for ClientIdentity {
    fn clone(&self) -> Self {
        Self {
            chain: self.chain.clone(),
            key: self.key.clone_key(),
        }
    }
}

/// The key is not printed: a configuration is logged, and a key in a log is a key given away.
impl std::fmt::Debug for ClientIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientIdentity")
            .field("chain", &self.chain.len())
            .finish_non_exhaustive()
    }
}

/// The name a server certificate is verified against, from what [`TlsConfig::server_name`] holds.
fn verified_name(written: &str) -> Result<ServerName<'static>, TransportError> {
    let refuse = |message: String| ConfigurationSnafu { message }.fail();

    // Before it is parsed or quoted, so a password never reaches the refusal's text.
    if written.contains('@') {
        return refuse("the server name carries `user:password@`".to_owned());
    }
    let host = written
        .parse::<http::uri::Authority>()
        .map(|authority| authority.host().to_owned())
        .unwrap_or_default();
    match crate::tls::server_name(&host) {
        Some(name) => Ok(name),
        None => refuse(format!(
            "`{written}` names no host a certificate can be verified against; it has to be a DNS \
             name or an IP address, as in `server.example.com`, `10.0.0.1` or `[::1]`"
        )),
    }
}

pub type TransportConnection = hyper_rustls::MaybeHttpsStream<TokioIo<TcpStream>>;

#[derive(Clone, Debug)]
pub struct TransportConnector {
    https: HttpsConnector<HttpConnector>,
    connect_timeout: Duration,
}

impl TransportConnector {
    pub fn new(config: TransportConfig) -> Result<Self, TransportError> {
        config.dialable()?;
        let tls = config.tls;
        let server_name = tls.server_name.as_deref().map(verified_name).transpose()?;

        let mut http = HttpConnector::new();
        http.set_nodelay(true);
        // The TLS layer above reads the scheme; this one dials either.
        http.enforce_http(false);

        // A cleartext endpoint gets a configuration that reads nothing: the system's store is a
        // synchronous read that fails on a host with no CA bundle, for a handshake never made.
        let trust = if config.endpoint.scheme_str() == Some("http") {
            Trust::Roots(Vec::new())
        } else if tls.accept_any_server {
            Trust::Anything
        } else if tls.roots.is_empty() {
            Trust::System
        } else {
            Trust::Roots(tls.roots)
        };
        let identity = tls.identity.map(|identity| (identity.chain, identity.key));
        let client_config = crate::tls::client_config(trust, identity).map_err(|refused| {
            let message = match refused {
                Refused::Protocols(error) => {
                    format!(
                        "no TLS protocol version is available to secure the connection: {error}"
                    )
                }
                Refused::Root(error) => format!("a root certificate is refused: {error}"),
                Refused::SystemRoots(error) => {
                    format!("the system's root certificates could not be read: {error}")
                }
                Refused::Identity(error) => {
                    format!("the client certificate and its key are refused: {error}")
                }
            };
            ConfigurationSnafu { message }.build()
        })?;

        let builder = HttpsConnectorBuilder::new()
            .with_tls_config(client_config)
            .https_or_http();
        let builder = match server_name {
            Some(name) => builder.with_server_name_resolver(FixedServerNameResolver::new(name)),
            None => builder,
        };

        Ok(Self {
            // HTTP/2 alone, which is also what ALPN offers: gRPC has no HTTP/1 mapping.
            https: builder.enable_http2().wrap_connector(http),
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
                    endpoint: safe_endpoint(endpoint),
                    cause: chain(&error, ": "),
                }
                .build()
            })
    };

    match tokio::time::timeout(deadline, opening).await {
        Ok(result) => result,
        Err(_) => TimeoutSnafu {
            endpoint: safe_endpoint(endpoint),
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
        let dialling = self.https.call(target.clone());

        Box::pin(async move {
            dialling.await.map_err(|error| {
                let endpoint = safe_endpoint(&target);
                let cause = chain(error.as_ref(), ": ");
                if refused_by_tls(error.as_ref()) {
                    TlsHandshakeSnafu { endpoint, cause }.build()
                } else {
                    ConnectSnafu { endpoint, cause }.build()
                }
            })
        })
    }
}

/// Whether a dial failed in the TLS handshake rather than before it.
///
/// The rustls error arrives inside an `io::Error`, twice over: tokio-rustls wraps it, and
/// hyper-rustls wraps that. An `io::Error` names its inner error's source and not the inner error
/// itself, so the walk opens each one rather than following `source` alone.
fn refused_by_tls(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(error);
    while let Some(error) = next {
        if error.is::<rustls::Error>() {
            return true;
        }
        next = error
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
            .map(|inner| inner as &(dyn std::error::Error + 'static))
            .or_else(|| error.source());
    }
    false
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
    // Rendered, not held: `TransportConnector` is public and its `Service<Uri>` takes any URI,
    // not only the one `new` validated, so the guarantee at the top of this file has to be in the
    // type rather than in who calls it.
    #[snafu(display("`{endpoint}` could not be reached: {cause}"))]
    Connect { endpoint: String, cause: String },
    #[snafu(display("the TLS handshake with `{endpoint}` failed: {cause}"))]
    TlsHandshake { endpoint: String, cause: String },
    #[snafu(display("the HTTP/2 preface could not be written to `{endpoint}`: {cause}"))]
    Http2Handshake { endpoint: String, cause: String },
    #[snafu(display("connecting to `{endpoint}` outlasted {after:?}"))]
    Timeout { endpoint: String, after: Duration },
}

impl TransportError {
    pub fn kind(&self) -> TransportErrorKind {
        match self {
            Self::Configuration { .. } => TransportErrorKind::Configuration,
            // A handshake is part of reaching the peer, as a dial is.
            Self::Connect { .. } | Self::TlsHandshake { .. } => TransportErrorKind::Connect,
            Self::Http2Handshake { .. } => TransportErrorKind::Http2Handshake,
            Self::Timeout { .. } => TransportErrorKind::Timeout,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialable(endpoint: &str) -> Result<(), TransportError> {
        TransportConfig::new(endpoint.parse().expect("a uri")).dialable()
    }

    #[test]
    fn a_plain_http_endpoint_is_what_this_connector_dials() {
        for endpoint in [
            "http://127.0.0.1:1",
            "HTTP://Host:1",
            "http://[::1]:1",
            "http://host",
            "http://host:1/",
        ] {
            assert!(dialable(endpoint).is_ok(), "{endpoint}");
        }
    }

    #[test]
    fn a_scheme_this_connector_does_not_speak_says_so() {
        assert!(dialable("https://h:1").is_ok());
        let refused = dialable("ftp://h:1").expect_err("neither HTTP nor HTTPS");
        assert_eq!(refused.kind(), TransportErrorKind::Configuration);
        assert!(refused.to_string().contains("ftp://"), "{refused}");
    }

    #[test]
    fn tls_settings_on_a_cleartext_endpoint_are_refused_rather_than_ignored() {
        let mut config = TransportConfig::new("http://h:1".parse().expect("a uri"));
        config.tls.accept_any_server = true;
        let refused = config.dialable().expect_err("TLS settings on http://");
        assert!(refused.to_string().contains("http://"), "{refused}");
    }

    #[test]
    fn accepting_any_server_and_naming_roots_is_a_contradiction() {
        let mut config = TransportConfig::new("https://h:1".parse().expect("a uri"));
        config.tls.accept_any_server = true;
        config.tls.roots = vec![CertificateDer::from(vec![0u8])];
        assert!(config.dialable().is_err());
    }

    #[test]
    fn a_server_name_is_a_host_and_its_port_is_not_read() {
        let address = |text: &str| {
            ServerName::from(rustls::pki_types::IpAddr::try_from(text).expect("an address"))
        };
        assert_eq!(
            verified_name("[::1]").expect("an IPv6 literal"),
            address("::1")
        );
        assert_eq!(
            verified_name("[2001:db8::1]:5003").expect("with a port"),
            address("2001:db8::1")
        );
        assert_eq!(
            verified_name("10.0.0.1:5003").expect("an IPv4 address"),
            address("10.0.0.1")
        );
        assert_eq!(
            verified_name("server.example.com").expect("a DNS name"),
            ServerName::try_from("server.example.com").expect("a name")
        );
    }

    #[test]
    fn a_server_name_that_names_nothing_verifiable_is_refused_by_what_it_said() {
        for written in ["-nope-", "[example.com]", ""] {
            let refused = verified_name(written).expect_err(written);
            assert_eq!(
                refused.kind(),
                TransportErrorKind::Configuration,
                "{written}"
            );
            assert!(
                refused.to_string().contains(&format!("`{written}`")),
                "{refused}"
            );
        }
        let refused = verified_name("alice:s3cret@h").expect_err("credentials");
        assert!(!refused.to_string().contains("s3cret"), "{refused}");
    }

    #[test]
    fn an_endpoint_carrying_a_password_is_refused_and_the_password_is_not_in_the_refusal() {
        // It would reach three places otherwise: `:authority` on every request, the message of
        // every dial error, and the connection log.
        let refused = dialable("http://alice:s3cret@h:1").expect_err("credentials");
        assert_eq!(refused.kind(), TransportErrorKind::Configuration);

        // Names, not the shape: the refusal says the words "user:password" to explain itself.
        let said = refused.to_string();
        assert!(!said.contains("s3cret"), "{said}");
        assert!(!said.contains("alice"), "{said}");
    }

    #[test]
    fn an_endpoint_carrying_a_query_is_refused_and_the_query_is_not_in_the_refusal() {
        let refused = dialable("http://h:1/?token=s3cret").expect_err("a query");
        assert!(!refused.to_string().contains("s3cret"), "{refused}");
    }

    #[test]
    fn a_port_that_is_not_a_port_is_refused_rather_than_dialling_eighty() {
        for (endpoint, port) in [
            ("http://h:99999", "99999"),
            ("http://h:0", "0"),
            ("http://[::1]:0", "0"),
        ] {
            let refused = dialable(endpoint).expect_err(endpoint);
            assert_eq!(
                refused.kind(),
                TransportErrorKind::Configuration,
                "{endpoint}"
            );
            let said = refused.to_string();
            assert!(said.contains(&format!("`{port}` is not a port")), "{said}");
        }
    }

    #[test]
    fn a_colon_with_no_port_after_it_is_refused_as_such() {
        for endpoint in ["http://h:", "http://[::1]:"] {
            let refused = dialable(endpoint).expect_err(endpoint);
            assert_eq!(
                refused.kind(),
                TransportErrorKind::Configuration,
                "{endpoint}"
            );
            let said = refused.to_string();
            assert!(said.contains("and no port"), "{said}");
        }
    }

    #[test]
    fn text_after_a_bracketed_host_that_is_not_a_port_is_refused() {
        for endpoint in ["http://[::1]8080", "http://[::1]x:80"] {
            let refused = dialable(endpoint).expect_err(endpoint);
            assert_eq!(
                refused.kind(),
                TransportErrorKind::Configuration,
                "{endpoint}"
            );
        }
    }

    #[test]
    fn a_port_that_is_not_digits_is_refused_and_not_quoted() {
        for (endpoint, text) in [("http://h:+80", "+80"), ("http://alice:s3cret", "s3cret")] {
            let refused = dialable(endpoint).expect_err(endpoint);
            assert_eq!(
                refused.kind(),
                TransportErrorKind::Configuration,
                "{endpoint}"
            );
            let said = refused.to_string();
            assert!(!said.contains(text), "{said}");
        }
    }

    #[test]
    fn an_authority_with_no_host_is_refused() {
        assert!(dialable("http://:80").is_err());
    }

    #[test]
    fn a_path_is_refused_because_the_method_replaces_it() {
        assert!(dialable("http://h:1/prefix").is_err());
    }

    #[test]
    fn a_refusal_reads_as_one_sentence() {
        // A string continued onto the next line without its `\` carries the indentation.
        let mut timeless = TransportConfig::new("http://h:1".parse().expect("a uri"));
        timeless.connect_timeout = Duration::ZERO;
        let refusals = [
            dialable("http://alice:s3cret@h:1").expect_err("credentials"),
            dialable("http://h:1/prefix").expect_err("a path"),
            timeless.dialable().expect_err("a timeout of zero"),
        ];
        for refused in refusals {
            let said = refused.to_string();
            assert!(!said.contains("  "), "{said}");
        }
    }

    #[test]
    fn a_connect_timeout_of_zero_is_refused_because_no_dial_could_beat_it() {
        let mut config = TransportConfig::new("http://h:1".parse().expect("a uri"));
        config.connect_timeout = Duration::ZERO;
        assert!(config.dialable().is_err());
    }
}
