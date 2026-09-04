//! The channel configuration, as the host sends it.

use std::time::Duration;

use armonik_transport::grpc::GrpcChannelConfig;
use armonik_transport::http2::TransportConfig;
use armonik_transport::reexports::http::Uri;
use serde::Deserialize;
use tokio::sync::Semaphore;

/// The two windows the header calls mirrors of each other, and their defaults, together: the
/// send window bounds the buffers a call may have out, the delivery window the payloads.
const MAX_SENDS_IN_FLIGHT: usize = 1;
const DELIVERY_CREDITS: usize = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
/// What the JSON says. Unknown fields are refused rather than ignored: an option spelled wrong
/// and dropped in silence is the failure this configuration path exists to avoid.
pub(crate) struct ChannelSettings {
    endpoint: String,
    #[serde(default)]
    connect_timeout_ms: Option<u64>,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    max_recv_message_size: Option<usize>,
    #[serde(default)]
    delivery_credits: Option<usize>,
    #[serde(default)]
    max_sends_in_flight: Option<usize>,
}

impl ChannelSettings {
    /// The ABI's delivery window for calls of this channel. The host sizes its own per-call
    /// queue from it, which is why it is a channel option and not something negotiated later.
    pub(crate) fn delivery_credits(&self) -> usize {
        self.delivery_credits.unwrap_or(DELIVERY_CREDITS)
    }

    /// The send window's mirror of `delivery_credits`: how many buffers a call of this channel
    /// may have out at once, counting those being filled and those awaiting their WRITE_DONE.
    pub(crate) fn max_sends_in_flight(&self) -> usize {
        self.max_sends_in_flight.unwrap_or(MAX_SENDS_IN_FLIGHT)
    }

    /// The engine's shape of these settings, with the endpoint `parse` already read.
    pub(crate) fn into_channel_config(self, endpoint: Uri) -> GrpcChannelConfig {
        let mut transport = TransportConfig::new(endpoint);
        if let Some(millis) = self.connect_timeout_ms {
            transport.connect_timeout = Duration::from_millis(millis);
        }

        let mut config = GrpcChannelConfig::new(transport);
        // Read before its neighbour moves out of `self`.
        config.max_sends_in_flight = self.max_sends_in_flight();
        config.user_agent = self.user_agent;
        if let Some(max) = self.max_recv_message_size {
            config.max_recv_message_size = max;
        }
        config
    }
}

/// Reads the configuration, or refuses it. The endpoint comes back parsed, so nothing
/// downstream has to parse it again or answer for it not being a URI.
pub(crate) fn parse(json: &[u8]) -> Option<(ChannelSettings, Uri)> {
    let settings: ChannelSettings = serde_json::from_slice(json).ok()?;
    let endpoint = settings.endpoint.parse::<Uri>().ok()?;
    // Each window is a semaphore of that many permits: zero admits nothing at all, and a value
    // past what a semaphore can hold would be a panic rather than a refusal.
    let admits = |window: Option<usize>| {
        window.is_none_or(|window| window > 0 && window <= Semaphore::MAX_PERMITS)
    };
    if !admits(settings.delivery_credits) || !admits(settings.max_sends_in_flight) {
        return None;
    }
    Some((settings, endpoint))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_of(json: &[u8]) -> GrpcChannelConfig {
        let (settings, endpoint) = parse(json).expect("valid");
        settings.into_channel_config(endpoint)
    }

    #[test]
    fn the_endpoint_is_the_only_thing_a_configuration_must_carry() {
        let config = config_of(br#"{"endpoint":"http://127.0.0.1:5000"}"#);

        assert_eq!(
            config.transport.endpoint.to_string(),
            "http://127.0.0.1:5000/"
        );
        assert_eq!(config.max_sends_in_flight, MAX_SENDS_IN_FLIGHT);
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
    fn the_delivery_window_defaults_to_the_abi_depth_and_zero_is_refused() {
        assert_eq!(
            parse(br#"{"endpoint":"http://h:1"}"#)
                .expect("valid")
                .0
                .delivery_credits(),
            DELIVERY_CREDITS
        );
        assert_eq!(
            parse(br#"{"endpoint":"http://h:1","delivery_credits":4}"#)
                .expect("valid")
                .0
                .delivery_credits(),
            4
        );
        assert!(parse(br#"{"endpoint":"http://h:1","delivery_credits":0}"#).is_none());
    }

    #[test]
    fn the_send_window_is_the_mirror_of_the_delivery_one() {
        assert_eq!(
            parse(br#"{"endpoint":"http://h:1"}"#)
                .expect("valid")
                .0
                .max_sends_in_flight(),
            MAX_SENDS_IN_FLIGHT
        );
        assert_eq!(
            config_of(br#"{"endpoint":"http://h:1","max_sends_in_flight":3}"#).max_sends_in_flight,
            3
        );
        assert!(parse(br#"{"endpoint":"http://h:1","max_sends_in_flight":0}"#).is_none());
    }

    #[test]
    fn a_window_past_what_a_semaphore_holds_is_refused_rather_than_panicked_on() {
        let past = Semaphore::MAX_PERMITS + 1;
        assert!(parse(
            format!(r#"{{"endpoint":"http://h:1","delivery_credits":{past}}}"#).as_bytes()
        )
        .is_none());
        assert!(parse(
            format!(r#"{{"endpoint":"http://h:1","max_sends_in_flight":{past}}}"#).as_bytes()
        )
        .is_none());
    }

    #[test]
    fn what_the_json_sets_reaches_the_channel() {
        let config = config_of(
            br#"{"endpoint":"http://h:1","connect_timeout_ms":250,
                 "user_agent":"probe/1","max_recv_message_size":99}"#,
        );

        assert_eq!(config.transport.connect_timeout, Duration::from_millis(250));
        assert_eq!(config.user_agent.as_deref(), Some("probe/1"));
        assert_eq!(config.max_recv_message_size, 99);
    }
}
