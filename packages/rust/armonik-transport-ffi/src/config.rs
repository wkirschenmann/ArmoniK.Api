//! The channel configuration, as the host sends it.

use std::time::Duration;

use armonik_transport::grpc::GrpcChannelConfig;
use armonik_transport::http2::TransportConfig;
use armonik_transport::reexports::http::Uri;
use serde::Deserialize;

/// How many buffers one call may have out at once.
pub(crate) const MAX_SENDS_IN_FLIGHT: u32 = 1;

/// How many delivered payloads one call may have unconsumed at once.
pub(crate) const DELIVERY_CREDITS: usize = 1;

/// What the JSON says. Unknown fields are refused rather than ignored: an option spelled wrong
/// and dropped in silence is the failure mode this whole configuration path exists to avoid.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChannelSettings {
    endpoint: String,
    #[serde(default)]
    connect_timeout_ms: Option<u64>,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    max_recv_message_size: Option<usize>,
}

impl ChannelSettings {
    pub(crate) fn into_channel_config(self) -> GrpcChannelConfig {
        let mut transport = TransportConfig::new(
            self.endpoint
                .parse::<Uri>()
                .expect("the endpoint parsed when the settings were read"),
        );
        if let Some(millis) = self.connect_timeout_ms {
            transport.connect_timeout = Duration::from_millis(millis);
        }

        let mut config = GrpcChannelConfig::new(transport);
        config.user_agent = self.user_agent;
        config.max_sends_in_flight = MAX_SENDS_IN_FLIGHT as usize;
        if let Some(max) = self.max_recv_message_size {
            config.max_recv_message_size = max;
        }
        config
    }
}

/// Reads the configuration, or refuses it.
pub(crate) fn parse(json: &[u8]) -> Option<ChannelSettings> {
    let settings: ChannelSettings = serde_json::from_slice(json).ok()?;
    // Parsed here so `into_channel_config` cannot be reached with an endpoint that is not a URI.
    settings.endpoint.parse::<Uri>().ok()?;
    Some(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_is_the_only_thing_a_configuration_must_carry() {
        let settings = parse(br#"{"endpoint":"http://127.0.0.1:5000"}"#).expect("valid");
        let config = settings.into_channel_config();

        assert_eq!(config.transport.endpoint.to_string(), "http://127.0.0.1:5000/");
        assert_eq!(config.max_sends_in_flight, MAX_SENDS_IN_FLIGHT as usize);
    }

    #[test]
    fn an_option_spelled_wrong_is_refused_rather_than_ignored() {
        assert!(parse(br#"{"endpoint":"http://x:1","user_agnt":"typo"}"#).is_none());
    }

    #[test]
    fn an_endpoint_that_is_not_a_uri_is_refused_before_anything_is_built() {
        assert!(parse(br#"{"endpoint":"not a uri"}"#).is_none());
        assert!(parse(b"{}").is_none());
        assert!(parse(b"not json").is_none());
    }

    #[test]
    fn what_the_json_sets_reaches_the_channel() {
        let config = parse(
            br#"{"endpoint":"http://h:1","connect_timeout_ms":250,
                 "user_agent":"probe/1","max_recv_message_size":99}"#,
        )
        .expect("valid")
        .into_channel_config();

        assert_eq!(config.transport.connect_timeout, Duration::from_millis(250));
        assert_eq!(config.user_agent.as_deref(), Some("probe/1"));
        assert_eq!(config.max_recv_message_size, 99);
    }
}
