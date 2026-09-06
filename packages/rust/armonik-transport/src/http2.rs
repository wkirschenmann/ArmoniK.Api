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

use crate::utils::{chain, safe_endpoint};

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

    /// Everything this connector will not dial, refused before it can be dialled.
    ///
    /// No message prints the endpoint whole. What is refused here includes an endpoint carrying a
    /// password, and a `Configuration` error is rendered into the status a caller reads and into
    /// the log, so printing the thing being refused is how the secret would travel.
    fn dialable(&self) -> Result<(), TransportError> {
        let refuse = |message: String| ConfigurationSnafu { message }.fail();

        match self.endpoint.scheme_str() {
            Some("http") => {}
            Some(other) => {
                return refuse(format!(
                    "`{other}://` is not a scheme this connector dials; it speaks plain HTTP"
                ))
            }
            None => {
                return refuse(
                    "the endpoint names no scheme; it has to be an `http://` URI".to_owned(),
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
                "the endpoint carries `user:password@`, which HTTP/2 forbids in `:authority` and                  which this connector would otherwise put on the wire and in its errors"
                    .to_owned(),
            );
        }

        let host = authority.host();
        if host.is_empty() {
            return refuse("the endpoint names no host".to_owned());
        }

        // A port that does not parse leaves `port_u16` empty and the connector dials 80, so a typo
        // reaches the wrong service rather than being refused.
        let port = &authority.as_str()[host.len()..];
        if !port.is_empty() && authority.port_u16().is_none_or(|port| port == 0) {
            return refuse(format!("`{port}` is not a port; it has to be 1 to 65535"));
        }

        // Neither reaches the wire: a call is addressed by its method path, which replaces both.
        if self.endpoint.query().is_some() || !matches!(self.endpoint.path(), "" | "/") {
            return refuse(
                "the endpoint carries a path or a query, and a call is addressed by its method,                  so neither would be sent"
                    .to_owned(),
            );
        }

        if self.connect_timeout.is_zero() {
            return refuse(
                "a `connect_timeout` of zero elapses before a dial can finish, so every call                  would report a timeout"
                    .to_owned(),
            );
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
        let dialling = self.http.call(target.clone());

        Box::pin(async move {
            dialling.await.map_err(|error| {
                ConnectSnafu {
                    endpoint: safe_endpoint(&target),
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
    // Rendered, not held: `TransportConnector` is public and its `Service<Uri>` takes any URI,
    // not only the one `new` validated, so the guarantee at the top of this file has to be in the
    // type rather than in who calls it.
    #[snafu(display("`{endpoint}` could not be reached: {cause}"))]
    Connect { endpoint: String, cause: String },
    #[snafu(display("the HTTP/2 preface could not be written to `{endpoint}`: {cause}"))]
    Http2Handshake { endpoint: String, cause: String },
    #[snafu(display("connecting to `{endpoint}` outlasted {after:?}"))]
    Timeout { endpoint: String, after: Duration },
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
        let refused = dialable("https://h:1").expect_err("not plain HTTP");
        assert_eq!(refused.kind(), TransportErrorKind::Configuration);
        assert!(refused.to_string().contains("https://"), "{refused}");
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
        for endpoint in ["http://h:99999", "http://h:0"] {
            let refused = dialable(endpoint).expect_err(endpoint);
            assert_eq!(
                refused.kind(),
                TransportErrorKind::Configuration,
                "{endpoint}"
            );
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
    fn a_connect_timeout_of_zero_is_refused_because_no_dial_could_beat_it() {
        let mut config = TransportConfig::new("http://h:1".parse().expect("a uri"));
        config.connect_timeout = Duration::ZERO;
        assert!(config.dialable().is_err());
    }
}
