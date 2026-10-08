//! A `hyper` connector built from a channel's transport configuration, for HTTP requests other
//! than gRPC to the server a channel reaches, in cleartext or over TLS as the endpoint's scheme
//! says.

use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use snafu::{IntoError, Snafu};

use crate::http2::TransportConfig;
use crate::tls::{Refused, Trust};
use crate::utils::safe_endpoint;

/// Build a hyper connector, TCP then TLS or mTLS as the endpoint's scheme says, from the
/// transport configuration a channel uses. Hidden: its return type names this crate's
/// dependencies; `pub` only so the signature is expressible.
#[doc(hidden)]
pub async fn https_connector(
    transport: TransportConfig,
) -> Result<HttpsConnector<HttpConnector>, ConnectionError> {
    let endpoint = transport.endpoint;
    let tls = transport.tls;

    let trust = if tls.accept_any_server {
        Trust::Anything
    } else if !tls.roots.is_empty() {
        Trust::Roots(tls.roots)
    } else {
        Trust::System
    };
    let identity = tls.identity.map(|identity| (identity.chain, identity.key));
    let tls_config =
        crate::tls::client_config(trust, identity).map_err(|refused| match refused {
            Refused::SystemRoots(source) => IoSnafu {}.into_error(source),
            Refused::Protocols(source) | Refused::Root(source) | Refused::Identity(source) => {
                TlsSnafu {
                    endpoint: safe_endpoint(&endpoint),
                }
                .into_error(source)
            }
        })?;

    // Configure the connector to use http or https depending on the URI scheme
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http();

    let mut http = HttpConnector::new();
    http.enforce_http(false); // required for hyper-rustls to switch schemes
    http.set_nodelay(true);
    http.set_keepalive(transport.tcp.keepalive);
    http.set_keepalive_interval(transport.tcp.keepalive_interval);
    http.set_keepalive_retries(transport.tcp.keepalive_retries);
    http.set_connect_timeout(Some(transport.connect_timeout));

    Ok(https.enable_http1().enable_http2().wrap_connector(http))
}

/// Everything that can go wrong between a transport configuration and a connector.
#[derive(Debug, Snafu)]
#[non_exhaustive]
pub enum ConnectionError {
    #[snafu(display("Could not establish TLS connection to the remote {endpoint} [{location}]"))]
    #[non_exhaustive]
    Tls {
        endpoint: String,
        #[snafu(source(from(rustls::Error, Box::new)))]
        source: Box<rustls::Error>,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("Could not read system cert store [{location}]"))]
    #[non_exhaustive]
    Io {
        #[snafu(source(from(std::io::Error, Box::new)))]
        source: Box<std::io::Error>,
        #[snafu(implicit)]
        location: snafu::Location,
    },
}
