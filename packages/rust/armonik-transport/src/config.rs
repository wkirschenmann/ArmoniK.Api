use std::time::Duration;

use hyper::{http::HeaderValue, Uri};
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName};
use snafu::{ResultExt, Snafu};

use crate::grpc::GrpcChannelConfig;
use crate::http2::{ClientIdentity, TransportConfig};

/// Options for creating a gRPC Client
///
/// The Rust client's configuration, read from its `GrpcClient__*` variables and turned into the
/// engine's by [`ClientConfig::channel_config`]. The engine behind the C ABI is configured by
/// [`crate::options::ChannelOptions`] instead.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ClientConfig {
    /// Endpoint for sending requests
    pub endpoint: Uri,
    /// Allow unsafe connections to the endpoint (without SSL), defaults to false
    pub allow_unsafe_connection: bool,
    /// TLS identity of the client: the chain from `GrpcClient__CertPem`, then its key
    ///
    /// A chain and not one certificate, because a PEM file holds what the server needs to build a
    /// path: the leaf, then the intermediates that sign it. Sending the leaf alone fails against
    /// any server that does not already hold them.
    pub identity: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
    /// CA certificates to authenticate the server, from `GrpcClient__CaCert`
    ///
    /// Every certificate in the file, because a bundle during a root rotation holds two, and
    /// taking the first would refuse the half of the fleet signed by the other.
    pub cacert: Vec<CertificateDer<'static>>,
    /// Override the endpoint name during SSL verification: the name the server certificate is
    /// verified against, and nothing else - requests keep the endpoint's `:authority`
    pub override_target: Option<Uri>,
    /// Timeout for establishing a connection to the server, defaults to 60s
    pub connect_timeout: Option<Duration>,
    /// Deadline of each call, counted from its start and streaming calls included, defaults to
    /// no deadline
    pub timeout: Option<Duration>,
    /// Rate limit for requests; refused by [`ClientConfig::channel_config`]
    pub rate_limit: Option<(u64, Duration)>,
    /// TCP keepalive duration, defaults to no keepalive
    pub tcp_keepalive: Option<Duration>,
    /// Interval between TCP keepalive probes, defaults to OS default; read only with
    /// `tcp_keepalive`
    pub tcp_keepalive_interval: Option<Duration>,
    /// Number of TCP keepalive retries, defaults to OS default; read only with `tcp_keepalive`
    pub tcp_keepalive_retries: Option<u32>,
    /// Enable Nagle's algorithm (disable TCP_NODELAY), defaults to false; true is refused by
    /// [`ClientConfig::channel_config`]
    pub tcp_nagle_algorithm: bool,
    /// HTTP/2 PING frame interval, defaults to no keepalive
    pub http2_keep_alive_interval: Option<Duration>,
    /// HTTP/2 PING timeout, defaults to 20s
    pub http2_keep_alive_timeout: Option<Duration>,
    /// Send HTTP/2 keepalive PINGs even when idle, defaults to false
    pub http2_keep_alive_while_idle: bool,
    /// HTTP/2 max header list size in bytes; refused by [`ClientConfig::channel_config`]
    pub http2_max_header_list_size: Option<u32>,
    /// User-Agent header value sent with each request
    pub user_agent: Option<HeaderValue>,
}

impl Clone for ClientConfig {
    fn clone(&self) -> Self {
        Self {
            endpoint: self.endpoint.clone(),
            allow_unsafe_connection: self.allow_unsafe_connection,
            identity: self
                .identity
                .as_ref()
                .map(|(cert, key)| (cert.clone(), key.clone_key())),
            cacert: self.cacert.clone(),
            override_target: self.override_target.clone(),
            connect_timeout: self.connect_timeout,
            timeout: self.timeout,
            rate_limit: self.rate_limit,
            tcp_keepalive: self.tcp_keepalive,
            tcp_keepalive_interval: self.tcp_keepalive_interval,
            tcp_keepalive_retries: self.tcp_keepalive_retries,
            tcp_nagle_algorithm: self.tcp_nagle_algorithm,
            http2_keep_alive_interval: self.http2_keep_alive_interval,
            http2_keep_alive_timeout: self.http2_keep_alive_timeout,
            http2_keep_alive_while_idle: self.http2_keep_alive_while_idle,
            http2_max_header_list_size: self.http2_max_header_list_size,
            user_agent: self.user_agent.clone(),
        }
    }
}

/// Options for creating a gRPC Client (as given in the environment)
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ClientConfigArgs {
    /// Endpoint for sending requests
    pub endpoint: String,
    /// Path to the certificate file in pem format
    #[cfg_attr(feature = "serde", serde(default))]
    pub cert_pem: String,
    /// Path to the key file in pem format
    #[cfg_attr(feature = "serde", serde(default))]
    pub key_pem: String,
    /// Path to the Certificate Authority file in pem format
    #[cfg_attr(feature = "serde", serde(default))]
    pub ca_cert: String,
    /// Allow unsafe connections to the endpoint (without SSL), defaults to false
    #[cfg_attr(feature = "serde", serde(default))]
    pub allow_unsafe_connection: bool,
    /// Override the endpoint name during SSL verification; requests keep the endpoint's authority
    #[cfg_attr(feature = "serde", serde(default))]
    pub override_target_name: String,
    /// Timeout for establishing a connection to the server, defaults to 60s
    #[cfg_attr(feature = "serde", serde(default))]
    pub connect_timeout: String,
    /// Deadline of each call, streaming calls included, defaults to no deadline
    #[cfg_attr(feature = "serde", serde(default))]
    pub timeout: String,
    /// Rate limit for requests; refused by [`ClientConfig::channel_config`]
    #[cfg_attr(feature = "serde", serde(default))]
    pub rate_limit: String,
    /// TCP keepalive duration (e.g. `30s`), defaults to no keepalive
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_keepalive: String,
    /// Interval between TCP keepalive probes (e.g. `5s`), defaults to OS default; read only with
    /// `tcp_keepalive`
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_keepalive_interval: String,
    /// Number of TCP keepalive retries, defaults to OS default; read only with `tcp_keepalive`
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_keepalive_retries: String,
    /// Enable Nagle's algorithm (disable TCP_NODELAY), defaults to false; true is refused by
    /// [`ClientConfig::channel_config`]
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_nagle_algorithm: bool,
    /// HTTP/2 PING frame interval (e.g. `20s`), defaults to no keepalive
    #[cfg_attr(feature = "serde", serde(default))]
    pub http2_keep_alive_interval: String,
    /// HTTP/2 PING timeout (e.g. `10s`), defaults to 20s
    #[cfg_attr(feature = "serde", serde(default))]
    pub http2_keep_alive_timeout: String,
    /// Send HTTP/2 keepalive PINGs even when idle, defaults to false
    #[cfg_attr(feature = "serde", serde(default))]
    pub http2_keep_alive_while_idle: bool,
    /// HTTP/2 max header list size in bytes; refused by [`ClientConfig::channel_config`]
    #[cfg_attr(feature = "serde", serde(default))]
    pub http2_max_header_list_size: String,
    /// User-Agent header value sent with each request
    #[cfg_attr(feature = "serde", serde(default))]
    pub user_agent: String,
}

