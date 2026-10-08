use std::fmt;

use armonik_transport::configuration::{ConfigRefusal as LoadRefusal, Configuration};
use armonik_transport::options::{ChannelOptions, OptionRefusal, RuntimeOptions};
use armonik_transport::reexports::http::Uri;
use armonik_transport::settings::{ChannelSettings, SettingRefusal};

use crate::abi::{
    ak_config, ak_config_source, ak_error_kind, ak_source_kind, ak_status, AK_CONFIG_NO_PREFIX,
};
use crate::refusal::Refusal;

/// Why a document was refused, named by the key it was refused over.
#[derive(Debug)]
pub(crate) enum ConfigRefusal {
    /// A source the loader refused: a file that is missing or does not parse, a text that is not
    /// JSON, or a value of the wrong type, with the path of the key it was refused at.
    Loaded(LoadRefusal),
    /// A document whose bytes are not UTF-8, the only encoding the ABI takes JSON in.
    NotUtf8,
    /// A memory ceiling of zero, which a configuration has no reason to write: it leaves the
    /// option out for the default.
    ZeroCeiling { key: &'static str },
    /// An Endpoint that names nothing a channel could reach.
    Endpoint { why: &'static str },
    /// Options the engine cannot be configured with, by the key at fault.
    Settled(SettingRefusal),
    /// An alternative whose own values contradict it, found before any merge.
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
            Self::Loaded(refused) => refused.fmt(f),
            Self::NotUtf8 => f.write_str("the configuration document is not UTF-8"),
            Self::ZeroCeiling { key } => write!(f, "{key} is 0, and has to be at least 1"),
            Self::Endpoint { why } => write!(f, "Endpoint {why}"),
            Self::Settled(refused) => refused.fmt(f),
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

// No source: the text of every variant says its cause already, and a source is said again after
// it.
impl std::error::Error for ConfigRefusal {}

/// Reads a runtime's channel defaults: a channel document, refused as a channel's own is, and
/// kept as the options each channel's own are merged over. Empty is none.
pub(crate) fn defaults(json: &[u8]) -> Result<Option<ChannelOptions>, ConfigRefusal> {
    if json.is_empty() {
        return Ok(None);
    }
    let options = read(json).map_err(|refused| ConfigRefusal::Defaults(Box::new(refused)))?;
    admit_defaults(&options)?;
    Ok(Some(options))
}

/// Refuses channel defaults a channel's own document would be refused for, except for options
/// that cannot hold together: a channel can still override those, so they are said in the log and
/// the runtime is created.
fn admit_defaults(options: &ChannelOptions) -> Result<(), ConfigRefusal> {
    ChannelSettings::settle_defaults(options.clone())
        .map_err(|refused| ConfigRefusal::Defaults(Box::new(ConfigRefusal::Settled(refused))))
}

/// Loads a runtime's options from the sources a host listed, and refuses what the runtime could
/// not be created with: a zero ceiling, an endpoint that is not a URI, channel defaults a channel
/// would be refused for.
pub(crate) fn runtime(configuration: &Configuration) -> Result<RuntimeOptions, ConfigRefusal> {
    let options: RuntimeOptions = configuration.load().map_err(ConfigRefusal::Loaded)?;
    for (key, ceiling) in [
        ("MemoryCeiling", options.memory_ceiling),
        ("MemoryHardCeiling", options.memory_hard_ceiling),
    ] {
        if ceiling == Some(0) {
            return Err(ConfigRefusal::ZeroCeiling { key });
        }
    }
    // Not quoted: a URI may carry credentials in its userinfo.
    match options.endpoint.as_deref() {
        Some("") => {
            return Err(ConfigRefusal::Endpoint {
                why: "is empty, and has to name the server, as http://host:port",
            })
        }
        Some(endpoint) if endpoint.parse::<Uri>().is_err() => {
            return Err(ConfigRefusal::Endpoint {
                why: "is not a URI such as http://host:port or https://host:port",
            })
        }
        _ => {}
    }
    if let Some(defaults) = &options.channel_defaults {
        admit_defaults(defaults)?;
    }
    Ok(options)
}

/// The configuration a host's `ak_config` lists, every field of it checked: what is malformed is
/// refused here, and no source is read before the load.
///
/// # Safety
///
/// `config.sources` must point at `config.source_count` sources unless that is zero, and every
/// byte view at its length.
pub(crate) unsafe fn sources(config: &ak_config) -> Result<Configuration, Refusal> {
    let prefix = text(unsafe { config.prefix.as_slice() })?;
    let mut configuration = match (config.flags & AK_CONFIG_NO_PREFIX != 0, prefix) {
        (true, "") => Configuration::with_prefix(""),
        (true, _) => return Err(PREFIX_BESIDE_NONE),
        (false, "") => Configuration::new(),
        (false, prefix) => Configuration::with_prefix(prefix),
    };
    let sources: &[ak_config_source] = match (config.source_count, config.sources.is_null()) {
        (0, _) => &[],
        (_, true) => return Err(crate::NULL_ARGUMENT),
        (count, false) => unsafe { std::slice::from_raw_parts(config.sources, count as usize) },
    };
    for source in sources {
        if source.reserved != 0 {
            return Err(SOURCE_RESERVED_SET);
        }
        let value = text(unsafe { source.value.as_slice() })?;
        configuration = match source.kind {
            kind if kind == ak_source_kind::AK_SOURCE_FILE as u32 => configuration.file(value),
            kind if kind == ak_source_kind::AK_SOURCE_OPTIONAL_FILE as u32 => {
                configuration.optional_file(value)
            }
            kind if kind == ak_source_kind::AK_SOURCE_ENVIRONMENT as u32 => {
                if !value.is_empty() {
                    return Err(ENVIRONMENT_VALUE);
                }
                configuration.environment()
            }
            kind if kind == ak_source_kind::AK_SOURCE_DOCUMENT as u32 => {
                configuration.document(value)
            }
            kind if kind == ak_source_kind::AK_SOURCE_PAIRS as u32 => {
                configuration.pairs_json(value)
            }
            _ => return Err(UNKNOWN_KIND),
        };
    }
    Ok(configuration)
}

/// A byte view of `ak_config` as the text it has to be.
fn text(bytes: Option<&[u8]>) -> Result<&str, Refusal> {
    std::str::from_utf8(bytes.ok_or(crate::NULL_SLICE)?).map_err(|_| NOT_UTF8)
}

const PREFIX_BESIDE_NONE: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "the configuration names a prefix and AK_CONFIG_NO_PREFIX at once",
);
const SOURCE_RESERVED_SET: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "a source's reserved field is not zero",
);
const UNKNOWN_KIND: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "a source's kind is not one ak_source_kind defines",
);
const ENVIRONMENT_VALUE: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "an environment source carries a value, where it takes none",
);
const NOT_UTF8: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "the prefix or a source's value is not UTF-8",
);

