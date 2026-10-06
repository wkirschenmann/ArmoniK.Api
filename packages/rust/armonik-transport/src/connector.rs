//! A `hyper` connector built from a [`ClientConfig`], for HTTP requests other than gRPC to the
//! server a channel reaches, in cleartext or over TLS as the endpoint's scheme says.

use hyper_rustls::{FixedServerNameResolver, HttpsConnector};
use hyper_util::client::legacy::connect::HttpConnector;
use snafu::{IntoError, ResultExt, Snafu};

use crate::config::{override_server_name, ConfigError};
use crate::tls::{Refused, Trust};
use crate::utils::safe_endpoint;
use crate::ClientConfig;

/// Build a hyper connector, TCP then TLS or mTLS as the endpoint's scheme says, from the
/// configuration a channel uses. Hidden: its return type names this crate's dependencies; `pub`
/// only so the signature is expressible.
#[doc(hidden)]
pub async fn https_connector(
    config: ClientConfig,
) -> Result<HttpsConnector<HttpConnector>, ConnectionError> {
    let endpoint = config.endpoint;

    let trust = if config.allow_unsafe_connection {
        Trust::Anything
    } else if !config.cacert.is_empty() {
        Trust::Roots(config.cacert)
    } else {
        Trust::System
    };
    let tls_config =
        crate::tls::client_config(trust, config.identity).map_err(|refused| match refused {
            Refused::SystemRoots(source) => IoSnafu {}.into_error(source),
            Refused::Protocols(source) | Refused::Root(source) | Refused::Identity(source) => {
                TlsSnafu {
                    endpoint: safe_endpoint(&endpoint),
                }
                .into_error(source)
            }
        })?;

    // Configure the connector to use http or https depending on the URI scheme
    let mut https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http();

    if let Some(hostname) = &config.override_target {
        let server_name = override_server_name(hostname).context(ConfigSnafu {})?;
        https = https.with_server_name_resolver(FixedServerNameResolver::new(server_name));
    };

    let mut http = HttpConnector::new();
    http.enforce_http(false); // required for hyper-rustls to switch schemes
    http.set_nodelay(!config.tcp_nagle_algorithm);
    http.set_keepalive(config.tcp_keepalive);
    http.set_keepalive_interval(config.tcp_keepalive_interval);
    http.set_keepalive_retries(config.tcp_keepalive_retries);
    if let Some(timeout) = config.connect_timeout {
        http.set_connect_timeout(Some(timeout));
    }

    Ok(https.enable_http1().enable_http2().wrap_connector(http))
}

/// Everything that can go wrong between a [`ClientConfig`] and a connector.
#[derive(Debug, Snafu)]
#[non_exhaustive]
pub enum ConnectionError {
    #[snafu(display("Could not read the client config [{location}]"))]
    #[non_exhaustive]
    Config {
        #[snafu(source(from(ConfigError, Box::new)))]
        source: Box<ConfigError>,
        #[snafu(implicit)]
        location: snafu::Location,
    },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClientConfigArgs;

    /// A configuration whose only interesting part is the override target. Unsafe connections so that
    /// building the connector reads no certificate store.
    fn config(override_target_name: &str) -> ClientConfig {
        ClientConfig::from_config_args(ClientConfigArgs {
            endpoint: String::from("https://10.0.0.1:5003"),
            override_target_name: String::from(override_target_name),
            allow_unsafe_connection: true,
            ..Default::default()
        })
        .expect("the override target should be a valid authority")
    }

    #[tokio::test]
    async fn a_bracketed_ipv6_override_builds_a_connector() {
        // The whole path, since the name is only pinned onto the connector at the end of it.
        https_connector(config("[::1]"))
            .await
            .expect("a bracketed IPv6 override is a valid one");
    }

    #[tokio::test]
    async fn an_override_that_names_nothing_verifiable_fails_rather_than_panics() {
        let error = https_connector(config("-nope-"))
            .await
            .expect_err("the connector cannot be built without a name to verify against");

        assert!(matches!(error, ConnectionError::Config { .. }), "{error:?}");
    }
}