impl ClientConfigArgs {
    pub fn from_env() -> Result<Self, ConfigError> {
        use crate::utils::{read_env, read_env_bool};
        let ctx = EnvSnafu {};
        Ok(Self {
            endpoint: read_env("GrpcClient__Endpoint").context(ctx)?,
            cert_pem: read_env("GrpcClient__CertPem").context(ctx)?,
            key_pem: read_env("GrpcClient__KeyPem").context(ctx)?,
            ca_cert: read_env("GrpcClient__CaCert").context(ctx)?,
            allow_unsafe_connection: read_env_bool("GrpcClient__AllowUnsafeConnection")
                .context(ctx)?,
            override_target_name: read_env("GrpcClient__OverrideTargetName").context(ctx)?,
            connect_timeout: read_env("GrpcClient__ConnectTimeout").context(ctx)?,
            timeout: read_env("GrpcClient__Timeout").context(ctx)?,
            rate_limit: read_env("GrpcClient__RateLimit").context(ctx)?,
            tcp_keepalive: read_env("GrpcClient__TcpKeepalive").context(ctx)?,
            tcp_keepalive_interval: read_env("GrpcClient__TcpKeepaliveInterval").context(ctx)?,
            tcp_keepalive_retries: read_env("GrpcClient__TcpKeepaliveRetries").context(ctx)?,
            tcp_nagle_algorithm: read_env_bool("GrpcClient__TcpNagleAlgorithm").context(ctx)?,
            http2_keep_alive_interval: read_env("GrpcClient__Http2KeepAliveInterval")
                .context(ctx)?,
            http2_keep_alive_timeout: read_env("GrpcClient__Http2KeepAliveTimeout").context(ctx)?,
            http2_keep_alive_while_idle: read_env_bool("GrpcClient__Http2KeepAliveWhileIdle")
                .context(ctx)?,
            http2_max_header_list_size: read_env("GrpcClient__Http2MaxHeaderListSize")
                .context(ctx)?,
            user_agent: read_env("GrpcClient__UserAgent").context(ctx)?,
        })
    }
}

