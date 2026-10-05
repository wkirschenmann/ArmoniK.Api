use std::fmt;
use std::time::Duration;

use armonik_transport::grpc::{GrpcChannelConfig, RetryConfig};
use armonik_transport::http2::{Http2Config, ProxyConfig, TcpConfig, TlsConfig, TransportConfig};
use armonik_transport::options::{
    ChannelOptions, OptionRefusal, ProxyOptions, Seconds, LARGEST_WINDOW,
};
use armonik_transport::reexports::http::Uri;

// What a configuration that names neither gets. One send, the smallest window. Four deliveries:
// the head and the message of a unary call each take one, and a stream has the rest.
const MAX_SENDS_IN_FLIGHT: i32 = 1;
const DELIVERY_CREDITS: i32 = 4;

/// The options a host sent, read and found admissible.
///
/// Each unit is held as the engine configuration it became rather than as what was written:
/// converting once, where the document is refused, is what leaves nothing here that can fail -
/// and the files the TLS unit names are read there, once.
pub(crate) struct ChannelSettings {
    options: ChannelOptions,
    connect_timeout: Option<Duration>,
    default_deadline: Option<Duration>,
    tls: TlsConfig,
    tcp: TcpConfig,
    http2: Http2Config,
    proxy: ProxyConfig,
    retry: RetryConfig,
}

impl ChannelSettings {
    pub(crate) fn delivery_credits(&self) -> usize {
        self.options.delivery_credits.unwrap_or(DELIVERY_CREDITS) as usize
    }

    pub(crate) fn connect_eagerly(&self) -> bool {
        self.options.connect_eagerly.unwrap_or(false)
    }

    pub(crate) fn max_sends_in_flight(&self) -> usize {
        self.options
            .max_sends_in_flight
            .unwrap_or(MAX_SENDS_IN_FLIGHT) as usize
    }

    pub(crate) fn into_channel_config(self, endpoint: Uri) -> GrpcChannelConfig {
        let max_sends_in_flight = self.max_sends_in_flight();
        let mut transport = TransportConfig::new(endpoint);
        if let Some(connect_timeout) = self.connect_timeout {
            transport.connect_timeout = connect_timeout;
        }
        transport.tls = self.tls;
        transport.tcp = self.tcp;
        transport.http2 = self.http2;
        transport.proxy = self.proxy;

        let mut config = GrpcChannelConfig::new(transport);
        config.max_sends_in_flight = max_sends_in_flight;
        config.user_agent = self.options.user_agent;
        if let Some(max) = self.options.max_receive_message_size {
            config.max_recv_message_size = max as usize;
        }
        if let Some(bytes) = self.options.delivery_coalescing_bytes {
            config.delivery_coalescing = bytes as usize;
        }
        config.default_deadline = self.default_deadline;
        config.retry = Some(self.retry);
        config
    }
}

/// Why a document was refused, named by the key it was refused over.
#[derive(Debug)]
pub(crate) enum ConfigRefusal {
    /// Not a document of this vocabulary: not JSON, a key it does not have, or a value of the
    /// wrong type, with the path of the key it was refused at.
    Document(serde_path_to_error::Error<serde_json::Error>),
    /// A window outside what the schema admits.
    Window { key: &'static str, value: i32 },
    /// A receive limit that admits no message at all.
    NoMessage { value: i32 },
    /// A count of bytes below zero.
    Bytes { key: &'static str, value: i32 },
    /// An empty user agent.
    EmptyUserAgent,
    /// A duration no `Duration` holds, or one below what it holds.
    Seconds { key: &'static str, seconds: f64 },
    /// An option of a unit the engine converts, a file it names included.
    Option(OptionRefusal),
    /// A refusal of the runtime's channel defaults, read alone.
    Defaults(Box<ConfigRefusal>),
    /// A refusal of a channel's options merged over the runtime's channel defaults, which the
    /// channel's options alone do not earn.
    Merged(Box<ConfigRefusal>),
}

impl fmt::Display for ConfigRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The path is `.` for the document itself, which a syntax error is refused at.
            Self::Document(error) => match error.path().to_string().as_str() {
                "." => write!(
                    f,
                    "the configuration document is refused: {}",
                    error.inner()
                ),
                path => write!(f, "{path} is refused: {}", error.inner()),
            },
            Self::Window { key, value } => write!(
                f,
                "{key} is {value}, and has to be between 1 and {LARGEST_WINDOW}"
            ),
            Self::NoMessage { value } => write!(
                f,
                "MaxReceiveMessageSize is {value}, and has to be at least 1 - a channel that \
                 receives no message at all"
            ),
            Self::Bytes { key, value } => write!(f, "{key} is {value}, and has to be at least 0"),
            Self::EmptyUserAgent => f.write_str("UserAgent is empty, and has to name something"),
            Self::Seconds { key, seconds } => write!(
                f,
                "{key} is {seconds}, and has to be at least 1e-9 and less than 2^64"
            ),
            Self::Option(refused) => refused.fmt(f),
            Self::Defaults(refused) => write!(f, "ChannelDefaults: {refused}"),
            Self::Merged(refused) => {
                write!(
                    f,
                    "{refused}, once merged over the runtime's ChannelDefaults"
                )
            }
        }
    }
}

