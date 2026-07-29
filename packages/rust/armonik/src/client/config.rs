use std::time::Duration;

use hyper::{http::HeaderValue, Uri};
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use snafu::{ResultExt, Snafu};

/// Where to find the HTTP proxy used to reach the endpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProxySource {
    /// Connect directly, ignoring any proxy configured in the environment.
    ///
    /// This is the default, so that adding proxy support does not change the behaviour of clients
    /// that never asked for it.
    #[default]
    Disabled,
    /// Read the proxy from the `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` environment variables
    /// (their lowercase spellings are accepted too).
    System,
    /// Use this specific proxy.
    Explicit(Uri),
}

/// Configuration of the HTTP proxy used to reach the endpoint.
///
/// Proxying is performed with an HTTP `CONNECT` tunnel, so TLS — including mutual TLS — is
/// negotiated end to end with the real server and the proxy never sees the plaintext.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProxyConfig {
    /// Where to find the proxy.
    pub source: ProxySource,
    /// Username for proxy authentication, empty for none.
    pub username: String,
    /// Password for proxy authentication, empty for none.
    pub password: String,
}

impl ProxyConfig {
    /// Whether a proxy should be used at all.
    pub fn is_enabled(&self) -> bool {
        !matches!(self.source, ProxySource::Disabled)
    }

    /// Credentials to present to the proxy, if any were configured.
    pub fn credentials(&self) -> Option<(&str, &str)> {
        if self.username.is_empty() && self.password.is_empty() {
            None
        } else {
            Some((&self.username, &self.password))
        }
    }
}

/// Policy for automatically replaying failed requests.
///
/// Only requests that are known to be safe to replay are retried: see
/// [`RetryPolicy::may_replay`](crate::client::RetryPolicy::may_replay).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RetryPolicy {
    /// Maximum number of attempts, counting the initial one. Never below 1.
    pub max_attempts: u32,
    /// Delay before the first retry.
    pub initial_backoff: Duration,
    /// Upper bound on the delay between two attempts.
    pub max_backoff: Duration,
    /// Factor applied to the delay after each attempt.
    pub backoff_multiplier: f64,
    /// Status codes that make a request eligible for a retry.
    pub retryable_status_codes: Vec<tonic::Code>,
}

impl Default for RetryPolicy {
    /// The same policy as the one the .NET client installs through its gRPC service config:
    /// 5 attempts, 1s initial backoff capped at 5s, multiplied by 1.5 each time, on
    /// `UNAVAILABLE`, `ABORTED` and `UNKNOWN`.
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(5),
            backoff_multiplier: 1.5,
            retryable_status_codes: vec![
                tonic::Code::Unavailable,
                tonic::Code::Aborted,
                tonic::Code::Unknown,
            ],
        }
    }
}