impl ClientConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_config_args(ClientConfigArgs::from_env()?)
    }
    pub fn from_config_args(args: ClientConfigArgs) -> Result<Self, ConfigError> {
        // Neither the endpoint nor the override target is in the span, and neither are the two
        // PEM paths' contents: both fields are refused below for carrying `user:password@`, and
        // the span is built before that check, so it would record what the check keeps out.
        let _span = tracing::debug_span!(
            "ClientConfig",
            args.cert_pem,
            args.key_pem,
            args.ca_cert,
            args.allow_unsafe_connection,
            args.connect_timeout,
            args.timeout,
            args.rate_limit,
            args.tcp_keepalive,
            args.tcp_keepalive_interval,
            args.tcp_keepalive_retries,
            args.tcp_nagle_algorithm,
            args.http2_keep_alive_interval,
            args.http2_keep_alive_timeout,
            args.http2_keep_alive_while_idle,
            args.http2_max_header_list_size,
            args.user_agent,
        );

        let ClientConfigArgs {
            endpoint,
            cert_pem: cert_path,
            key_pem: key_path,
            ca_cert: cacert_path,
            allow_unsafe_connection,
            override_target_name,
            connect_timeout,
            timeout,
            rate_limit,
            tcp_keepalive,
            tcp_keepalive_interval,
            tcp_keepalive_retries,
            tcp_nagle_algorithm,
            http2_keep_alive_interval,
            http2_keep_alive_timeout,
            http2_keep_alive_while_idle,
            http2_max_header_list_size,
            user_agent,
        } = args;

        // Read CAcert file
        let cacert = if cacert_path.is_empty() {
            Vec::new()
        } else {
            // As bytes, like the key below: a PEM file is base64 in ASCII armour, but nothing
            // says the text around it is UTF-8, and a preamble byte that is not would otherwise
            // surface as "could not read file".
            let cacert_pem =
                std::fs::read(cacert_path.clone()).context(IoSnafu { path: &cacert_path })?;
            let cacert = CertificateDer::pem_slice_iter(&cacert_pem)
                .collect::<Result<Vec<_>, _>>()
                .context(TlsSnafu {})?;
            if cacert.is_empty() {
                return HoldsNoCertificateSnafu {
                    name: "GrpcClient__CaCert",
                    path: cacert_path,
                }
                .fail();
            }
            cacert
        };

        // Read client cert and key files
        let identity = match (cert_path.as_str(), key_path.as_str()) {
            ("", "") => None,
            ("", _) | (_, "") => return IncompatibleOptionsSnafu{msg: format!("`GrpcClient__CertPem={cert_path}` and `GrpcClient__KeyPem={key_path}` must be either both empty or both set")}.fail(),
            (cert_path, key_path) => {
                let cert_pem = std::fs::read(cert_path).context(IoSnafu { path: cert_path })?;
                let key_pem = std::fs::read(key_path).context(IoSnafu { path: key_path })?;
                let chain = CertificateDer::pem_slice_iter(&cert_pem)
                    .collect::<Result<Vec<_>, _>>()
                    .context(TlsSnafu {})?;
                if chain.is_empty() {
                    return HoldsNoCertificateSnafu {
                        name: "GrpcClient__CertPem",
                        path: cert_path.to_owned(),
                    }
                    .fail();
                }
                let key = PrivateKeyDer::from_pem_slice(key_pem.as_slice()).context(TlsSnafu{})?;

                Some((chain, key))
            }
        };

        // Before it is parsed, so nothing below splits a string that still holds a password:
        // HTTP/2 forbids userinfo in `:authority`, this crate's own connector refuses such an
        // endpoint, and an error message is not where a caller should learn it was there.
        if endpoint.contains('@') {
            return CarriesUserinfoSnafu {
                name: "GrpcClient__Endpoint",
            }
            .fail();
        }

        let endpoint = Uri::try_from(endpoint.clone()).context(UriSnafu {
            uri: endpoint.clone(),
        })?;

        let override_target = if override_target_name.is_empty() {
            None
        } else {
            if override_target_name.contains('@') {
                return CarriesUserinfoSnafu {
                    name: "GrpcClient__OverrideTargetName",
                }
                .fail();
            }

            let authority;
            let path_and_query;

            if let Ok(auth) = override_target_name.parse::<hyper::http::uri::Authority>() {
                authority = Some(auth);
                path_and_query = endpoint.path_and_query().cloned();
            } else {
                hyper::http::uri::Parts {
                    authority,
                    path_and_query,
                    ..
                } = Uri::try_from(override_target_name.clone())
                    .context(UriSnafu {
                        uri: override_target_name.clone(),
                    })?
                    .into_parts();
            }

            // An override with no authority overrides nothing: the name to verify against comes
            // from the endpoint, and the caller who set this would never be told it had no
            // effect.
            if authority.is_none() {
                return NamesNoAuthoritySnafu {
                    value: override_target_name,
                }
                .fail();
            }

            let mut uri = hyper::http::uri::Builder::new();

            if let Some(scheme) = endpoint.scheme() {
                uri = uri.scheme(scheme.clone());
            }
            if let Some(authority) = authority.or_else(|| endpoint.authority().cloned()) {
                uri = uri.authority(authority);
            }
            if let Some(path_and_query) = path_and_query {
                uri = uri.path_and_query(path_and_query);
            }

            Some(uri.build().context(HttpSnafu {
                uri: override_target_name,
            })?)
        };

        let connect_timeout = if connect_timeout.is_empty() {
            Some(Duration::from_secs(60))
        } else {
            Some(
                connect_timeout
                    .parse::<humantime::Duration>()
                    .context(InvalidDurationSnafu {
                        name: "GrpcClient__ConnectTimeout",
                        value: connect_timeout,
                    })?
                    .into(),
            )
        };

        let timeout = if timeout.is_empty() {
            None
        } else {
            Some(
                timeout
                    .parse::<humantime::Duration>()
                    .context(InvalidDurationSnafu {
                        name: "GrpcClient__Timeout",
                        value: timeout,
                    })?
                    .into(),
            )
        };

        let rate_limit = if rate_limit.is_empty() {
            None
        } else {
            let parts: Vec<&str> = rate_limit.split('/').collect();
            if parts.len() != 2 {
                return IncompatibleOptionsSnafu {
                    msg: format!("Rate limit should be in the format `number/duration`, e.g. `100/1s`, but got `{rate_limit}`"),
                }.fail();
            }
            let limit = parts[0]
                .parse::<u64>()
                .context(InvalidRateLimitCountSnafu {
                    value: parts[0].to_string(),
                })?;
            let duration: Duration = parts[1]
                .parse::<humantime::Duration>()
                .context(InvalidDurationSnafu {
                    name: "GrpcClient__RateLimit",
                    value: rate_limit.clone(),
                })?
                .into();
            // A zero count or duration is no rate at all: a mistyped option, refused here by name.
            if limit == 0 || duration.is_zero() {
                return IncompatibleOptionsSnafu {
                    msg: format!(
                        "`GrpcClient__RateLimit={rate_limit}` has a zero count or duration. Both have \
                         to be above zero, as in `100/1s`; leave it empty for no rate limit"
                    ),
                }
                .fail();
            }
            Some((limit, duration))
        };

        let tcp_keepalive = if tcp_keepalive.is_empty() {
            None
        } else {
            Some(
                tcp_keepalive
                    .parse::<humantime::Duration>()
                    .context(InvalidDurationSnafu {
                        name: "GrpcClient__TcpKeepalive",
                        value: tcp_keepalive,
                    })?
                    .into(),
            )
        };

        let tcp_keepalive_interval = if tcp_keepalive_interval.is_empty() {
            None
        } else {
            Some(
                tcp_keepalive_interval
                    .parse::<humantime::Duration>()
                    .context(InvalidDurationSnafu {
                        name: "GrpcClient__TcpKeepaliveInterval",
                        value: tcp_keepalive_interval,
                    })?
                    .into(),
            )
        };

        let tcp_keepalive_retries = if tcp_keepalive_retries.is_empty() {
            None
        } else {
            Some(
                tcp_keepalive_retries
                    .parse::<u32>()
                    .context(InvalidIntegerSnafu {
                        name: "GrpcClient__TcpKeepaliveRetries",
                        value: tcp_keepalive_retries,
                    })?,
            )
        };

        let http2_keep_alive_interval = if http2_keep_alive_interval.is_empty() {
            None
        } else {
            Some(
                http2_keep_alive_interval
                    .parse::<humantime::Duration>()
                    .context(InvalidDurationSnafu {
                        name: "GrpcClient__Http2KeepAliveInterval",
                        value: http2_keep_alive_interval,
                    })?
                    .into(),
            )
        };

        let http2_keep_alive_timeout = if http2_keep_alive_timeout.is_empty() {
            None
        } else {
            Some(
                http2_keep_alive_timeout
                    .parse::<humantime::Duration>()
                    .context(InvalidDurationSnafu {
                        name: "GrpcClient__Http2KeepAliveTimeout",
                        value: http2_keep_alive_timeout,
                    })?
                    .into(),
            )
        };

        let http2_max_header_list_size = if http2_max_header_list_size.is_empty() {
            None
        } else {
            Some(
                http2_max_header_list_size
                    .parse::<u32>()
                    .context(InvalidIntegerSnafu {
                        name: "GrpcClient__Http2MaxHeaderListSize",
                        value: http2_max_header_list_size,
                    })?,
            )
        };

        let user_agent = if user_agent.is_empty() {
            None
        } else {
            let header = HeaderValue::from_str(&user_agent)
                .context(InvalidUserAgentSnafu { value: user_agent })?;
            Some(header)
        };

        Ok(Self {
            endpoint,
            allow_unsafe_connection,
            identity,
            cacert,
            override_target,
            connect_timeout,
            timeout,
            rate_limit,
            tcp_keepalive,
            tcp_keepalive_interval,
            tcp_keepalive_retries,
            tcp_nagle_algorithm,
            http2_keep_alive_interval,
            http2_keep_alive_timeout,
            http2_keep_alive_while_idle,
            http2_max_header_list_size,
            user_agent,
        })
    }

    /// The engine's channel configuration for these options.
    ///
    /// TLS options apply to an `https://` endpoint only; an `http://` one is plaintext whatever
    /// they hold. Refused: what the engine has not got - a rate limit, a bound on the header list -
    /// and Nagle's algorithm, which the engine always disables, so that none is read and then
    /// ignored.
    pub fn channel_config(self) -> Result<GrpcChannelConfig, ConfigError> {
        if self.rate_limit.is_some() {
            return NotBuiltSnafu {
                name: "GrpcClient__RateLimit",
            }
            .fail();
        }
        if self.http2_max_header_list_size.is_some() {
            return NotBuiltSnafu {
                name: "GrpcClient__Http2MaxHeaderListSize",
            }
            .fail();
        }
        if self.tcp_nagle_algorithm {
            return NotBuiltSnafu {
                name: "GrpcClient__TcpNagleAlgorithm",
            }
            .fail();
        }

        // Checked whatever the scheme, so that a mistyped name is refused before it matters.
        if let Some(target) = &self.override_target {
            override_server_name(target)?;
        }

        let mut transport = TransportConfig::new(self.endpoint);
        if let Some(timeout) = self.connect_timeout {
            transport.connect_timeout = timeout;
        }
        if transport.endpoint.scheme() == Some(&hyper::http::uri::Scheme::HTTPS) {
            // Accepting any server makes the roots moot, and the engine refuses the two together.
            transport.tls.accept_any_server = self.allow_unsafe_connection;
            if !self.allow_unsafe_connection {
                transport.tls.roots = self.cacert;
            }
            transport.tls.identity = self
                .identity
                .map(|(chain, key)| ClientIdentity { chain, key });
            transport.tls.server_name = self
                .override_target
                .as_ref()
                .and_then(Uri::host)
                .map(str::to_owned);
        }
        // The probes' interval and count tune a keepalive, and the engine refuses them without one.
        if self.tcp_keepalive.is_some() {
            transport.tcp.keepalive = self.tcp_keepalive;
            transport.tcp.keepalive_interval = self.tcp_keepalive_interval;
            transport.tcp.keepalive_retries = self.tcp_keepalive_retries;
        }
        transport.http2.keep_alive_interval = self.http2_keep_alive_interval;
        if let Some(timeout) = self.http2_keep_alive_timeout {
            transport.http2.keep_alive_timeout = timeout;
        }
        transport.http2.keep_alive_while_idle = self.http2_keep_alive_while_idle;

        let mut config = GrpcChannelConfig::new(transport);
        config.default_deadline = self.timeout;
        // Text as `ClientConfigArgs` built it; a value built from bytes that are not UTF-8 is read
        // as near as text can say it, rather than taking the process down.
        config.user_agent = self
            .user_agent
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned());
        Ok(config)
    }
}