/// Reads a channel's document over the runtime's defaults, as `ChannelOptions::over` merges
/// them, and answers the settings and the options they were made from. A refusal the channel's
/// document earns alone is the one reported, in its own terms; one it does not is the merge's, and
/// so is an incoherence, whatever the document says alone.
pub(crate) fn parse_effective(
    defaults: Option<&ChannelOptions>,
    json: &[u8],
) -> Result<(ChannelSettings, ChannelOptions), ConfigRefusal> {
    let own = read(json)?;
    let Some(defaults) = defaults else {
        return settle(own.clone()).map(|settings| (settings, own));
    };
    // What the document gets wrong by itself is refused before any merge is made, by the
    // document's own path: such a refusal does not depend on the defaults.
    own.check().map_err(ConfigRefusal::Option)?;
    let merged = own.clone().over(defaults);
    match settle(merged.clone()) {
        Ok(settings) => Ok((settings, merged)),
        Err(refused) => Err(match settle(own) {
            // Options that cannot hold together are a fact of the merge: alone, a document may
            // well lack what the defaults give it.
            Err(alone) if !is_incoherence(&alone) => alone,
            _ => ConfigRefusal::Merged(Box::new(refused)),
        }),
    }
}

/// What a runtime was created with, logged once: every option, each as its own type renders it, so
/// that a password and a proxy's credentials show redacted and a certificate shows as the path it
/// names.
pub(crate) fn log_runtime(options: &RuntimeOptions) {
    tracing::info!(
        endpoint = options
            .endpoint
            .as_deref()
            .and_then(|endpoint| endpoint.parse::<Uri>().ok())
            .map(|endpoint| armonik_transport::safe_endpoint(&endpoint)),
        memory_ceiling = options.memory_ceiling,
        memory_hard_ceiling = options.memory_hard_ceiling,
        channel_defaults = ?options.channel_defaults,
        log_filter = options
            .logging
            .filter
            .as_deref()
            .unwrap_or(crate::log::DEFAULT_FILTER),
        "the runtime's effective configuration"
    );
}

/// What a channel was created with, merged over the runtime's defaults, logged as `log_runtime`
/// logs the runtime's - unless the merge is the defaults themselves, which the runtime's log has
/// said.
pub(crate) fn log_channel(
    endpoint: &Uri,
    options: &ChannelOptions,
    defaults: Option<&ChannelOptions>,
) {
    let inherited = match defaults {
        Some(defaults) => options == defaults,
        None => *options == ChannelOptions::default(),
    };
    if inherited {
        return;
    }
    tracing::info!(
        endpoint = %armonik_transport::safe_endpoint(endpoint),
        options = ?options,
        "the channel's effective configuration"
    );
}

fn is_incoherence(refused: &ConfigRefusal) -> bool {
    matches!(
        refused,
        ConfigRefusal::Settled(SettingRefusal::Incoherent(_))
    )
}

/// A document read alone, as a channel with no runtime defaults reads it.
#[cfg(test)]
pub(crate) fn parse(json: &[u8]) -> Result<ChannelSettings, ConfigRefusal> {
    settle(read(json)?)
}

/// A channel document, through the loader a runtime's configuration goes through, so that a key
/// it does not declare is logged as one in any other source is.
fn read(json: &[u8]) -> Result<ChannelOptions, ConfigRefusal> {
    let json = std::str::from_utf8(json).map_err(|_| ConfigRefusal::NotUtf8)?;
    Configuration::with_prefix("")
        .document(json)
        .load()
        .map_err(ConfigRefusal::Loaded)
}

