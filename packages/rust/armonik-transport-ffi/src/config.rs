use std::fmt;
use std::time::Duration;

use armonik_transport::grpc::GrpcChannelConfig;
use armonik_transport::http2::TransportConfig;
use armonik_transport::options::{ChannelOptions, LARGEST_WINDOW};
use armonik_transport::reexports::http::Uri;

// What a configuration that names neither gets: one each, the smallest window either admits.
const MAX_SENDS_IN_FLIGHT: i32 = 1;
const DELIVERY_CREDITS: i32 = 1;

/// The options a host sent, read and found admissible.
///
/// The timeout is held as the `Duration` it became rather than as the number it was written as:
/// converting once, where the document is refused, is what leaves nothing here that can fail.
pub(crate) struct ChannelSettings {
    options: ChannelOptions,
    connect_timeout: Option<Duration>,
}

impl ChannelSettings {
    pub(crate) fn delivery_credits(&self) -> usize {
        self.options.delivery_credits.unwrap_or(DELIVERY_CREDITS) as usize
    }

    pub(crate) fn max_sends_in_flight(&self) -> usize {
        self.options
            .max_sends_in_flight
            .unwrap_or(MAX_SENDS_IN_FLIGHT) as usize
    }

    pub(crate) fn into_channel_config(self, endpoint: Uri) -> GrpcChannelConfig {
        let mut transport = TransportConfig::new(endpoint);
        if let Some(connect_timeout) = self.connect_timeout {
            transport.connect_timeout = connect_timeout;
        }

        let mut config = GrpcChannelConfig::new(transport);
        config.max_sends_in_flight = self.max_sends_in_flight();
        config.user_agent = self.options.user_agent;
        if let Some(max) = self.options.max_receive_message_size {
            config.max_recv_message_size = max as usize;
        }
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
    /// An empty user agent.
    EmptyUserAgent,
    /// A connect timeout no `Duration` holds, or one below what it holds.
    ConnectTimeout { seconds: f64 },
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
            Self::EmptyUserAgent => f.write_str("UserAgent is empty, and has to name something"),
            Self::ConnectTimeout { seconds } => write!(
                f,
                "Transport.ConnectTimeoutSeconds is {seconds}, and has to be at least 1e-9 and \
                 less than 2^64"
            ),
        }
    }
}

impl std::error::Error for ConfigRefusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Document(error) => Some(error.inner()),
            _ => None,
        }
    }
}

/// Reads the document, refusing exactly what the schema refuses, and saying over which key.
///
/// Every bound checked here is stated in the schema the options type derives - a minimum, a
/// maximum, a minimum length - so a document a validator would reject is one this refuses too.
/// They are checked again rather than trusted: nothing obliges a host to have validated, and the
/// engine is what a bad value would break.
pub(crate) fn parse(json: &[u8]) -> Result<ChannelSettings, ConfigRefusal> {
    let options: ChannelOptions =
        serde_path_to_error::deserialize(&mut serde_json::Deserializer::from_slice(json))
            .map_err(ConfigRefusal::Document)?;

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

    if options.user_agent.as_deref().is_some_and(str::is_empty) {
        return Err(ConfigRefusal::EmptyUserAgent);
    }

    // Below a nanosecond is refused, as the schema's `minimum` refuses it: `Duration` holds
    // nothing finer, so the conversion could round it to zero, which no dial could beat. And a
    // number is not yet a duration: `Duration` holds no value past its own range either, so the
    // conversion is what says whether the document named one.
    let connect_timeout = match options.transport.connect_timeout_seconds {
        None => None,
        Some(seconds) => {
            let refused = ConfigRefusal::ConnectTimeout { seconds: seconds.0 };
            if seconds.0 < 1e-9 {
                return Err(refused);
            }
            Some(Duration::try_from(seconds).map_err(|_| refused)?)
        }
    };

    Ok(ChannelSettings {
        options,
        connect_timeout,
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
            stated("/$defs/TransportOptions/properties/ConnectTimeoutSeconds/description"),
            config.transport.connect_timeout.as_secs_f64()
        );
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
    fn a_window_past_what_the_schema_admits_is_refused() {
        let past = format!(r#"{{"DeliveryCredits":{}}}"#, LARGEST_WINDOW as i64 + 1);
        assert!(parse(past.as_bytes()).is_err());
    }
}