/// The name the server certificate is verified against, from the host of an override target.
///
/// A host that is neither a DNS name nor an IP address is a mistyped
/// `GrpcClient__OverrideTargetName`, reported as the configuration error it is: this runs inside a
/// library, where a panic leaves the caller nothing to read.
pub(crate) fn override_server_name(target: &Uri) -> Result<ServerName<'static>, ConfigError> {
    let host = target.host().unwrap_or_default();

    match crate::tls::server_name(host) {
        Some(server_name) => Ok(server_name),
        None => IncompatibleOptionsSnafu {
            msg: format!(
                "`GrpcClient__OverrideTargetName` names the host `{host}`, which no certificate can \
                 be verified against. It has to be a DNS name or an IP address, as in \
                 `server.example.com`, `10.0.0.1` or `[::1]`"
            ),
        }
        .fail(),
    }
}

#[derive(Debug, Snafu)]
#[non_exhaustive]
pub enum ConfigError {
    #[snafu(display("Could not read environment variable [{location}]"))]
    #[non_exhaustive]
    Env {
        #[snafu(source(from(crate::utils::ReadEnvError, Box::new)))]
        source: Box<crate::utils::ReadEnvError>,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("Invalid TLS configuration [{location}]"))]
    #[non_exhaustive]
    Tls {
        #[snafu(source(from(rustls::pki_types::pem::Error, Box::new)))]
        source: Box<rustls::pki_types::pem::Error>,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("Endpoint URI is not valid: `{uri}` [{location}]"))]
    #[non_exhaustive]
    Uri {
        #[snafu(source(from(hyper::http::uri::InvalidUri, Box::new)))]
        source: Box<hyper::http::uri::InvalidUri>,
        uri: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("Override URI is not valid: `{uri}` [{location}]"))]
    #[non_exhaustive]
    Http {
        #[snafu(source(from(hyper::http::Error, Box::new)))]
        source: Box<hyper::http::Error>,
        uri: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("Could not read file `{path}` [{location}]"))]
    #[non_exhaustive]
    Io {
        #[snafu(source(from(std::io::Error, Box::new)))]
        source: Box<std::io::Error>,
        path: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("{msg} [{location}]"))]
    #[non_exhaustive]
    IncompatibleOptions {
        msg: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display(
        "`{name}={value}` is not a valid duration (e.g. `30s` or `1m`) [{location}]"
    ))]
    #[non_exhaustive]
    InvalidDuration {
        name: &'static str,
        source: humantime::DurationError,
        value: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("Rate limit count `{value}` is not a valid integer [{location}]"))]
    #[non_exhaustive]
    InvalidRateLimitCount {
        source: std::num::ParseIntError,
        value: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("`{name}={path}` holds no certificate [{location}]"))]
    #[non_exhaustive]
    HoldsNoCertificate {
        name: &'static str,
        path: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("`{name}` carries `user:password@`, which HTTP/2 forbids in `:authority` and which this client would put in its errors and on the wire [{location}]"))]
    #[non_exhaustive]
    CarriesUserinfo {
        name: &'static str,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("`GrpcClient__OverrideTargetName={value}` names no authority, so it would override nothing [{location}]"))]
    #[non_exhaustive]
    NamesNoAuthority {
        value: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("`{name}={value}` is not a valid integer [{location}]"))]
    #[non_exhaustive]
    InvalidInteger {
        name: &'static str,
        source: std::num::ParseIntError,
        value: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("Invalid user agent `{value}` [{location}]"))]
    #[non_exhaustive]
    InvalidUserAgent {
        source: hyper::http::header::InvalidHeaderValue,
        value: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("`{name}` is not supported by this client: unset it [{location}]"))]
    #[non_exhaustive]
    NotBuilt {
        name: &'static str,
        #[snafu(implicit)]
        location: snafu::Location,
    },
}