/// Options for creating a gRPC Client
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ClientConfig {
    /// Endpoint for sending requests
    pub endpoint: Uri,
    /// Allow unsafe connections to the endpoint (without SSL), defaults to false
    pub allow_unsafe_connection: bool,
    /// TLS identity of the client: key + cert
    pub identity: Option<(CertificateDer<'static>, PrivateKeyDer<'static>)>,
    /// CA certificate to authenticate the server
    pub cacert: Option<CertificateDer<'static>>,
    /// Override the endpoint name during SSL verification
    pub override_target: Option<Uri>,
    /// Timeout for establishing a connection to the server, defaults to no timeout
    pub connect_timeout: Option<Duration>,
    /// Timeout for each request, defaults to no timeout
    pub timeout: Option<Duration>,
    /// Rate limit for requests, defaults to no rate limit
    pub rate_limit: Option<(u64, Duration)>,
    /// TCP keepalive duration, defaults to no keepalive
    pub tcp_keepalive: Option<Duration>,
    /// Interval between TCP keepalive probes, defaults to OS default
    pub tcp_keepalive_interval: Option<Duration>,
    /// Number of TCP keepalive retries, defaults to OS default
    pub tcp_keepalive_retries: Option<u32>,
    /// Enable Nagle's algorithm (disable TCP_NODELAY), defaults to false
    pub tcp_nagle_algorithm: bool,
    /// HTTP/2 PING frame interval, defaults to no keepalive
    pub http2_keep_alive_interval: Option<Duration>,
    /// HTTP/2 PING timeout, defaults to no timeout
    pub http2_keep_alive_timeout: Option<Duration>,
    /// Send HTTP/2 keepalive PINGs even when idle, defaults to false
    pub http2_keep_alive_while_idle: bool,
    /// HTTP/2 max header list size in bytes, defaults to no limit
    pub http2_max_header_list_size: Option<u32>,
    /// User-Agent header value sent with each request
    pub user_agent: Option<HeaderValue>,
    /// HTTP proxy used to reach the endpoint, defaults to a direct connection
    pub proxy: ProxyConfig,
    /// Policy for replaying failed requests, defaults to no retry
    pub retry: Option<RetryPolicy>,
    /// Allow the OS to reuse local ports for outgoing connections, defaults to false.
    ///
    /// On Windows this sets `SO_REUSE_UNICASTPORT`, which avoids ephemeral port exhaustion when
    /// many connections are opened in a short window. It has no effect on other platforms.
    pub reuse_ports: bool,
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
            proxy: self.proxy.clone(),
            retry: self.retry.clone(),
            reuse_ports: self.reuse_ports,
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
    /// Override the endpoint name during SSL verification
    #[cfg_attr(feature = "serde", serde(default))]
    pub override_target_name: String,
    /// Timeout for establishing a connection to the server, defaults to no timeout
    #[cfg_attr(feature = "serde", serde(default))]
    pub connect_timeout: String,
    /// Timeout for each request, defaults to no timeout
    #[cfg_attr(feature = "serde", serde(default))]
    pub timeout: String,
    /// Rate limit for requests, defaults to no rate limit
    #[cfg_attr(feature = "serde", serde(default))]
    pub rate_limit: String,
    /// TCP keepalive duration (e.g. `30s`), defaults to no keepalive
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_keepalive: String,
    /// Interval between TCP keepalive probes (e.g. `5s`), defaults to OS default
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_keepalive_interval: String,
    /// Number of TCP keepalive retries, defaults to OS default
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_keepalive_retries: String,
    /// Enable Nagle's algorithm (disable TCP_NODELAY), defaults to false
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_nagle_algorithm: bool,
    /// HTTP/2 PING frame interval (e.g. `20s`), defaults to no keepalive
    #[cfg_attr(feature = "serde", serde(default))]
    pub http2_keep_alive_interval: String,
    /// HTTP/2 PING timeout (e.g. `10s`), defaults to no timeout
    #[cfg_attr(feature = "serde", serde(default))]
    pub http2_keep_alive_timeout: String,
    /// Send HTTP/2 keepalive PINGs even when idle, defaults to false
    #[cfg_attr(feature = "serde", serde(default))]
    pub http2_keep_alive_while_idle: bool,
    /// HTTP/2 max header list size in bytes, defaults to no limit
    #[cfg_attr(feature = "serde", serde(default))]
    pub http2_max_header_list_size: String,
    /// User-Agent header value sent with each request
    #[cfg_attr(feature = "serde", serde(default))]
    pub user_agent: String,
    /// HTTP proxy to reach the endpoint through.
    ///
    /// Empty for a direct connection, `none` to explicitly disable proxying, `system` to read
    /// `HTTPS_PROXY`/`HTTP_PROXY`/`NO_PROXY` from the environment, otherwise the proxy URL.
    /// A URL without a scheme is assumed to be `http`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub proxy: String,
    /// Username for proxy authentication
    #[cfg_attr(feature = "serde", serde(default))]
    pub proxy_username: String,
    /// Password for proxy authentication
    #[cfg_attr(feature = "serde", serde(default))]
    pub proxy_password: String,
    /// Maximum number of attempts per request, counting the initial one.
    ///
    /// Empty or `1` disables retries, which is the default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub max_attempts: String,
    /// Delay before the first retry (e.g. `1s`), defaults to `1s`
    #[cfg_attr(feature = "serde", serde(default))]
    pub initial_backoff: String,
    /// Upper bound on the delay between two attempts (e.g. `5s`), defaults to `5s`
    #[cfg_attr(feature = "serde", serde(default))]
    pub max_backoff: String,
    /// Factor applied to the delay after each attempt, defaults to `1.5`
    #[cfg_attr(feature = "serde", serde(default))]
    pub backoff_multiplier: String,
    /// Comma-separated gRPC status codes that trigger a retry, by name (e.g.
    /// `unavailable,aborted`) or by number. Defaults to `unavailable,aborted,unknown`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub retryable_status_codes: String,
    /// Allow the OS to reuse local ports for outgoing connections, defaults to false
    #[cfg_attr(feature = "serde", serde(default))]
    pub reuse_ports: bool,
}