impl std::error::Error for ConfigRefusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // Not the refusal Defaults and Merged wrap: their text says it already, and a source is
        // said again after it.
        match self {
            Self::Document(error) => Some(error.inner()),
            _ => None,
        }
    }
}

/// Reads a runtime's channel defaults: a channel document, refused as a channel's own is, and
/// kept as the options each channel's own are merged over. Empty is none.
pub(crate) fn defaults(json: &[u8]) -> Result<Option<ChannelOptions>, ConfigRefusal> {
    if json.is_empty() {
        return Ok(None);
    }
    let refused = |refused| ConfigRefusal::Defaults(Box::new(refused));
    let options = read(json).map_err(refused)?;
    settle(options.clone()).map_err(refused)?;
    Ok(Some(options))
}

/// Reads a channel's document over the runtime's defaults, as `ChannelOptions::over` merges
/// them. A refusal the channel's document earns alone is the one reported, in its own terms;
/// only one it does not is the merge's.
pub(crate) fn parse_over(
    defaults: Option<&ChannelOptions>,
    json: &[u8],
) -> Result<ChannelSettings, ConfigRefusal> {
    let own = read(json)?;
    let Some(defaults) = defaults else {
        return settle(own);
    };
    settle(own.clone().over(defaults)).map_err(|merged| match settle(own) {
        Err(alone) => alone,
        Ok(_) => ConfigRefusal::Merged(Box::new(merged)),
    })
}

/// A document read alone, as a channel with no runtime defaults reads it.
#[cfg(test)]
pub(crate) fn parse(json: &[u8]) -> Result<ChannelSettings, ConfigRefusal> {
    settle(read(json)?)
}

fn read(json: &[u8]) -> Result<ChannelOptions, ConfigRefusal> {
    serde_path_to_error::deserialize(&mut serde_json::Deserializer::from_slice(json))
        .map_err(ConfigRefusal::Document)
}