#[cfg(test)]
mod tests {
    use rustls::pki_types::IpAddr;

    use super::*;

    /// The minimum viable arguments: an endpoint, and nothing else set.
    fn args() -> ClientConfigArgs {
        ClientConfigArgs {
            endpoint: String::from("http://localhost:5001"),
            ..Default::default()
        }
    }

    /// Every message in the chain, joined. snafu keeps the detail in the source, so asserting on the
    /// outermost `Display` alone would pass whatever the cause turned out to be.
    fn chain(error: &ConfigError) -> String {
        crate::utils::chain(error, " | ")
    }

    fn override_target(override_target_name: &str) -> Uri {
        ClientConfig::from_config_args(ClientConfigArgs {
            override_target_name: String::from(override_target_name),
            ..args()
        })
        .expect("a valid authority")
        .override_target
        .expect("an override target")
    }

    /// The name derived from an override target, for a value that yields one.
    fn server_name(override_target_name: &str) -> ServerName<'static> {
        override_server_name(&override_target(override_target_name))
            .expect("the host should name something verifiable")
    }

    /// The password never reaches a message, a span, or the wire.
    ///
    /// Refused rather than scrubbed: HTTP/2 forbids userinfo in `:authority`, so an endpoint
    /// carrying one is a configuration that cannot work, and telling the caller that is more use
    /// than dialling something they did not ask for.
    #[test]
    fn an_endpoint_that_carries_a_password_is_refused_and_the_password_is_not_repeated() {
        let error = ClientConfig::from_config_args(ClientConfigArgs {
            endpoint: String::from("https://alice:s3cret@host:5001"),
            ..args()
        })
        .expect_err("userinfo is not dialable");

        let said = chain(&error);
        assert!(said.contains("GrpcClient__Endpoint"), "{said}");
        assert!(!said.contains("s3cret"), "the message repeats it: {said}");
        assert!(!said.contains("alice"), "the message repeats it: {said}");
    }

    /// The same promise for the override target, which is an authority and carries a password as
    /// readily as the endpoint does.
    ///
    /// Read out of the source, because what is asserted is a field's absence and a subscriber
    /// only ever sees the fields that are there. The span is built before any validation, so this
    /// is the one place where the two guards below cannot help.
    #[test]
    fn no_field_the_userinfo_guard_refuses_is_recorded_in_the_span() {
        let source = include_str!("config.rs");
        let opened = source
            .find("\"ClientConfig\",")
            .expect("the span is built in this file");
        let closed = source[opened..]
            .find(");")
            .expect("the span's arguments are a call");
        let span = &source[opened..opened + closed];

        for field in ["endpoint", "override_target_name"] {
            assert!(
                !span.contains(field),
                "`{field}` is refused below for carrying `user:password@`, and this span records \
                 it before that check: {span}"
            );
        }
    }

    #[test]
    fn an_override_that_carries_a_password_is_refused_and_the_password_is_not_repeated() {
        let error = ClientConfig::from_config_args(ClientConfigArgs {
            override_target_name: String::from("alice:s3cret@other:5001"),
            ..args()
        })
        .expect_err("userinfo is not an authority");

        let said = chain(&error);
        assert!(said.contains("GrpcClient__OverrideTargetName"), "{said}");
        assert!(!said.contains("s3cret"), "the message repeats it: {said}");
    }

    #[test]
    fn an_override_that_names_no_authority_is_refused_rather_than_ignored() {
        let error = ClientConfig::from_config_args(ClientConfigArgs {
            override_target_name: String::from("/other"),
            ..args()
        })
        .expect_err("a path overrides no name");

        assert!(
            matches!(error, ConfigError::NamesNoAuthority { .. }),
            "{error:?}"
        );
    }

    /// An override that cannot be parsed names itself, not the endpoint - which is the one value
    /// the caller got right.
    #[test]
    fn an_override_that_cannot_be_parsed_names_the_override() {
        let error = ClientConfig::from_config_args(ClientConfigArgs {
            override_target_name: String::from("not a uri at all"),
            ..args()
        })
        .expect_err("a space is not a uri");

        let said = chain(&error);
        assert!(said.contains("not a uri at all"), "{said}");
        assert!(!said.contains("localhost:5001"), "{said}");
    }

    #[test]
    fn the_minimum_is_an_endpoint() {
        let config = ClientConfig::from_config_args(args()).expect("an endpoint is enough");

        assert_eq!(config.endpoint.to_string(), "http://localhost:5001/");
        assert!(config.identity.is_none());
        assert!(config.cacert.is_empty());
        assert_eq!(config.override_target, None);
        assert_eq!(config.rate_limit, None);
    }

    #[test]
    fn an_endpoint_that_is_not_a_uri_is_reported() {
        let error = ClientConfig::from_config_args(ClientConfigArgs {
            endpoint: String::new(),
            ..args()
        })
        .expect_err("an empty endpoint is not a URI");

        assert!(matches!(error, ConfigError::Uri { .. }), "{error:?}");
    }

    // --- what the engine's channel is given ---

    #[test]
    fn what_the_engine_has_not_got_is_refused_by_name() {
        let refused = [
            (
                "GrpcClient__RateLimit",
                ClientConfigArgs {
                    rate_limit: String::from("100/1s"),
                    ..args()
                },
            ),
            (
                "GrpcClient__Http2MaxHeaderListSize",
                ClientConfigArgs {
                    http2_max_header_list_size: String::from("16384"),
                    ..args()
                },
            ),
            (
                "GrpcClient__TcpNagleAlgorithm",
                ClientConfigArgs {
                    tcp_nagle_algorithm: true,
                    ..args()
                },
            ),
        ];
        for (name, args) in refused {
            let config = ClientConfig::from_config_args(args).expect("a valid value");
            let error = config
                .channel_config()
                .expect_err("not something the engine has");

            assert!(matches!(error, ConfigError::NotBuilt { .. }), "{error:?}");
            assert!(error.to_string().contains(name), "{error}");
        }
    }

    #[test]
    fn tls_options_reach_an_https_endpoint_only() {
        let tls = |endpoint: &str| ClientConfigArgs {
            endpoint: String::from(endpoint),
            allow_unsafe_connection: true,
            override_target_name: String::from("other:5001"),
            ..args()
        };

        let plain = ClientConfig::from_config_args(tls("http://localhost:5001"))
            .expect("valid")
            .channel_config()
            .expect("built");
        assert!(!plain.transport.tls.accept_any_server);
        assert_eq!(plain.transport.tls.server_name, None);

        let secure = ClientConfig::from_config_args(tls("https://localhost:5001"))
            .expect("valid")
            .channel_config()
            .expect("built");
        assert!(secure.transport.tls.accept_any_server);
        assert_eq!(secure.transport.tls.server_name.as_deref(), Some("other"));
    }

    #[test]
    fn an_override_written_as_a_bracketed_ipv6_literal_names_the_address() {
        // `[::1]` is how an IPv6 host is written in an authority, and `http` hands the brackets back
        // with it. The name a certificate is checked against is the address inside them.
        assert_eq!(
            server_name("[::1]"),
            ServerName::from(IpAddr::try_from("::1").expect("an address")),
        );
        assert_eq!(
            server_name("[2001:db8::1]:5003"),
            ServerName::from(IpAddr::try_from("2001:db8::1").expect("an address")),
        );
    }

    #[test]
    fn an_override_written_as_a_dns_name_or_an_ipv4_address_is_taken_as_it_stands() {
        assert_eq!(
            server_name("server.example.com"),
            ServerName::try_from("server.example.com").expect("a name"),
        );
        assert_eq!(
            server_name("10.0.0.1:5003"),
            ServerName::from(IpAddr::try_from("10.0.0.1").expect("an address")),
        );
    }

    #[test]
    fn brackets_around_something_that_is_not_an_address_are_not_read_as_a_name() {
        // `http` balances the brackets without looking inside them, so `[example.com]` reaches here.
        // Dropping the brackets and taking what is left would verify against a host nobody wrote.
        let error = override_server_name(&override_target("[example.com]"))
            .expect_err("brackets are an IP literal or nothing");
        assert!(
            matches!(error, ConfigError::IncompatibleOptions { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_host_that_names_nothing_verifiable_is_refused_against_its_option() {
        // Whoever set it has a dozen `GrpcClient__*` variables to choose from, so the message has to
        // name the one at fault and quote what it read.
        for endpoint in ["https://10.0.0.1:5003", "http://10.0.0.1:5003"] {
            let error = ClientConfig::from_config_args(ClientConfigArgs {
                endpoint: String::from(endpoint),
                override_target_name: String::from("-nope-"),
                ..args()
            })
            .expect("a valid authority")
            .channel_config()
            .expect_err("no certificate can be verified against it");

            let rendered = chain(&error);
            assert!(
                rendered.contains("GrpcClient__OverrideTargetName"),
                "{rendered}"
            );
            assert!(rendered.contains("-nope-"), "{rendered}");
        }
    }

    #[test]
    fn an_empty_timeout_means_no_timeout_rather_than_a_minute() {
        // What keeps a one-minute deadline off every call of every caller who set nothing.
        let config = ClientConfig::from_config_args(args()).expect("valid");

        assert_eq!(config.timeout, None);
        assert_eq!(
            config.channel_config().expect("built").default_deadline,
            None
        );
    }

    #[test]
    fn an_empty_connect_timeout_still_means_a_minute() {
        // The mirror image of the test above: an absent `ConnectTimeout` bounds the connection at a
        // minute, which is what a caller who sets nothing gets.
        let config = ClientConfig::from_config_args(args()).expect("valid");

        assert_eq!(config.connect_timeout, Some(Duration::from_secs(60)));
    }

    #[test]
    fn a_timeout_is_parsed_in_the_units_it_was_written_in() {
        for (written, expected) in [
            ("30s", Duration::from_secs(30)),
            ("500ms", Duration::from_millis(500)),
            ("2m", Duration::from_secs(120)),
        ] {
            let config = ClientConfig::from_config_args(ClientConfigArgs {
                timeout: String::from(written),
                ..args()
            })
            .expect("valid");
            assert_eq!(config.timeout, Some(expected), "for {written}");
        }
    }

    #[tokio::test]
    async fn keepalive_probes_with_no_keepalive_are_not_read() {
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            tcp_keepalive_interval: String::from("5s"),
            tcp_keepalive_retries: String::from("3"),
            ..args()
        })
        .expect("valid")
        .channel_config()
        .expect("built");

        assert_eq!(config.transport.tcp.keepalive_interval, None);
        crate::grpc::GrpcChannel::new(config, tokio::runtime::Handle::current())
            .expect("accepted by the engine");
    }

    #[tokio::test]
    async fn accepting_any_server_takes_precedence_over_the_roots() {
        let mut config = ClientConfig::from_config_args(ClientConfigArgs {
            endpoint: String::from("https://localhost:5001"),
            allow_unsafe_connection: true,
            ..args()
        })
        .expect("valid");
        config.cacert = vec![CertificateDer::from(vec![0u8])];

        let config = config.channel_config().expect("built");
        assert!(config.transport.tls.accept_any_server);
        assert!(config.transport.tls.roots.is_empty());
        crate::grpc::GrpcChannel::new(config, tokio::runtime::Handle::current())
            .expect("accepted by the engine");
    }

    #[test]
    fn the_request_timeout_is_the_channels_default_deadline() {
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            timeout: String::from("300ms"),
            user_agent: String::from("armonik-test"),
            ..args()
        })
        .expect("valid")
        .channel_config()
        .expect("built");

        assert_eq!(config.default_deadline, Some(Duration::from_millis(300)));
        assert_eq!(config.user_agent.as_deref(), Some("armonik-test"));
    }

    // --- durations and numbers ---

    #[test]
    fn durations_are_read_in_the_units_they_are_written_in() {
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            connect_timeout: String::from("500ms"),
            tcp_keepalive: String::from("30s"),
            tcp_keepalive_interval: String::from("2m"),
            http2_keep_alive_interval: String::from("1h"),
            ..args()
        })
        .expect("valid durations");

        assert_eq!(config.connect_timeout, Some(Duration::from_millis(500)));
        assert_eq!(config.tcp_keepalive, Some(Duration::from_secs(30)));
        assert_eq!(
            config.tcp_keepalive_interval,
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            config.http2_keep_alive_interval,
            Some(Duration::from_secs(3600))
        );
    }

    #[test]
    fn a_duration_that_cannot_be_parsed_names_the_variable_and_the_value() {
        let error = ClientConfig::from_config_args(ClientConfigArgs {
            tcp_keepalive: String::from("soon"),
            ..args()
        })
        .expect_err("`soon` is not a duration");

        assert!(
            matches!(error, ConfigError::InvalidDuration { .. }),
            "{error:?}"
        );
        // The variable, because a dozen of them are durations and the value alone leaves the
        // reader to guess which one they mistyped.
        assert!(
            chain(&error).contains("GrpcClient__TcpKeepalive=soon"),
            "{}",
            chain(&error)
        );
    }

    #[test]
    fn integers_are_read_and_a_bad_one_names_the_value() {
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            tcp_keepalive_retries: String::from("3"),
            http2_max_header_list_size: String::from("16384"),
            ..args()
        })
        .expect("valid integers");
        assert_eq!(config.tcp_keepalive_retries, Some(3));
        assert_eq!(config.http2_max_header_list_size, Some(16384));

        let error = ClientConfig::from_config_args(ClientConfigArgs {
            tcp_keepalive_retries: String::from("many"),
            ..args()
        })
        .expect_err("`many` is not an integer");
        assert!(
            matches!(error, ConfigError::InvalidInteger { .. }),
            "{error:?}"
        );
        assert!(chain(&error).contains("many"), "{}", chain(&error));
    }

    #[test]
    fn an_integer_that_does_not_fit_is_rejected_rather_than_wrapped() {
        // These are `u32`; a value past the top must fail rather than silently become something else.
        let error = ClientConfig::from_config_args(ClientConfigArgs {
            http2_max_header_list_size: String::from("4294967296"),
            ..args()
        })
        .expect_err("2^32 does not fit in a u32");

        assert!(
            matches!(error, ConfigError::InvalidInteger { .. }),
            "{error:?}"
        );
    }

    // --- rate limit ---

    #[test]
    fn a_rate_limit_is_a_count_and_a_duration() {
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            rate_limit: String::from("100/1s"),
            ..args()
        })
        .expect("valid");

        assert_eq!(config.rate_limit, Some((100, Duration::from_secs(1))));
    }

    #[test]
    fn a_zero_rate_limit_is_rejected_rather_than_left_to_panic() {
        // A zero count or duration is no rate at all, and the message says which option it is.
        for value in ["0/1s", "1/0s", "0/0s"] {
            let error = ClientConfig::from_config_args(ClientConfigArgs {
                rate_limit: String::from(value),
                ..args()
            })
            .expect_err("a zero rate limit must be rejected")
            .to_string();

            assert!(error.contains("zero count or duration"), "{value}: {error}");
            assert!(
                error.contains(value),
                "the message should quote it: {error}"
            );
        }
    }

    #[test]
    fn a_rate_limit_missing_its_duration_is_reported_with_the_expected_shape() {
        // The message has to show the format, since `100` on its own looks perfectly reasonable to whoever
        // wrote it.
        let error = ClientConfig::from_config_args(ClientConfigArgs {
            rate_limit: String::from("100"),
            ..args()
        })
        .expect_err("a rate limit needs both halves");

        assert!(
            matches!(error, ConfigError::IncompatibleOptions { .. }),
            "{error:?}"
        );
        let rendered = chain(&error);
        assert!(rendered.contains("number/duration"), "{rendered}");
        assert!(rendered.contains("100"), "{rendered}");
    }

    #[test]
    fn each_half_of_a_rate_limit_is_validated_separately() {
        let count = ClientConfig::from_config_args(ClientConfigArgs {
            rate_limit: String::from("plenty/1s"),
            ..args()
        })
        .expect_err("`plenty` is not a count");
        assert!(
            matches!(count, ConfigError::InvalidRateLimitCount { .. }),
            "{count:?}"
        );

        let duration = ClientConfig::from_config_args(ClientConfigArgs {
            rate_limit: String::from("100/soon"),
            ..args()
        })
        .expect_err("`soon` is not a duration");
        assert!(
            matches!(duration, ConfigError::InvalidDuration { .. }),
            "{duration:?}"
        );
    }

    // --- certificates ---

    #[test]
    fn half_an_identity_is_rejected_and_names_both_variables() {
        // Half an identity is silent on a plain-TLS endpoint and only surfaces as a rejected handshake
        // on an mTLS one. Neither path is read from disk before the check, so this needs no fixture.
        for (cert, key) in [("cert.pem", ""), ("", "key.pem")] {
            let error = ClientConfig::from_config_args(ClientConfigArgs {
                cert_pem: String::from(cert),
                key_pem: String::from(key),
                ..args()
            })
            .expect_err("half an identity must be rejected");

            assert!(
                matches!(error, ConfigError::IncompatibleOptions { .. }),
                "{error:?}"
            );
            let rendered = chain(&error);
            assert!(rendered.contains("GrpcClient__CertPem"), "{rendered}");
            assert!(rendered.contains("GrpcClient__KeyPem"), "{rendered}");
        }
    }

    #[test]
    fn neither_half_is_no_identity_rather_than_an_error() {
        let config = ClientConfig::from_config_args(args()).expect("valid");
        assert!(config.identity.is_none());
    }

    #[test]
    fn a_certificate_path_that_does_not_exist_is_reported_with_the_path() {
        // These options are paths, not contents. A typo in one has to name the file rather than surface
        // later as a TLS failure.
        let error = ClientConfig::from_config_args(ClientConfigArgs {
            cert_pem: String::from("no/such/cert.pem"),
            key_pem: String::from("no/such/key.pem"),
            ..args()
        })
        .expect_err("a missing file must be reported");

        assert!(matches!(error, ConfigError::Io { .. }), "{error:?}");
        assert!(
            chain(&error).contains("no/such/cert.pem"),
            "{}",
            chain(&error)
        );
    }

    #[test]
    fn a_missing_ca_certificate_is_reported_with_the_path() {
        let error = ClientConfig::from_config_args(ClientConfigArgs {
            ca_cert: String::from("no/such/ca.pem"),
            ..args()
        })
        .expect_err("a missing file must be reported");

        assert!(matches!(error, ConfigError::Io { .. }), "{error:?}");
        assert!(
            chain(&error).contains("no/such/ca.pem"),
            "{}",
            chain(&error)
        );
    }

    // --- override target ---

    #[test]
    fn an_override_target_given_as_a_host_keeps_the_endpoints_scheme_and_path() {
        // The common case: the certificate names one host, the endpoint is reached at another. Only the
        // authority is being overridden, so everything else has to come from the endpoint.
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            endpoint: String::from("https://10.0.0.1:5003/base"),
            override_target_name: String::from("server.example.com"),
            ..args()
        })
        .expect("valid");

        let override_target = config.override_target.expect("an override target");
        assert_eq!(override_target.scheme_str(), Some("https"));
        assert_eq!(
            override_target.authority().map(|a| a.as_str()),
            Some("server.example.com")
        );
        assert_eq!(override_target.path(), "/base");
    }

    #[test]
    fn an_override_target_given_as_a_uri_replaces_the_authority_and_the_path() {
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            endpoint: String::from("https://10.0.0.1:5003/base"),
            override_target_name: String::from("https://server.example.com/other"),
            ..args()
        })
        .expect("valid");

        let override_target = config.override_target.expect("an override target");
        assert_eq!(
            override_target.authority().map(|a| a.as_str()),
            Some("server.example.com")
        );
        assert_eq!(override_target.path(), "/other");
        // The scheme still comes from the endpoint: the connection is made to the endpoint, and this only
        // changes the name it is verified against.
        assert_eq!(override_target.scheme_str(), Some("https"));
    }

    #[test]
    fn no_override_target_leaves_it_unset() {
        let config = ClientConfig::from_config_args(args()).expect("valid");
        assert_eq!(config.override_target, None);
    }

    // --- the serde feature ---

    #[cfg(feature = "serde")]
    #[test]
    fn arguments_round_trip_through_serde_with_absent_fields_defaulted() {
        // Every field but the endpoint carries `serde(default)`, so a configuration file need only name
        // what it changes. The feature is off by default, so nothing else here would notice it breaking.
        let deserialised: ClientConfigArgs =
            serde_json::from_str(r#"{"endpoint":"http://localhost:5001","timeout":"30s"}"#)
                .expect("absent fields should default");

        assert_eq!(deserialised.endpoint, "http://localhost:5001");
        assert_eq!(deserialised.timeout, "30s");
        assert_eq!(deserialised.cert_pem, "", "an absent field defaults");
        assert!(!deserialised.allow_unsafe_connection);

        let round_tripped: ClientConfigArgs =
            serde_json::from_str(&serde_json::to_string(&deserialised).expect("serialise"))
                .expect("deserialise");
        assert_eq!(round_tripped, deserialised);
    }
}