impl ClientConfigArgs {
    pub fn from_env() -> Result<Self, super::ConfigError> {
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
            proxy: read_env("GrpcClient__Proxy").context(ctx)?,
            proxy_username: read_env("GrpcClient__ProxyUsername").context(ctx)?,
            proxy_password: read_env("GrpcClient__ProxyPassword").context(ctx)?,
            max_attempts: read_env("GrpcClient__MaxAttempts").context(ctx)?,
            initial_backoff: read_env("GrpcClient__InitialBackOff").context(ctx)?,
            max_backoff: read_env("GrpcClient__MaxBackOff").context(ctx)?,
            backoff_multiplier: read_env("GrpcClient__BackoffMultiplier").context(ctx)?,
            retryable_status_codes: read_env("GrpcClient__RetryableStatusCodes").context(ctx)?,
            reuse_ports: read_env_bool("GrpcClient__ReusePorts").context(ctx)?,
        })
    }
}

impl ClientConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_config_args(ClientConfigArgs::from_env()?)
    }
    pub fn from_config_args(args: ClientConfigArgs) -> Result<Self, ConfigError> {
        let _span = tracing::debug_span!(
            "ClientConfig",
            args.endpoint,
            args.cert_pem,
            args.key_pem,
            args.ca_cert,
            args.allow_unsafe_connection,
            args.override_target_name,
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
            args.proxy,
            args.proxy_username,
            args.max_attempts,
            args.initial_backoff,
            args.max_backoff,
            args.backoff_multiplier,
            args.retryable_status_codes,
            args.reuse_ports,
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
            proxy,
            proxy_username,
            proxy_password,
            max_attempts,
            initial_backoff,
            max_backoff,
            backoff_multiplier,
            retryable_status_codes,
            reuse_ports,
        } = args;

        // Read CAcert file
        let cacert = if !cacert_path.is_empty() {
            let cacert_pem = std::fs::read_to_string(cacert_path.clone())
                .context(IoSnafu { path: cacert_path })?;
            Some(CertificateDer::from_pem_slice(cacert_pem.as_bytes()).context(TlsSnafu {})?)
        } else {
            None
        };

        // Read client cert and key files
        let identity = match (cert_path.as_str(), key_path.as_str()) {
            ("", "") => None,
            ("", _) | (_, "") => return IncompatibleOptionsSnafu{msg: format!("`GrpcClient__CertPem={cert_path}` and `GrpcClient__KeyPem={key_path}` must be either both empty or both set")}.fail(),
            (cert_path, key_path) => {
                let cert_pem =
                    std::fs::read_to_string(cert_path).context(IoSnafu { path: cert_path })?;
                let key_pem = std::fs::read(key_path).context(IoSnafu { path: key_path })?;
                let cert = CertificateDer::from_pem_slice(cert_pem.as_bytes()).context(TlsSnafu {})?;
                let key = PrivateKeyDer::from_pem_slice(key_pem.as_slice()).context(TlsSnafu{})?;

                Some((cert, key))
            }
        };

        let endpoint = Uri::try_from(endpoint.clone()).context(UriSnafu { uri: endpoint })?;

        let override_target = if override_target_name.is_empty() {
            None
        } else {
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
                        uri: endpoint.to_string(),
                    })?
                    .into_parts();
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
                        value: connect_timeout,
                    })?
                    .into(),
            )
        };

        // Defaults to no timeout, as documented on the field. Until this option was wired into the
        // transport it defaulted to 60s here without ever being applied, so honouring that value
        // now would have silently capped every request of every existing client.
        let timeout = if timeout.is_empty() {
            None
        } else {
            Some(
                timeout
                    .parse::<humantime::Duration>()
                    .context(InvalidDurationSnafu { value: timeout })?
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
            let duration = parts[1]
                .parse::<humantime::Duration>()
                .context(InvalidDurationSnafu { value: rate_limit })?
                .into();
            Some((limit, duration))
        };

        let tcp_keepalive = if tcp_keepalive.is_empty() {
            None
        } else {
            Some(
                tcp_keepalive
                    .parse::<humantime::Duration>()
                    .context(InvalidDurationSnafu {
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

        let proxy = ProxyConfig {
            source: parse_proxy_source(&proxy)?,
            username: proxy_username,
            password: proxy_password,
        };

        let retry = parse_retry_policy(
            &max_attempts,
            &initial_backoff,
            &max_backoff,
            &backoff_multiplier,
            &retryable_status_codes,
        )?;

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
            proxy,
            retry,
            reuse_ports,
        })
    }
}

/// Interpret the `GrpcClient__Proxy` value.
///
/// Mirrors the .NET client: empty is a direct connection, `none` explicitly disables proxying,
/// `system` reads the environment, anything else is a proxy URL. A URL without a scheme is assumed
/// to be `http`, so `proxy.corp:3128` works as well as `http://proxy.corp:3128`.
fn parse_proxy_source(proxy: &str) -> Result<ProxySource, ConfigError> {
    match proxy {
        "" => Ok(ProxySource::Disabled),
        _ if proxy.eq_ignore_ascii_case("none") => Ok(ProxySource::Disabled),
        _ if proxy.eq_ignore_ascii_case("system") => Ok(ProxySource::System),
        _ => {
            let with_scheme = if proxy.contains("://") {
                proxy.to_owned()
            } else {
                format!("http://{proxy}")
            };
            // Deliberately not reported through `UriSnafu`: its message talks about the endpoint,
            // which would send whoever reads it looking at the wrong option.
            let uri = Uri::try_from(&with_scheme).ok().filter(|uri| {
                uri.authority()
                    .is_some_and(|authority| !authority.host().is_empty())
            });
            match uri {
                Some(uri) => Ok(ProxySource::Explicit(uri)),
                None => IncompatibleOptionsSnafu {
                    msg: format!(
                        "`GrpcClient__Proxy={proxy}` is not a valid proxy URL. Expected `none`, \
                         `system`, or a URL such as `http://proxy.example.com:3128`"
                    ),
                }
                .fail(),
            }
        }
    }
}

/// Interpret the `GrpcClient__MaxAttempts` family of values.
///
/// Retries are opt-in: without an explicit attempt count above 1 there is no policy at all, which
/// keeps the behaviour of clients that never configured one. The remaining values fall back to
/// [`RetryPolicy::default`], which mirrors the .NET service config.
fn parse_retry_policy(
    max_attempts: &str,
    initial_backoff: &str,
    max_backoff: &str,
    backoff_multiplier: &str,
    retryable_status_codes: &str,
) -> Result<Option<RetryPolicy>, ConfigError> {
    if max_attempts.is_empty() {
        return Ok(None);
    }

    let max_attempts = max_attempts.parse::<u32>().context(InvalidIntegerSnafu {
        value: max_attempts.to_owned(),
    })?;

    if max_attempts <= 1 {
        return Ok(None);
    }

    let defaults = RetryPolicy::default();

    let initial_backoff = if initial_backoff.is_empty() {
        defaults.initial_backoff
    } else {
        initial_backoff
            .parse::<humantime::Duration>()
            .context(InvalidDurationSnafu {
                value: initial_backoff.to_owned(),
            })?
            .into()
    };

    let max_backoff = if max_backoff.is_empty() {
        defaults.max_backoff
    } else {
        max_backoff
            .parse::<humantime::Duration>()
            .context(InvalidDurationSnafu {
                value: max_backoff.to_owned(),
            })?
            .into()
    };

    if max_backoff < initial_backoff {
        return IncompatibleOptionsSnafu {
            msg: format!(
                "`GrpcClient__MaxBackOff={}` must not be shorter than \
                 `GrpcClient__InitialBackOff={}`",
                humantime::Duration::from(max_backoff),
                humantime::Duration::from(initial_backoff),
            ),
        }
        .fail();
    }

    let backoff_multiplier = if backoff_multiplier.is_empty() {
        defaults.backoff_multiplier
    } else {
        let parsed = backoff_multiplier
            .parse::<f64>()
            .context(InvalidNumberSnafu {
                value: backoff_multiplier.to_owned(),
            })?;
        if !parsed.is_finite() || parsed < 1.0 {
            return IncompatibleOptionsSnafu {
                msg: format!(
                    "`GrpcClient__BackoffMultiplier={parsed}` must be a finite value of at least 1"
                ),
            }
            .fail();
        }
        parsed
    };

    let retryable_status_codes = if retryable_status_codes.is_empty() {
        defaults.retryable_status_codes
    } else {
        retryable_status_codes
            .split(',')
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .map(parse_status_code)
            .collect::<Result<Vec<_>, _>>()?
    };

    Ok(Some(RetryPolicy {
        max_attempts,
        initial_backoff,
        max_backoff,
        backoff_multiplier,
        retryable_status_codes,
    }))
}

/// Parse a single gRPC status code, by name (`unavailable`, `deadline_exceeded`, …) or by number.
fn parse_status_code(code: &str) -> Result<tonic::Code, ConfigError> {
    if let Ok(number) = code.parse::<i32>() {
        // `Code::from_i32` maps anything unknown onto `Unknown`, which would silently accept
        // nonsense, so reject numbers that do not round-trip.
        let parsed = tonic::Code::from_i32(number);
        if parsed as i32 == number {
            return Ok(parsed);
        }
        return IncompatibleOptionsSnafu {
            msg: format!("`{number}` is not a valid gRPC status code"),
        }
        .fail();
    }

    let normalized = code.replace(['-', '_'], "");
    for candidate in [
        tonic::Code::Ok,
        tonic::Code::Cancelled,
        tonic::Code::Unknown,
        tonic::Code::InvalidArgument,
        tonic::Code::DeadlineExceeded,
        tonic::Code::NotFound,
        tonic::Code::AlreadyExists,
        tonic::Code::PermissionDenied,
        tonic::Code::ResourceExhausted,
        tonic::Code::FailedPrecondition,
        tonic::Code::Aborted,
        tonic::Code::OutOfRange,
        tonic::Code::Unimplemented,
        tonic::Code::Internal,
        tonic::Code::Unavailable,
        tonic::Code::DataLoss,
        tonic::Code::Unauthenticated,
    ] {
        if format!("{candidate:?}").eq_ignore_ascii_case(&normalized) {
            return Ok(candidate);
        }
    }

    IncompatibleOptionsSnafu {
        msg: format!("`{code}` is not a valid gRPC status code name"),
    }
    .fail()
}

impl TryFrom<&ClientConfig> for tonic::transport::Endpoint {
    type Error = ConfigError;

    fn try_from(value: &ClientConfig) -> Result<Self, Self::Error> {
        Ok(Self::from(value.endpoint.clone()))
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
        backtrace: snafu::Backtrace,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("`GrpcClient__ConnectTimeout={value}` is not a valid duration (e.g. `30s` or `1m`) [{location}]"))]
    #[non_exhaustive]
    InvalidDuration {
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
    #[snafu(display("`{value}` is not a valid integer [{location}]"))]
    #[non_exhaustive]
    InvalidInteger {
        source: std::num::ParseIntError,
        value: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    #[snafu(display("`{value}` is not a valid number [{location}]"))]
    #[non_exhaustive]
    InvalidNumber {
        source: std::num::ParseFloatError,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build args with everything empty, so each test only sets what it exercises.
    fn args() -> ClientConfigArgs {
        ClientConfigArgs {
            endpoint: String::from("http://localhost:5001"),
            ..Default::default()
        }
    }

    #[test]
    fn defaults_keep_previous_behaviour() {
        let config = ClientConfig::from_config_args(args()).unwrap();

        // Retries, proxying and port reuse are all opt-in, so a client that configures none of
        // them behaves exactly as it did before these options existed.
        assert_eq!(config.proxy.source, ProxySource::Disabled);
        assert!(!config.proxy.is_enabled());
        assert_eq!(config.retry, None);
        assert!(!config.reuse_ports);

        // The request timeout is documented as "no timeout"; the 60s that used to be parsed here
        // was never applied to the transport.
        assert_eq!(config.timeout, None);
        assert_eq!(config.rate_limit, None);

        // Unlike the request timeout, the connect timeout was always applied.
        assert_eq!(config.connect_timeout, Some(Duration::from_secs(60)));
    }

    #[test]
    fn proxy_none_and_empty_disable_proxying() {
        for value in ["", "none", "None", "NONE"] {
            let config = ClientConfig::from_config_args(ClientConfigArgs {
                proxy: String::from(value),
                ..args()
            })
            .unwrap();
            assert_eq!(
                config.proxy.source,
                ProxySource::Disabled,
                "{value:?} should disable proxying"
            );
        }
    }

    #[test]
    fn proxy_system_reads_the_environment() {
        for value in ["system", "System", "SYSTEM"] {
            let config = ClientConfig::from_config_args(ClientConfigArgs {
                proxy: String::from(value),
                ..args()
            })
            .unwrap();
            assert_eq!(config.proxy.source, ProxySource::System, "{value:?}");
        }
    }

    #[test]
    fn proxy_url_defaults_to_http_scheme() {
        let with_scheme = ClientConfig::from_config_args(ClientConfigArgs {
            proxy: String::from("http://proxy.corp:3128"),
            ..args()
        })
        .unwrap();
        let without_scheme = ClientConfig::from_config_args(ClientConfigArgs {
            proxy: String::from("proxy.corp:3128"),
            ..args()
        })
        .unwrap();

        assert_eq!(with_scheme.proxy.source, without_scheme.proxy.source);
        let ProxySource::Explicit(uri) = with_scheme.proxy.source else {
            panic!("expected an explicit proxy");
        };
        assert_eq!(uri.host(), Some("proxy.corp"));
        assert_eq!(uri.port_u16(), Some(3128));
    }

    #[test]
    fn proxy_credentials_are_optional() {
        let none = ClientConfig::from_config_args(ClientConfigArgs {
            proxy: String::from("proxy.corp:3128"),
            ..args()
        })
        .unwrap();
        assert_eq!(none.proxy.credentials(), None);

        let some = ClientConfig::from_config_args(ClientConfigArgs {
            proxy: String::from("proxy.corp:3128"),
            proxy_username: String::from("user"),
            proxy_password: String::from("secret"),
            ..args()
        })
        .unwrap();
        assert_eq!(some.proxy.credentials(), Some(("user", "secret")));
    }

    #[test]
    fn proxy_without_host_is_rejected() {
        // The message must point at the proxy option; reporting these through the endpoint URI
        // error would send whoever reads it looking at the wrong setting.
        for value in ["http:///no-host", "http://", "http://:3128", "://"] {
            let error = ClientConfig::from_config_args(ClientConfigArgs {
                proxy: String::from(value),
                ..args()
            })
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("is not a valid proxy URL"),
                "unexpected error for {value:?}: {error}"
            );
        }
    }

    #[test]
    fn retry_is_opt_in() {
        // Absent, and an attempt count that leaves no room for a second try, both mean "no retry".
        for value in ["", "0", "1"] {
            let config = ClientConfig::from_config_args(ClientConfigArgs {
                max_attempts: String::from(value),
                ..args()
            })
            .unwrap();
            assert_eq!(
                config.retry, None,
                "MaxAttempts={value:?} should not install a policy"
            );
        }
    }

    #[test]
    fn retry_falls_back_to_dotnet_defaults() {
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            max_attempts: String::from("5"),
            ..args()
        })
        .unwrap();

        let retry = config.retry.expect("a policy");
        assert_eq!(retry.max_attempts, 5);
        assert_eq!(retry.initial_backoff, Duration::from_secs(1));
        assert_eq!(retry.max_backoff, Duration::from_secs(5));
        assert_eq!(retry.backoff_multiplier, 1.5);
        assert_eq!(
            retry.retryable_status_codes,
            vec![
                tonic::Code::Unavailable,
                tonic::Code::Aborted,
                tonic::Code::Unknown
            ]
        );
    }

    #[test]
    fn retry_reads_every_field() {
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            max_attempts: String::from("3"),
            initial_backoff: String::from("250ms"),
            max_backoff: String::from("2s"),
            backoff_multiplier: String::from("2"),
            retryable_status_codes: String::from("deadline_exceeded, INTERNAL ,14"),
            ..args()
        })
        .unwrap();

        let retry = config.retry.expect("a policy");
        assert_eq!(retry.max_attempts, 3);
        assert_eq!(retry.initial_backoff, Duration::from_millis(250));
        assert_eq!(retry.max_backoff, Duration::from_secs(2));
        assert_eq!(retry.backoff_multiplier, 2.0);
        assert_eq!(
            retry.retryable_status_codes,
            vec![
                tonic::Code::DeadlineExceeded,
                tonic::Code::Internal,
                tonic::Code::Unavailable
            ]
        );
    }

    #[test]
    fn retry_status_code_names_are_forgiving() {
        // Names may be spelled with dashes, underscores or nothing at all, in any case.
        for spelling in [
            "deadline_exceeded",
            "deadline-exceeded",
            "DeadlineExceeded",
            "DEADLINEEXCEEDED",
        ] {
            let config = ClientConfig::from_config_args(ClientConfigArgs {
                max_attempts: String::from("2"),
                retryable_status_codes: String::from(spelling),
                ..args()
            })
            .unwrap();
            assert_eq!(
                config.retry.expect("a policy").retryable_status_codes,
                vec![tonic::Code::DeadlineExceeded],
                "{spelling:?} should parse"
            );
        }
    }

    #[test]
    fn retry_rejects_unknown_status_codes() {
        // A typo must not silently degrade into `Unknown`, which is itself a retryable code.
        for value in ["unavailabel", "999"] {
            let error = ClientConfig::from_config_args(ClientConfigArgs {
                max_attempts: String::from("2"),
                retryable_status_codes: String::from(value),
                ..args()
            })
            .unwrap_err();
            assert!(
                error.to_string().contains("not a valid gRPC status code"),
                "unexpected error for {value:?}: {error}"
            );
        }
    }

    #[test]
    fn retry_rejects_incoherent_backoffs() {
        let error = ClientConfig::from_config_args(ClientConfigArgs {
            max_attempts: String::from("3"),
            initial_backoff: String::from("10s"),
            max_backoff: String::from("1s"),
            ..args()
        })
        .unwrap_err();
        assert!(
            error.to_string().contains("must not be shorter than"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn retry_rejects_shrinking_multipliers() {
        for value in ["0.5", "0", "-1"] {
            let error = ClientConfig::from_config_args(ClientConfigArgs {
                max_attempts: String::from("3"),
                backoff_multiplier: String::from(value),
                ..args()
            })
            .unwrap_err();
            assert!(
                error.to_string().contains("at least 1"),
                "unexpected error for {value:?}: {error}"
            );
        }
    }

    #[test]
    fn timeout_and_rate_limit_are_parsed() {
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            timeout: String::from("30s"),
            rate_limit: String::from("100/1s"),
            ..args()
        })
        .unwrap();

        assert_eq!(config.timeout, Some(Duration::from_secs(30)));
        assert_eq!(config.rate_limit, Some((100, Duration::from_secs(1))));
    }

    #[test]
    fn reuse_ports_round_trips() {
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            reuse_ports: true,
            ..args()
        })
        .unwrap();
        assert!(config.reuse_ports);
    }

    #[test]
    fn clone_carries_every_field() {
        // `ClientConfig` implements `Clone` by hand because private keys are not `Clone`, so a new
        // field is easy to forget there. Comparing the debug rendering catches that structurally.
        let config = ClientConfig::from_config_args(ClientConfigArgs {
            proxy: String::from("proxy.corp:3128"),
            proxy_username: String::from("user"),
            proxy_password: String::from("secret"),
            max_attempts: String::from("4"),
            timeout: String::from("30s"),
            rate_limit: String::from("100/1s"),
            reuse_ports: true,
            ..args()
        })
        .unwrap();

        assert_eq!(format!("{config:?}"), format!("{:?}", config.clone()));
    }
}