fn settle(options: ChannelOptions) -> Result<ChannelSettings, ConfigRefusal> {
    ChannelSettings::settle(options).map_err(ConfigRefusal::Settled)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use armonik_transport::grpc::Encoding;
    use armonik_transport::grpc::{GrpcChannelConfig, RateLimitConfig};
    use armonik_transport::http2::{FixedWindows, ReceiveWindows};
    use armonik_transport::options::LARGEST_WINDOW;

    /// `parse_effective`, for the tests that read the settings alone.
    fn parse_over(
        defaults: Option<&ChannelOptions>,
        json: &[u8],
    ) -> Result<ChannelSettings, ConfigRefusal> {
        parse_effective(defaults, json).map(|(settings, _)| settings)
    }

    fn config_of(json: &[u8]) -> GrpcChannelConfig {
        let settings = parse(json).expect("valid");
        settings.into_channel_config("http://127.0.0.1:5000".parse().expect("an endpoint"))
    }

    /// Every option of the runtime's schema but the endpoint and the logging filter is a field of
    /// `ak_runtime_config`, so that `ak_runtime_create` takes what `ak_runtime_create_from` loads;
    /// the endpoint is what a channel names itself there, and the filter is given through the
    /// loader's options alone.
    #[test]
    fn every_runtime_option_but_the_endpoint_and_the_filter_is_a_field_of_the_config() {
        let schema: serde_json::Value =
            serde_json::from_str(&armonik_transport::options::runtime_schema())
                .expect("the schema is a document");
        let mut names: Vec<_> = schema["properties"]
            .as_object()
            .expect("the schema has properties")
            .keys()
            .cloned()
            .collect();
        names.sort();

        let config = crate::abi::ak_runtime_config {
            struct_size: 0,
            version: 0,
            flags: 0,
            reserved: 0,
            memory_ceiling: 0,
            memory_hard_ceiling: 0,
            channel_defaults_json: crate::abi::ak_bytes_in {
                ptr: std::ptr::null(),
                len: 0,
            },
            log_callback: None,
            log_ctx: std::ptr::null_mut(),
        };
        // Logging.Filter is no field: a filter other than the default is only loaded, and
        // ak_runtime_create has no sources to load it from.
        let _ = (
            config.memory_ceiling,
            config.memory_hard_ceiling,
            config.channel_defaults_json,
        );
        assert_eq!(
            names,
            [
                "ChannelDefaults",
                "Endpoint",
                "Logging",
                "MemoryCeiling",
                "MemoryHardCeiling"
            ]
        );
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
            stated("/$defs/HostReceiveOptions/properties/Window/description"),
            settings.delivery_credits() as f64
        );
        assert_eq!(
            stated("/$defs/HostSendOptions/properties/Window/description"),
            settings.max_sends_in_flight() as f64
        );

        let config = config_of(b"{}");
        assert_eq!(
            stated("/$defs/GrpcReceiveOptions/properties/MaxMessageSize/description"),
            config.max_recv_message_size as f64
        );
        assert_eq!(config.max_send_message_size, None);
        assert_eq!(
            stated("/$defs/HostReceiveOptions/properties/CoalescingBytes/description"),
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
        let ReceiveWindows::Fixed(windows) = http2.receive_windows else {
            panic!("fixed windows by default: {:?}", http2.receive_windows);
        };
        assert_eq!(
            stated("/$defs/Http2FixedWindows/properties/StreamWindowSize/description"),
            windows.stream as f64
        );
        assert_eq!(
            stated("/$defs/Http2FixedWindows/properties/ConnectionWindowSize/description"),
            windows.connection as f64
        );
        assert_eq!(
            stated("/$defs/Http2SendOptions/properties/CoalescingBytes/description"),
            http2.write_coalescing as f64
        );
        assert_eq!(
            stated("/$defs/Http2SendOptions/properties/StreamBufferSize/description"),
            http2.send_buffer as f64
        );
        assert_eq!(
            stated("/$defs/Http2SendOptions/properties/FramesPerWrite/description"),
            http2.frames_per_write as f64
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

        // Each option, by where the schema states it and the path a document names it by.
        let document = |path: &[&str], value: i64| {
            path.iter().rev().fold(value.to_string(), |inner, key| {
                format!(r#"{{"{key}":{inner}}}"#)
            })
        };
        for (option, path) in [
            (
                "/$defs/HostReceiveOptions/properties/Window",
                &["Grpc", "Host", "Receive", "Window"][..],
            ),
            (
                "/$defs/HostSendOptions/properties/Window",
                &["Grpc", "Host", "Send", "Window"][..],
            ),
            (
                "/$defs/GrpcSendOptions/properties/MaxMessageSize",
                &["Grpc", "Send", "MaxMessageSize"][..],
            ),
            (
                "/$defs/GrpcReceiveOptions/properties/MaxMessageSize",
                &["Grpc", "Receive", "MaxMessageSize"][..],
            ),
            (
                "/$defs/HostReceiveOptions/properties/CoalescingBytes",
                &["Grpc", "Host", "Receive", "CoalescingBytes"][..],
            ),
        ] {
            let minimum = stated(&format!("{option}/minimum"))
                .unwrap_or_else(|| panic!("{option} states no minimum"));
            assert!(
                !admits(document(path, minimum - 1)),
                "{option} is admitted below the minimum the schema states"
            );
            assert!(
                admits(document(path, minimum)),
                "{option} is refused at the minimum the schema states"
            );

            // Absent for the message sizes, whose largest value is a channel refusing nothing.
            if let Some(maximum) = stated(&format!("{option}/maximum")) {
                assert!(
                    admits(document(path, maximum)),
                    "{option} is refused at the maximum the schema states"
                );
                assert!(
                    !admits(document(path, maximum + 1)),
                    "{option} is admitted above the maximum the schema states"
                );
            }
        }

        assert_eq!(
            stated("/$defs/GrpcOptions/properties/UserAgent/minLength"),
            Some(1)
        );
        assert!(!admits(r#"{"Grpc":{"UserAgent":""}}"#.to_owned()));
        assert!(admits(r#"{"Grpc":{"UserAgent":"a"}}"#.to_owned()));

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

        // Zero is "none", so that a later source can turn off a deadline an earlier one set; the
        // least a deadline can be is still a nanosecond.
        assert_eq!(
            schema
                .pointer("/$defs/GrpcOptions/properties/DefaultDeadlineSeconds/minimum")
                .and_then(serde_json::Value::as_f64),
            Some(0.0)
        );
        assert_eq!(
            config_of(br#"{"Grpc":{"DefaultDeadlineSeconds":0.0}}"#).default_deadline,
            None
        );
        assert!(!admits(
            r#"{"Grpc":{"DefaultDeadlineSeconds":5e-10}}"#.to_owned()
        ));
        assert!(!admits(
            r#"{"Grpc":{"DefaultDeadlineSeconds":18446744073709551616.0}}"#.to_owned()
        ));
        assert_eq!(
            config_of(br#"{"Grpc":{"DefaultDeadlineSeconds":1e-9}}"#).default_deadline,
            Some(Duration::from_nanos(1))
        );
        assert_eq!(config_of(b"{}").default_deadline, None);

        assert!(parse(br#"{"Transport":{"ConnectEagerly":true}}"#)
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
            // Whole seconds, as the operating system counts them, and zero is none.
            (
                "/$defs/TcpKeepaliveOptions/properties/IdleSeconds/minimum",
                r#"{"Transport":{"TcpKeepalive":{"IdleSeconds":N}}}"#,
            ),
            (
                "/$defs/TcpKeepaliveOptions/properties/IntervalSeconds/minimum",
                r#"{"Transport":{"TcpKeepalive":{"IdleSeconds":30,"IntervalSeconds":N}}}"#,
            ),
            (
                "/$defs/Http2FixedWindows/properties/StreamWindowSize/minimum",
                r#"{"Http2":{"Receive":{"Fixed":{"StreamWindowSize":N}}}}"#,
            ),
            (
                "/$defs/Http2FixedWindows/properties/ConnectionWindowSize/minimum",
                r#"{"Http2":{"Receive":{"Fixed":{"ConnectionWindowSize":N}}}}"#,
            ),
            (
                "/$defs/Http2SendOptions/properties/CoalescingBytes/minimum",
                r#"{"Http2":{"Send":{"CoalescingBytes":N}}}"#,
            ),
            (
                "/$defs/Http2SendOptions/properties/StreamBufferSize/minimum",
                r#"{"Http2":{"Send":{"StreamBufferSize":N}}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/MaxAttempts/minimum",
                r#"{"Grpc":{"Retry":{"MaxAttempts":N}}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/CallReplayBytes/minimum",
                r#"{"Grpc":{"Retry":{"CallReplayBytes":N}}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/ChannelReplayBytes/minimum",
                r#"{"Grpc":{"Retry":{"ChannelReplayBytes":N}}}"#,
            ),
        ] {
            let minimum = stated(pointer).unwrap_or_else(|| panic!("{pointer} states none"));
            let at = |value: i64| document.replace('N', &value.to_string());
            assert!(!admits(at(minimum - 1)), "{pointer}: below is admitted");
            assert!(admits(at(minimum)), "{pointer}: the minimum is refused");
        }
        for (pointer, document) in [
            (
                "/$defs/Http2Options/properties/KeepAliveTimeoutSeconds/minimum",
                r#"{"Http2":{"KeepAliveTimeoutSeconds":N}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/InitialBackoffSeconds/minimum",
                r#"{"Grpc":{"Retry":{"InitialBackoffSeconds":N}}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/MaxBackoffSeconds/minimum",
                r#"{"Grpc":{"Retry":{"InitialBackoffSeconds":1e-9,"MaxBackoffSeconds":N}}}"#,
            ),
            (
                "/$defs/RetryOptions/properties/BackoffMultiplier/minimum",
                r#"{"Grpc":{"Retry":{"BackoffMultiplier":N}}}"#,
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

        // These two state "none" as zero, which the schema's minimum is; what the engine admits
        // above zero is its own bound, a nanosecond, stated in the option's description.
        for (pointer, document, least) in [
            (
                "/$defs/Http2Options/properties/KeepAliveIntervalSeconds/minimum",
                r#"{"Http2":{"KeepAliveIntervalSeconds":N}}"#,
                1e-9,
            ),
            (
                "/$defs/Http2Options/properties/IdleTimeoutSeconds/minimum",
                r#"{"Http2":{"IdleTimeoutSeconds":N}}"#,
                1e-9,
            ),
        ] {
            assert_eq!(
                schema.pointer(pointer).and_then(serde_json::Value::as_f64),
                Some(0.0),
                "{pointer}"
            );
            let at = |value: f64| document.replace('N', &format!("{value:e}"));
            assert!(admits(at(0.0)), "{pointer}: zero is refused");
            assert!(admits(at(least)), "{pointer}: the least is refused");
            assert!(!admits(at(least / 2.0)), "{pointer}: below is admitted");
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
                        "ServerCertificates": {{ "CaPem": "{certificate}" }},
                        "ClientCertificate": {{ "Pem": {{ "Certificate": "{certificate}", "Key": "{key}" }} }}
                    }},
                    "TcpKeepalive": {{ "IdleSeconds": 30, "IntervalSeconds": 5, "Retries": 3 }}
                }},
                "Http2": {{
                    "KeepAliveIntervalSeconds": 10,
                    "KeepAliveTimeoutSeconds": 2.5,
                    "KeepAliveWhileIdle": true,
                    "Receive": {{ "Fixed": {{ "StreamWindowSize": 1048576, "ConnectionWindowSize": 3145728 }} }}
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

        let tcp = config.transport.tcp;
        assert_eq!(tcp.keepalive, Some(Duration::from_secs(30)));
        assert_eq!(tcp.keepalive_interval, Some(Duration::from_secs(5)));
        assert_eq!(tcp.keepalive_retries, Some(3));

        let http2 = config.transport.http2;
        assert_eq!(http2.keep_alive_interval, Some(Duration::from_secs(10)));
        assert_eq!(http2.keep_alive_timeout, Duration::from_millis(2500));
        assert!(http2.keep_alive_while_idle);
        assert_eq!(
            http2.receive_windows,
            ReceiveWindows::Fixed(FixedWindows {
                stream: 1_048_576,
                connection: 3_145_728
            })
        );

        let unsafe_document = br#"{"Transport":{"Tls":{"ServerCertificates":"None"}}}"#;
        let config = parse(unsafe_document)
            .expect("admissible")
            .into_channel_config("https://127.0.0.1:5000".parse().expect("a uri"));
        assert!(config.transport.tls.accept_any_server);
    }

    #[test]
    fn the_retry_codes_reach_the_engine_and_a_list_naming_none_is_refused() {
        use armonik_transport::grpc::GrpcStatusCode;

        let codes = |document: &[u8]| {
            config_of(document)
                .retry
                .expect("a retry policy")
                .retryable_codes
        };
        assert_eq!(codes(b"{}"), [GrpcStatusCode::Unavailable]);
        assert_eq!(
            codes(br#"{"Grpc":{"Retry":{"Codes":"GrpcClient"}}}"#),
            [
                GrpcStatusCode::Unavailable,
                GrpcStatusCode::Aborted,
                GrpcStatusCode::Unknown
            ]
        );
        assert_eq!(
            codes(br#"{"Grpc":{"Retry":{"Codes":{"List":["ABORTED"]}}}}"#),
            [GrpcStatusCode::Aborted]
        );

        let refused = parse(br#"{"Grpc":{"Retry":{"Codes":{"List":[]}}}}"#)
            .err()
            .expect("refused")
            .to_string();
        assert!(refused.starts_with("Grpc.Retry.Codes.List"), "{refused}");
    }

    #[test]
    fn the_rate_limit_reaches_the_engine_and_a_limit_that_starts_nothing_is_refused() {
        assert_eq!(config_of(b"{}").rate_limit, None);
        assert_eq!(
            config_of(br#"{"Grpc":{"Rate":{"Limit":{"Calls":100,"PerSeconds":0.25}}}}"#).rate_limit,
            Some(RateLimitConfig::new(100, Duration::from_millis(250)))
        );

        // Zero calls states no limit, over one an earlier source set, whatever else is stated.
        assert_eq!(
            config_of(br#"{"Grpc":{"Rate":{"Limit":{"Calls":0,"PerSeconds":1}}}}"#).rate_limit,
            None
        );
        assert_eq!(
            config_of(br#"{"Grpc":{"Rate":{"Limit":{"Calls":0}}}}"#).rate_limit,
            None
        );

        for (document, key) in [
            (
                &br#"{"Grpc":{"Rate":{"Limit":{"Calls":-1,"PerSeconds":1}}}}"#[..],
                "Grpc.Rate.Limit.Calls",
            ),
            (
                &br#"{"Grpc":{"Rate":{"Limit":{"Calls":1,"PerSeconds":0}}}}"#[..],
                "Grpc.Rate.Limit.PerSeconds",
            ),
            (
                &br#"{"Grpc":{"Rate":{"Limit":{"Calls":1}}}}"#[..],
                "Grpc.Rate.Limit.Calls and Grpc.Rate.Limit.PerSeconds are incoherent",
            ),
            (
                &br#"{"Grpc":{"Rate":{"Limit":{"PerSeconds":1}}}}"#[..],
                "Grpc.Rate.Limit.Calls and Grpc.Rate.Limit.PerSeconds are incoherent",
            ),
        ] {
            let refused = parse(document).err().expect("refused").to_string();
            assert!(refused.starts_with(key), "{refused}");
        }

        // The bounds the schema states are the ones refused above.
        let schema: serde_json::Value = serde_json::from_str(&armonik_transport::options::schema())
            .expect("the schema renders as JSON");
        let minimum = |option: &str| {
            schema
                .pointer(&format!(
                    "/$defs/RateLimitOptions/properties/{option}/minimum"
                ))
                .and_then(serde_json::Value::as_f64)
        };
        assert_eq!(minimum("Calls"), Some(0.0));
        assert_eq!(minimum("PerSeconds"), Some(1e-9));
    }

    #[test]
    fn a_channel_that_could_send_or_receive_no_message_is_refused() {
        // Every other size is a channel that refuses some messages; zero refuses all but the
        // empty ones, which is a configuration with no use.
        for way in ["Send", "Receive"] {
            let document =
                |max: i32| format!(r#"{{"Grpc":{{"{way}":{{"MaxMessageSize":{max}}}}}}}"#);
            assert!(parse(document(0).as_bytes()).is_err(), "{way}");
            assert!(parse(document(1).as_bytes()).is_ok(), "{way}");
        }
        assert_eq!(
            config_of(br#"{"Grpc":{"Send":{"MaxMessageSize":7}}}"#).max_send_message_size,
            Some(7)
        );
    }

    #[test]
    fn compression_reaches_the_channel_per_direction() {
        let none = config_of(b"{}");
        assert_eq!((none.send_encoding, none.accept_encodings), (None, vec![]));

        let sends = config_of(br#"{"Grpc":{"Send":{"Compression":"Gzip"}}}"#);
        assert_eq!(sends.send_encoding, Some(Encoding::Gzip));
        assert_eq!(sends.accept_encodings, vec![]);

        let accepts = config_of(br#"{"Grpc":{"Receive":{"Compression":["Gzip"]}}}"#);
        assert_eq!(accepts.send_encoding, None);
        assert_eq!(accepts.accept_encodings, vec![Encoding::Gzip]);

        let both = config_of(
            br#"{"Grpc":{"Send":{"Compression":"Zstd"},"Receive":{"Compression":["Deflate","Gzip","Zstd"]}}}"#,
        );
        assert_eq!(both.send_encoding, Some(Encoding::Zstd));
        assert_eq!(
            both.accept_encodings,
            vec![Encoding::Deflate, Encoding::Gzip, Encoding::Zstd],
            "in the order stated"
        );
        let empty = config_of(br#"{"Grpc":{"Receive":{"Compression":[]}}}"#);
        assert_eq!(empty.accept_encodings, vec![]);

        assert!(parse(br#"{"Grpc":{"Send":{"Compression":"Brotli"}}}"#).is_err());
        assert!(parse(br#"{"Grpc":{"Receive":{"Compression":"Gzip"}}}"#).is_err());
    }

    /// `None` sends no compression, over an encoding the defaults state, and is no encoding to
    /// accept.
    #[test]
    fn none_turns_the_send_compression_off_and_names_nothing_to_accept() {
        let none = config_of(br#"{"Grpc":{"Send":{"Compression":"None"}}}"#);
        assert_eq!(none.send_encoding, None);

        let defaults =
            defaults(br#"{"Grpc":{"Send":{"Compression":"Gzip"}}}"#).expect("valid defaults");
        let kept = parse_over(defaults.as_ref(), b"{}").expect("the default's");
        assert_eq!(
            kept.into_channel_config("http://127.0.0.1:1".parse().expect("a uri"))
                .send_encoding,
            Some(Encoding::Gzip)
        );
        let off = parse_over(
            defaults.as_ref(),
            br#"{"Grpc":{"Send":{"Compression":"None"}}}"#,
        )
        .expect("None over Gzip");
        assert_eq!(
            off.into_channel_config("http://127.0.0.1:1".parse().expect("a uri"))
                .send_encoding,
            None
        );

        for document in [
            &br#"{"Grpc":{"Receive":{"Compression":["None"]}}}"#[..],
            &br#"{"Grpc":{"Receive":{"Compression":["Gzip","None"]}}}"#[..],
        ] {
            let refused = parse(document).err().expect("refused").to_string();
            assert!(refused.starts_with("Grpc.Receive.Compression"), "{refused}");
            assert!(refused.contains("identity is always accepted"), "{refused}");
        }
        assert_eq!(
            config_of(br#"{"Grpc":{"Receive":{"Compression":[]}}}"#).accept_encodings,
            vec![]
        );
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
        assert_eq!(config.max_sends_in_flight, 1);
    }

    /// The loader logs it, as it logs one from any source; the channel is the one its other options
    /// make.
    #[test]
    fn an_option_spelled_wrong_is_ignored_rather_than_refused() {
        let settings = parse(br#"{"UserAgnt":"typo","Grpc":{"Host":{"Receive":{"Window":2}}}}"#)
            .expect("an unknown key is no refusal");
        assert_eq!(settings.delivery_credits(), 2);
    }

    #[test]
    fn an_option_spelled_as_the_wrong_type_is_refused() {
        // The schema says a number, so a string spelled like one is not the same document. A
        // reader that took it would make the schema a suggestion.
        assert!(parse(br#"{"Grpc":{"Host":{"Receive":{"Window":"2"}}}}"#).is_err());
        assert!(parse(br#"{"Grpc":{"Host":{"Receive":{"Window":2}}}}"#).is_ok());
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
            &br#"{"Grpc":{"Host":{"Receive":{"Window":0}}}}"#[..],
            &br#"{"Grpc":{"Host":{"Send":{"Window":0}}}}"#[..],
            &br#"{"Grpc":{"UserAgent":""}}"#[..],
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
            (
                &br#"{"Grpc":{"Host":{"Receive":{"Window":"2"}}}}"#[..],
                "Grpc.Host.Receive.Window",
            ),
            (
                &br#"{"Grpc":{"Host":{"Receive":{"Window":0}}}}"#[..],
                "Grpc.Host.Receive.Window",
            ),
            (
                &br#"{"Grpc":{"Host":{"Send":{"Window":0}}}}"#[..],
                "Grpc.Host.Send.Window",
            ),
            (
                &br#"{"Grpc":{"Send":{"MaxMessageSize":0}}}"#[..],
                "Grpc.Send.MaxMessageSize",
            ),
            (
                &br#"{"Grpc":{"Receive":{"MaxMessageSize":0}}}"#[..],
                "Grpc.Receive.MaxMessageSize",
            ),
            (
                &br#"{"Grpc":{"Host":{"Receive":{"CoalescingBytes":-1}}}}"#[..],
                "Grpc.Host.Receive.CoalescingBytes",
            ),
            (&br#"{"Grpc":{"UserAgent":""}}"#[..], "Grpc.UserAgent"),
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
                &br#"{"Transport":{"Tls":{"ServerCertificates":{"CaPem":"no/such/file.pem"}}}}"#[..],
                "Transport.Tls.ServerCertificates.CaPem",
            ),
            (
                &br#"{"Transport":{"TcpKeepalive":{"Retries":3}}}"#[..],
                "Transport.TcpKeepalive.Retries",
            ),
            (
                &br#"{"Http2":{"Receive":{"Fixed":{"ConnectionWindowSize":65534}}}}"#[..],
                "Http2.Receive.Fixed.ConnectionWindowSize",
            ),
            (
                &br#"{"Transport":{"Tls":{"ClientCertificate":{"P12":{"Path":"c.p12","Password":123456}}}}}"#
                    [..],
                "Transport.Tls.ClientCertificate.P12.Password",
            ),
            (
                &br#"{"Transport":{"Proxy":{"Url":{"Address":"https://proxy.test"}}}}"#[..],
                "Transport.Proxy.Url.Address",
            ),
            (
                &br#"{"Transport":{"Proxy":{"None":null,"System":{}}}}"#[..],
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
            &br#"{"Transport":{"Tls":{"ClientCertificate":{"P12":{"Path":"c.p12","Password":123456}}}}}"#[..],
            &br#"{"Transport":{"Tls":{"ClientCertificate":{"P12":{"Path":"c.p12","Password":-123456.5}}}}}"#[..],
            &br#"{"Transport":{"Tls":{"ClientCertificate":{"P12":{"Path":"c.p12","Password":["s3cret"]}}}}}"#[..],
            &br#"{"Transport":{"Tls":{"ClientCertificate":{"P12":{"Path":"c.p12","Password":{"s3cret":1}}}}}}"#[..],
        ] {
            let Err(refused) = parse(document) else {
                panic!("{} is admitted", String::from_utf8_lossy(document));
            };
            let said = refused.to_string();
            assert!(said.contains("Transport.Tls.ClientCertificate.P12.Password"), "{said}");
            assert!(!said.contains("123456"), "{said}");
            assert!(!said.contains("s3cret"), "{said}");
        }
    }

    #[test]
    fn a_window_past_what_the_schema_admits_is_refused() {
        let past = format!(
            r#"{{"Grpc":{{"Host":{{"Receive":{{"Window":{}}}}}}}}}"#,
            LARGEST_WINDOW as i64 + 1
        );
        assert!(parse(past.as_bytes()).is_err());
    }

    /// A channel's document is merged over the runtime's defaults option by option, a struct's
    /// options within it: what the channel states wins, and what it leaves out is the default's.
    #[test]
    fn a_channel_document_is_merged_over_the_defaults() {
        let defaults = defaults(
            br#"{"Grpc":{"Host":{"Receive":{"Window":2}}},"Http2":{"KeepAliveWhileIdle":true,"Receive":{"Fixed":{"StreamWindowSize":70000}}}}"#,
        )
        .expect("valid defaults");
        let settings = parse_over(
            defaults.as_ref(),
            br#"{"Http2":{"Receive":{"Fixed":{"StreamWindowSize":80000}}}}"#,
        )
        .expect("a valid merge");
        assert_eq!(settings.delivery_credits(), 2);
        let http2 = settings
            .into_channel_config("http://127.0.0.1:5000".parse().expect("an endpoint"))
            .transport
            .http2;
        assert!(http2.keep_alive_while_idle);
        assert_eq!(
            http2.receive_windows,
            ReceiveWindows::Fixed(FixedWindows {
                stream: 80_000,
                ..FixedWindows::default()
            })
        );

        // An alternative over the defaults' other one replaces it.
        let settings = parse_over(defaults.as_ref(), br#"{"Http2":{"Receive":"Adaptive"}}"#)
            .expect("a valid merge");
        let http2 = settings
            .into_channel_config("http://127.0.0.1:5000".parse().expect("an endpoint"))
            .transport
            .http2;
        assert_eq!(http2.receive_windows, ReceiveWindows::Adaptive);
    }

    /// Defaults are refused as a channel's document is, and empty ones are none.
    #[test]
    fn defaults_are_read_as_a_channel_document() {
        assert!(defaults(b"").expect("empty is none").is_none());
        for document in [
            &br#"{"Grpc":{"Host":{"Receive":{"Window":0}}}}"#[..],
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
        let defaults =
            defaults(br#"{"Grpc":{"Host":{"Receive":{"Window":2}}}}"#).expect("valid defaults");
        let settings = parse_over(
            defaults.as_ref(),
            br#"{"Grpc":{"Host":{"Receive":{"Window":null}}}}"#,
        )
        .expect("a valid merge");
        assert_eq!(settings.delivery_credits(), 2);
    }

    /// Bounds the defaults and the channel each state half of cumulate, and a merge where they
    /// disagree is refused as the merge's, the channel's own document being admitted alone.
    #[test]
    fn bounds_that_disagree_once_merged_refuse_the_merge() {
        let defaults =
            defaults(br#"{"Grpc":{"Retry":{"MaxBackoffSeconds":2}}}"#).expect("valid defaults");
        let own = br#"{"Grpc":{"Retry":{"InitialBackoffSeconds":3}}}"#;
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

    /// Options that cannot hold together once merged are refused as the merge's, naming the merged
    /// keys, whether or not the channel's document is incoherent alone; one the defaults complete
    /// is admitted.
    #[test]
    fn incoherence_is_a_fact_of_the_merge() {
        let incoherent =
            defaults(br#"{"Grpc":{"Rate":{"Limit":{"Calls":5}}}}"#).expect("a warning only");
        // Incoherent alone and merged: refused as the merge's.
        let Err(refused) = parse_over(incoherent.as_ref(), b"{}") else {
            panic!("incoherent defaults kept by the channel are admitted");
        };
        let said = refused.to_string();
        assert!(said.contains("Grpc.Rate.Limit.PerSeconds"), "{said}");
        assert!(
            said.ends_with("once merged over the runtime's ChannelDefaults"),
            "{said}"
        );
        // A document that is incoherent alone and completed by the defaults is admitted.
        let window = defaults(br#"{"Grpc":{"Rate":{"Limit":{"PerSeconds":2}}}}"#).expect("valid");
        assert!(parse(br#"{"Grpc":{"Rate":{"Limit":{"Calls":5}}}}"#).is_err());
        parse_over(
            window.as_ref(),
            br#"{"Grpc":{"Rate":{"Limit":{"Calls":5}}}}"#,
        )
        .expect("completed");
    }

    /// Another alternative than the default's, stated by the channel, replaces it whole.
    #[test]
    fn an_alternative_the_channel_states_replaces_the_defaults() {
        let defaults = defaults(
            br#"{"Transport":{"Proxy":{"Url":{"Address":"proxy.test:3128","Username":"alice"}}}}"#,
        )
        .expect("valid defaults");
        let settings = parse_over(defaults.as_ref(), br#"{"Transport":{"Proxy":"None"}}"#)
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

    /// A channel's document is checked alone before it is merged: credentials in a `Url`
    /// address are refused by the document's own path, whatever the defaults state.
    #[test]
    fn a_channel_document_is_checked_before_it_is_merged() {
        let defaults = defaults(
            br#"{"Transport":{"Proxy":{"Url":{"Address":"proxy.test:3128","Username":"alice"}}}}"#,
        )
        .expect("valid defaults");
        let Err(refused) = parse_over(
            defaults.as_ref(),
            br#"{"Transport":{"Proxy":{"Url":{"Address":"http://bob:s3cret@proxy.test:3128"}}}}"#,
        ) else {
            panic!("credentials in a Url address are admitted");
        };
        let said = refused.to_string();
        assert!(said.starts_with("Transport.Proxy.Url.Address"), "{said}");
        assert!(!said.contains("ChannelDefaults"), "{said}");
        assert!(!said.contains("s3cret"), "{said}");
    }

    /// A channel's own document is refused over the defaults as it would be alone.
    #[test]
    fn a_channel_document_is_refused_over_the_defaults_as_alone() {
        let defaults =
            defaults(br#"{"Grpc":{"Host":{"Receive":{"Window":2}}}}"#).expect("valid defaults");
        assert!(parse_over(
            defaults.as_ref(),
            br#"{"Grpc":{"Host":{"Receive":{"Window":0}}}}"#
        )
        .is_err());
        assert!(parse_over(defaults.as_ref(), b"not json").is_err());
    }
}