/// Settles the options, refusing exactly what the schema refuses, and saying over which key.
///
/// Every bound checked here is stated in the schema the options type derives - a minimum, a
/// maximum, a minimum length - so a document a validator would reject is one this refuses too.
/// They are checked again rather than trusted: nothing obliges a host to have validated, and the
/// engine is what a bad value would break.
fn settle(options: ChannelOptions) -> Result<ChannelSettings, ConfigRefusal> {
    let window = |key: &'static str, asked: Option<i32>| match asked {
        Some(value) if !(1..=LARGEST_WINDOW).contains(&value) => {
            Err(ConfigRefusal::Window { key, value })
        }
        _ => Ok(()),
    };
    window("DeliveryCredits", options.delivery_credits)?;
    window("MaxSendsInFlight", options.max_sends_in_flight)?;

    // Zero is refused: it is a channel that can receive no message at all.
    if let Some(value) = options.max_receive_message_size.filter(|max| *max < 1) {
        return Err(ConfigRefusal::NoMessage { value });
    }

    if let Some(value) = options.delivery_coalescing_bytes.filter(|bytes| *bytes < 0) {
        return Err(ConfigRefusal::Bytes {
            key: "DeliveryCoalescingBytes",
            value,
        });
    }

    if options.user_agent.as_deref().is_some_and(str::is_empty) {
        return Err(ConfigRefusal::EmptyUserAgent);
    }

    // Below a nanosecond is refused, as the schema's `minimum` refuses it: `Duration` holds
    // nothing finer, so the conversion could round it to zero, which no dial or call could beat.
    // And a number is not yet a duration: `Duration` holds no value past its own range either, so
    // the conversion is what says whether the document named one.
    let duration = |key: &'static str, asked: Option<Seconds>| match asked {
        None => Ok(None),
        Some(seconds) => {
            let refused = ConfigRefusal::Seconds {
                key,
                seconds: seconds.0,
            };
            if seconds.0 < 1e-9 {
                return Err(refused);
            }
            Duration::try_from(seconds).map(Some).map_err(|_| refused)
        }
    };
    let connect_timeout = duration(
        "Transport.ConnectTimeoutSeconds",
        options.transport.connect_timeout_seconds,
    )?;
    let default_deadline = duration("DefaultDeadlineSeconds", options.default_deadline_seconds)?;

    let tls = options
        .transport
        .tls
        .load()
        .map_err(|refused| ConfigRefusal::Option(refused.under("Transport.Tls")))?;
    let tcp = options
        .transport
        .tcp_keepalive
        .to_config()
        .map_err(|refused| ConfigRefusal::Option(refused.under("Transport.TcpKeepalive")))?;
    let http2 = options
        .http2
        .to_config()
        .map_err(|refused| ConfigRefusal::Option(refused.under("Http2")))?;
    let proxy = options
        .transport
        .proxy
        .as_ref()
        .map_or_else(
            || ProxyOptions::default().to_config(),
            ProxyOptions::to_config,
        )
        .map_err(|refused| ConfigRefusal::Option(refused.under("Transport.Proxy")))?;
    let retry = options
        .retry
        .to_config()
        .map_err(|refused| ConfigRefusal::Option(refused.under("Retry")))?;

    Ok(ChannelSettings {
        options,
        connect_timeout,
        default_deadline,
        tls,
        tcp,
        http2,
        proxy,
        retry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_of(json: &[u8]) -> GrpcChannelConfig {
        let settings = parse(json).expect("valid");
        settings.into_channel_config("http://127.0.0.1:5000".parse().expect("an endpoint"))
    }

    /// A default is stated in words, in its option's description, and applied here. This is
    /// what says the words give the number the reader applies.
    #[test]
    fn every_default_the_schema_states_is_the_one_the_reader_applies() {
        let schema: serde_json::Value = serde_json::from_str(&armonik_transport::options::schema())
            .expect("the schema renders as JSON");
        let stated = |pointer: &str| -> f64 {
            let description = schema
                .pointer(pointer)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_else(|| panic!("{pointer} names no description"));
            let (_, after) = description
                .split_once("Defaults to ")
                .unwrap_or_else(|| panic!("{pointer} states no default: {description}"));
            let number: String = after
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            number
                .trim_end_matches('.')
                .parse()
                .unwrap_or_else(|_| panic!("{pointer} states no number: {after}"))
        };

        let settings = parse(b"{}").expect("an empty document is valid");
        assert_eq!(
            stated("/properties/DeliveryCredits/description"),
            settings.delivery_credits() as f64
        );
        assert_eq!(
            stated("/properties/MaxSendsInFlight/description"),
            settings.max_sends_in_flight() as f64
        );

        let config = config_of(b"{}");
        assert_eq!(
            stated("/properties/MaxReceiveMessageSize/description"),
            config.max_recv_message_size as f64
        );
        assert_eq!(
            stated("/properties/DeliveryCoalescingBytes/description"),
            config.delivery_coalescing as f64
        );
        assert_eq!(
            stated("/$defs/TransportOptions/properties/ConnectTimeoutSeconds/description"),
            config.transport.connect_timeout.as_secs_f64()
        );
        let http2 = config.transport.http2;
        assert_eq!(
            stated("/$defs/Http2Options/properties/KeepAliveTimeoutSeconds/description"),
            http2.keep_alive_timeout.as_secs_f64()
        );
        assert_eq!(
            stated("/$defs/Http2Options/properties/StreamWindowSize/description"),
            http2.stream_window as f64
        );
        assert_eq!(
            stated("/$defs/Http2Options/properties/ConnectionWindowSize/description"),
            http2.connection_window as f64
        );
        assert_eq!(
            stated("/$defs/Http2Options/properties/WriteCoalescingBytes/description"),
            http2.write_coalescing as f64
        );
        let retry = config.retry.expect("a retry policy by default");
        for (option, applied) in [
            ("MaxAttempts", f64::from(retry.max_attempts)),
            ("InitialBackoffSeconds", retry.initial_backoff.as_secs_f64()),
            ("MaxBackoffSeconds", retry.max_backoff.as_secs_f64()),
            ("BackoffMultiplier", retry.backoff_multiplier),
            ("CallReplayBytes", retry.call_replay_bytes as f64),
            ("ChannelReplayBytes", retry.channel_replay_bytes as f64),
        ] {
            assert_eq!(
                stated(&format!(
                    "/$defs/RetryOptions/properties/{option}/description"
                )),
                applied,
                "{option}"
            );
        }
    }

    /// The bounds below are stated twice - once as a schemars attribute, once as this reader -
    /// and this is what says the two are the same numbers.
    ///
    /// They are stated twice on purpose: nothing obliges a host to have validated its document,
    /// so the reader checks rather than trusts. What that costs is a copy, and a copy drifts:
    /// widen the schema alone and a document a validator accepts is refused here.
    #[test]
    fn every_bound_the_schema_states_is_one_the_reader_holds_to() {
        let schema: serde_json::Value = serde_json::from_str(&armonik_transport::options::schema())
            .expect("the schema renders as JSON");
        let stated = |pointer: &str| schema.pointer(pointer).and_then(serde_json::Value::as_i64);
        let admits = |document: String| parse(document.as_bytes()).is_ok();

        for option in [
            "DeliveryCredits",
            "MaxSendsInFlight",
            "MaxReceiveMessageSize",
            "DeliveryCoalescingBytes",
        ] {
            let minimum = stated(&format!("/properties/{option}/minimum"))
                .unwrap_or_else(|| panic!("{option} states no minimum"));
            assert!(
                !admits(format!(r#"{{"{option}":{}}}"#, minimum - 1)),
                "{option} is admitted below the minimum the schema states"
            );
            assert!(
                admits(format!(r#"{{"{option}":{minimum}}}"#)),
                "{option} is refused at the minimum the schema states"
            );

            // Absent for the receive size, whose largest value is a channel refusing nothing.
            if let Some(maximum) = stated(&format!("/properties/{option}/maximum")) {
                assert!(
                    admits(format!(r#"{{"{option}":{maximum}}}"#)),
                    "{option} is refused at the maximum the schema states"
                );
                assert!(
                    !admits(format!(r#"{{"{option}":{}}}"#, maximum + 1)),
                    "{option} is admitted above the maximum the schema states"
                );
            }
        }

        assert_eq!(stated("/properties/UserAgent/minLength"), Some(1));
        assert!(!admits(r#"{"UserAgent":""}"#.to_owned()));
        assert!(admits(r#"{"UserAgent":"a"}"#.to_owned()));

        assert_eq!(
            schema
                .pointer("/$defs/TransportOptions/properties/ConnectTimeoutSeconds/minimum")
                .and_then(serde_json::Value::as_f64),
            Some(1e-9)
        );
        assert!(!admits(
            r#"{"Transport":{"ConnectTimeoutSeconds":0.0}}"#.to_owned()
        ));
        assert!(!admits(
            r#"{"Transport":{"ConnectTimeoutSeconds":9.99e-10}}"#.to_owned()
        ));
        assert!(admits(
            r#"{"Transport":{"ConnectTimeoutSeconds":1e-9}}"#.to_owned()
        ));
        // The same number, as .NET Framework's System.Text.Json writes it: seventeen digits, which
        // only a correctly rounded parse reads back as the double the host meant.
        assert!(admits(
            r#"{"Transport":{"ConnectTimeoutSeconds":1.0000000000000001E-09}}"#.to_owned()
        ));
        // The smallest it admits is still a duration, and not the zero the transport refuses.
        assert_eq!(
            config_of(br#"{"Transport":{"ConnectTimeoutSeconds":1e-9}}"#)
                .transport
                .connect_timeout,
            Duration::from_nanos(1)
        );

        assert_eq!(
            schema
                .pointer("/properties/DefaultDeadlineSeconds/minimum")
                .and_then(serde_json::Value::as_f64),
            Some(1e-9)
        );
        assert!(!admits(r#"{"DefaultDeadlineSeconds":0.0}"#.to_owned()));
        assert!(!admits(
            r#"{"DefaultDeadlineSeconds":18446744073709551616.0}"#.to_owned()
        ));
        assert_eq!(
            config_of(br#"{"DefaultDeadlineSeconds":1e-9}"#).default_deadline,
            Some(Duration::from_nanos(1))
        );
        assert_eq!(config_of(b"{}").default_deadline, None);

        assert!(parse(br#"{"ConnectEagerly":true}"#)
            .expect("valid")
            .connect_eagerly());
        assert!(!parse(b"{}").expect("valid").connect_eagerly());

        // The ceiling is the type's rather than the option's, so it is read off `Seconds`: every
        // duration becomes a `Duration`, which holds `u64::MAX` seconds, and 2^64 is the first
        // value none can be. The value below it is the largest a `f64` can name.
        assert_eq!(
            schema
                .pointer("/$defs/Seconds/exclusiveMaximum")
                .and_then(serde_json::Value::as_f64),
            Some(18446744073709551616.0)
        );
        assert!(!admits(
            r#"{"Transport":{"ConnectTimeoutSeconds":18446744073709551616.0}}"#.to_owned()
        ));
        assert!(admits(
            r#"{"Transport":{"ConnectTimeoutSeconds":18446744073709549568.0}}"#.to_owned()
        ));

        // The new units' integers, read from where each sits in the document.
        for (pointer, document) in [
            (
                "/$defs/TcpKeepaliveOptions/properties/Retries/minimum",
                r#"{"Transport":{"TcpKeepalive":{"IdleSeconds":30,"Retries":N}}}"#,
            ),
            (
                "/$defs/Http2Options/properties/StreamWindowSize/minimum",
                r#"{"Http2":{"StreamWindowSize":N}}"#,
            ),
            (
                "/$defs/Http2Options/properties/ConnectionWindowSize/minimum",
                r#"{"Http2":{"ConnectionWindowSize":N}}"#,
            ),
            (
                "/$defs/Http2Options/properties/WriteCoalescingBytes/minimum",
                r#"{"Http2":{"WriteCoalescingBytes":N}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/MaxAttempts/minimum",
                r#"{"Retry":{"MaxAttempts":N}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/CallReplayBytes/minimum",
                r#"{"Retry":{"CallReplayBytes":N}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/ChannelReplayBytes/minimum",
                r#"{"Retry":{"ChannelReplayBytes":N}}"#,
            ),
        ] {
            let minimum = stated(pointer).unwrap_or_else(|| panic!("{pointer} states none"));
            let at = |value: i64| document.replace('N', &value.to_string());
            assert!(!admits(at(minimum - 1)), "{pointer}: below is admitted");
            assert!(admits(at(minimum)), "{pointer}: the minimum is refused");
        }
        for (pointer, document) in [
            (
                "/$defs/TcpKeepaliveOptions/properties/IdleSeconds/minimum",
                r#"{"Transport":{"TcpKeepalive":{"IdleSeconds":N}}}"#,
            ),
            (
                "/$defs/TcpKeepaliveOptions/properties/IntervalSeconds/minimum",
                r#"{"Transport":{"TcpKeepalive":{"IdleSeconds":30,"IntervalSeconds":N}}}"#,
            ),
            (
                "/$defs/Http2Options/properties/KeepAliveIntervalSeconds/minimum",
                r#"{"Http2":{"KeepAliveIntervalSeconds":N}}"#,
            ),
            (
                "/$defs/Http2Options/properties/KeepAliveTimeoutSeconds/minimum",
                r#"{"Http2":{"KeepAliveTimeoutSeconds":N}}"#,
            ),
            (
                "/$defs/Http2Options/properties/IdleTimeoutSeconds/minimum",
                r#"{"Http2":{"IdleTimeoutSeconds":N}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/InitialBackoffSeconds/minimum",
                r#"{"Retry":{"InitialBackoffSeconds":N}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/MaxBackoffSeconds/minimum",
                r#"{"Retry":{"InitialBackoffSeconds":1e-9,"MaxBackoffSeconds":N}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/BackoffMultiplier/minimum",
                r#"{"Retry":{"BackoffMultiplier":N}}"#,
            ),
        ] {
            let minimum = schema
                .pointer(pointer)
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_else(|| panic!("{pointer} states none"));
            let at = |value: f64| document.replace('N', &format!("{value:e}"));
            assert!(!admits(at(minimum / 2.0)), "{pointer}: below is admitted");
            assert!(admits(at(minimum)), "{pointer}: the minimum is refused");
        }
    }

    /// A certificate and its key, as PEM files in a directory of the test's own.
    fn pem_files(test: &str) -> (String, String) {
        let directory = std::env::temp_dir().join(format!(
            "armonik-transport-ffi-config-{}-{test}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        let key = rcgen::KeyPair::generate().expect("a key");
        let certificate = rcgen::CertificateParams::new(vec!["client.test".to_owned()])
            .expect("parameters")
            .self_signed(&key)
            .expect("a certificate");
        let write = |name: &str, content: String| {
            let path = directory.join(name);
            std::fs::write(&path, content).expect("a scratch file");
            // As a JSON string: a Windows path's backslashes are escapes there.
            path.to_string_lossy().replace('\\', "/")
        };
        (
            write("cert.pem", certificate.pem()),
            write("key.pem", key.serialize_pem()),
        )
    }

    #[test]
    fn every_option_of_the_tls_keepalive_and_http2_units_reaches_the_engine() {
        let (certificate, key) = pem_files("reaches");
        let document = format!(
            r#"{{
                "Transport": {{
                    "Tls": {{
                        "Server": {{ "CaPem": "{certificate}" }},
                        "Client": {{ "Pem": {{ "Certificate": "{certificate}", "Key": "{key}" }} }},
                        "OverrideTargetName": "server.test"
                    }},
                    "TcpKeepalive": {{ "IdleSeconds": 30, "IntervalSeconds": 5, "Retries": 3 }}
                }},
                "Http2": {{
                    "KeepAliveIntervalSeconds": 10,
                    "KeepAliveTimeoutSeconds": 2.5,
                    "KeepAliveWhileIdle": true,
                    "StreamWindowSize": 1048576,
                    "ConnectionWindowSize": 3145728
                }}
            }}"#
        );
        let settings = parse(document.as_bytes()).expect("an admissible document");
        let _ = std::fs::remove_dir_all(
            std::path::Path::new(&certificate)
                .parent()
                .expect("a directory"),
        );
        let config = settings.into_channel_config("https://127.0.0.1:5000".parse().expect("a uri"));

        let tls = &config.transport.tls;
        assert_eq!(tls.roots.len(), 1);
        assert_eq!(
            tls.identity.as_ref().map(|identity| identity.chain.len()),
            Some(1)
        );
        assert_eq!(tls.server_name.as_deref(), Some("server.test"));

        let tcp = config.transport.tcp;
        assert_eq!(tcp.keepalive, Some(Duration::from_secs(30)));
        assert_eq!(tcp.keepalive_interval, Some(Duration::from_secs(5)));
        assert_eq!(tcp.keepalive_retries, Some(3));

        let http2 = config.transport.http2;
        assert_eq!(http2.keep_alive_interval, Some(Duration::from_secs(10)));
        assert_eq!(http2.keep_alive_timeout, Duration::from_millis(2500));
        assert!(http2.keep_alive_while_idle);
        assert_eq!(http2.stream_window, 1_048_576);
        assert_eq!(http2.connection_window, 3_145_728);

        let unsafe_document = br#"{"Transport":{"Tls":{"Server":{"Unverified":true}}}}"#;
        let config = parse(unsafe_document)
            .expect("admissible")
            .into_channel_config("https://127.0.0.1:5000".parse().expect("a uri"));
        assert!(config.transport.tls.accept_any_server);
    }

    #[test]
    fn a_channel_that_could_receive_no_message_is_refused() {
        // Every other size is a channel that refuses some messages; zero refuses all of them,
        // which is a configuration with no use and a call that can only ever fail.
        assert!(parse(br#"{"MaxReceiveMessageSize":0}"#).is_err());
        assert!(parse(br#"{"MaxReceiveMessageSize":1}"#).is_ok());
    }

    #[test]
    fn a_document_that_names_nothing_is_a_valid_configuration() {
        // The endpoint crosses the ABI on its own, so nothing here is mandatory and every option
        // has a default.
        let config = config_of(b"{}");

        assert_eq!(
            config.transport.endpoint.to_string(),
            "http://127.0.0.1:5000/"
        );
        assert_eq!(config.max_sends_in_flight, MAX_SENDS_IN_FLIGHT as usize);
    }

    #[test]
    fn an_option_spelled_wrong_is_refused_rather_than_ignored() {
        assert!(parse(br#"{"UserAgnt":"typo"}"#).is_err());
    }

    #[test]
    fn an_option_spelled_as_the_wrong_type_is_refused() {
        // The schema says a number, so a string spelled like one is not the same document. A
        // reader that took it would make the schema a suggestion.
        assert!(parse(br#"{"DeliveryCredits":"2"}"#).is_err());
        assert!(parse(br#"{"DeliveryCredits":2}"#).is_ok());
    }

    #[test]
    fn a_nested_unit_is_read_as_an_object() {
        let config = config_of(br#"{"Transport":{"ConnectTimeoutSeconds":2.5}}"#);

        assert_eq!(
            config.transport.connect_timeout,
            std::time::Duration::from_millis(2500)
        );
    }

    #[test]
    fn a_timeout_no_duration_can_hold_is_refused_rather_than_read() {
        // JSON has no NaN and no infinity, but 1e300 is an ordinary double and no `Duration`
        // holds it. Refusing it here is what keeps the conversion below infallible in practice:
        // a value this admits and that panics one line later reaches a host as INTERNAL, which
        // says a fault where the truth is a configuration error.
        assert!(parse(br#"{"Transport":{"ConnectTimeoutSeconds":1e300}}"#).is_err());
    }

    #[test]
    fn every_document_this_reader_admits_becomes_a_config() {
        // The reader's own promise: what it returns `Ok` for is what `into_channel_config`
        // can build, so no admitted document panics on the way through.
        for admitted in [
            &b"{}"[..],
            &br#"{"Transport":{"ConnectTimeoutSeconds":1e300}}"#[..],
            &br#"{"Transport":{"ConnectTimeoutSeconds":2.5}}"#[..],
            &br#"{"Transport":{"ConnectTimeoutSeconds":1.7976931348623157e308}}"#[..],
        ] {
            if parse(admitted).is_ok() {
                let _ = config_of(admitted);
            }
        }
    }

    #[test]
    fn a_bound_the_schema_states_is_a_bound_this_refuses() {
        for refused in [
            &br#"{"DeliveryCredits":0}"#[..],
            &br#"{"MaxSendsInFlight":0}"#[..],
            &br#"{"UserAgent":""}"#[..],
            &br#"{"Transport":{"ConnectTimeoutSeconds":0.0}}"#[..],
        ] {
            assert!(
                parse(refused).is_err(),
                "{}",
                String::from_utf8_lossy(refused)
            );
        }
    }

    #[test]
    fn a_refusal_names_the_key_it_was_refused_over() {
        for (document, key) in [
            (&br#"{"UserAgnt":"typo"}"#[..], "UserAgnt"),
            (&br#"{"DeliveryCredits":"2"}"#[..], "DeliveryCredits"),
            (&br#"{"DeliveryCredits":0}"#[..], "DeliveryCredits"),
            (&br#"{"MaxSendsInFlight":0}"#[..], "MaxSendsInFlight"),
            (
                &br#"{"MaxReceiveMessageSize":0}"#[..],
                "MaxReceiveMessageSize",
            ),
            (
                &br#"{"DeliveryCoalescingBytes":-1}"#[..],
                "DeliveryCoalescingBytes",
            ),
            (&br#"{"UserAgent":""}"#[..], "UserAgent"),
            (
                &br#"{"Transport":{"ConnectTimeoutSeconds":0.0}}"#[..],
                "ConnectTimeoutSeconds",
            ),
            (
                &br#"{"Transport":{"ConnectTimeoutSeconds":1e300}}"#[..],
                "ConnectTimeoutSeconds",
            ),
            (
                &br#"{"Transport":{"ConnectTimeoutSeconds":"1"}}"#[..],
                "Transport.ConnectTimeoutSeconds",
            ),
            (
                &br#"{"Transport":{"Tls":{"Server":{"CaPem":"no/such/file.pem"}}}}"#[..],
                "Transport.Tls.Server.CaPem",
            ),
            (
                &br#"{"Transport":{"TcpKeepalive":{"Retries":3}}}"#[..],
                "Transport.TcpKeepalive.Retries",
            ),
            (
                &br#"{"Http2":{"ConnectionWindowSize":65534}}"#[..],
                "Http2.ConnectionWindowSize",
            ),
            (
                &br#"{"Transport":{"Tls":{"Client":{"P12":{"Path":"c.p12","Password":123456}}}}}"#
                    [..],
                "Transport.Tls.Client.P12.Password",
            ),
            (
                &br#"{"Transport":{"Proxy":{"Url":{"Address":"https://proxy.test"}}}}"#[..],
                "Transport.Proxy.Url.Address",
            ),
            (
                &br#"{"Transport":{"Proxy":{"None":true,"System":{}}}}"#[..],
                "Transport.Proxy",
            ),
        ] {
            let Err(refused) = parse(document) else {
                panic!("{} is admitted", String::from_utf8_lossy(document));
            };
            let said = refused.to_string();
            assert!(
                said.contains(key),
                "{} is refused without naming {key}: {said}",
                String::from_utf8_lossy(document)
            );
        }
    }

    #[test]
    fn the_proxy_options_reach_the_engine() {
        use armonik_transport::http2::ProxySource;

        let config = config_of(
            br#"{"Transport":{"Proxy":{"Url":{"Address":"proxy.test:3128","Username":"alice","Password":"s3cret"}}}}"#,
        );
        let proxy = &config.transport.proxy;
        let ProxySource::Explicit(uri) = &proxy.source else {
            panic!("{proxy:?}");
        };
        assert_eq!(uri.to_string(), "http://proxy.test:3128/");
        assert_eq!(proxy.username, "alice");
        assert!(!format!("{proxy:?}").contains("s3cret"));
    }

    #[test]
    fn a_password_of_the_wrong_type_is_refused_without_being_quoted() {
        for document in [
            &br#"{"Transport":{"Tls":{"Client":{"P12":{"Path":"c.p12","Password":123456}}}}}"#[..],
            &br#"{"Transport":{"Tls":{"Client":{"P12":{"Path":"c.p12","Password":-123456.5}}}}}"#[..],
            &br#"{"Transport":{"Tls":{"Client":{"P12":{"Path":"c.p12","Password":["s3cret"]}}}}}"#[..],
            &br#"{"Transport":{"Tls":{"Client":{"P12":{"Path":"c.p12","Password":{"s3cret":1}}}}}}"#[..],
        ] {
            let Err(refused) = parse(document) else {
                panic!("{} is admitted", String::from_utf8_lossy(document));
            };
            let said = refused.to_string();
            assert!(said.contains("Transport.Tls.Client.P12.Password"), "{said}");
            assert!(!said.contains("123456"), "{said}");
            assert!(!said.contains("s3cret"), "{said}");
        }
    }

    #[test]
    fn a_window_past_what_the_schema_admits_is_refused() {
        let past = format!(r#"{{"DeliveryCredits":{}}}"#, LARGEST_WINDOW as i64 + 1);
        assert!(parse(past.as_bytes()).is_err());
    }

    /// A channel's document is merged over the runtime's defaults option by option, a struct's
    /// options within it: what the channel states wins, and what it leaves out is the default's.
    #[test]
    fn a_channel_document_is_merged_over_the_defaults() {
        let defaults = defaults(
            br#"{"DeliveryCredits":2,"Http2":{"KeepAliveWhileIdle":true,"StreamWindowSize":70000}}"#,
        )
        .expect("valid defaults");
        let settings = parse_over(
            defaults.as_ref(),
            br#"{"Http2":{"StreamWindowSize":80000}}"#,
        )
        .expect("a valid merge");
        assert_eq!(settings.delivery_credits(), 2);
        let http2 = settings
            .into_channel_config("http://127.0.0.1:5000".parse().expect("an endpoint"))
            .transport
            .http2;
        assert!(http2.keep_alive_while_idle);
        assert_eq!(http2.stream_window, 80_000);
    }

    /// Defaults are refused as a channel's document is, and empty ones are none.
    #[test]
    fn defaults_are_read_as_a_channel_document() {
        assert!(defaults(b"").expect("empty is none").is_none());
        for document in [
            &br#"{"DeliveryCredits":0}"#[..],
            &br#"{"NoSuchOption":1}"#[..],
            &b"not json"[..],
        ] {
            let Err(refused) = defaults(document) else {
                panic!("{} is admitted", String::from_utf8_lossy(document));
            };
            let said = refused.to_string();
            assert!(said.starts_with("ChannelDefaults: "), "{said}");
        }
    }

    /// A key a channel states as null states nothing, as serde reads it, so the default stands.
    #[test]
    fn a_null_in_a_channel_document_leaves_the_default() {
        let defaults = defaults(br#"{"DeliveryCredits":2}"#).expect("valid defaults");
        let settings =
            parse_over(defaults.as_ref(), br#"{"DeliveryCredits":null}"#).expect("a valid merge");
        assert_eq!(settings.delivery_credits(), 2);
    }

    /// Bounds the defaults and the channel each state half of cumulate, and a merge where they
    /// disagree is refused as the merge's, the channel's own document being admitted alone.
    #[test]
    fn bounds_that_disagree_once_merged_refuse_the_merge() {
        let defaults = defaults(br#"{"Retry":{"MaxBackoffSeconds":2}}"#).expect("valid defaults");
        let own = br#"{"Retry":{"InitialBackoffSeconds":3}}"#;
        assert!(
            parse(own).is_ok(),
            "the channel's document alone is admitted"
        );
        let Err(refused) = parse_over(defaults.as_ref(), own) else {
            panic!("a merge whose backoff cannot grow is admitted");
        };
        let said = refused.to_string();
        assert!(said.contains("Retry"), "{said}");
        assert!(
            said.ends_with("once merged over the runtime's ChannelDefaults"),
            "{said}"
        );
    }

    /// Another alternative than the default's, stated by the channel, replaces it whole.
    #[test]
    fn an_alternative_the_channel_states_replaces_the_defaults() {
        let defaults = defaults(
            br#"{"Transport":{"Proxy":{"Url":{"Address":"proxy.test:3128","Username":"alice"}}}}"#,
        )
        .expect("valid defaults");
        let settings = parse_over(
            defaults.as_ref(),
            br#"{"Transport":{"Proxy":{"None":true}}}"#,
        )
        .expect("a valid merge");
        let proxy = settings
            .into_channel_config("http://127.0.0.1:5000".parse().expect("an endpoint"))
            .transport
            .proxy;
        assert_eq!(
            proxy.source,
            armonik_transport::http2::ProxySource::Disabled
        );
        assert_eq!(proxy.username, "");
    }

    /// A channel's own document is refused over the defaults as it would be alone.
    #[test]
    fn a_channel_document_is_refused_over_the_defaults_as_alone() {
        let defaults = defaults(br#"{"DeliveryCredits":2}"#).expect("valid defaults");
        assert!(parse_over(defaults.as_ref(), br#"{"DeliveryCredits":0}"#).is_err());
        assert!(parse_over(defaults.as_ref(), b"not json").is_err());
    }
}
