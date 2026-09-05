use std::time::Duration;

use armonik_transport::grpc::GrpcChannelConfig;
use armonik_transport::http2::TransportConfig;
use armonik_transport::reexports::http::Uri;
use serde::Deserialize;
use tokio::sync::Semaphore;

// What a configuration that names neither gets. One each, because the header promises a host that
// asks for nothing a channel it can drive without ever holding two of anything.
const MAX_SENDS_IN_FLIGHT: usize = 1;
const DELIVERY_CREDITS: usize = 1;

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
    #[serde(default)]
    delivery_credits: Option<usize>,
    #[serde(default)]
    max_sends_in_flight: Option<usize>,
}

impl ChannelSettings {
    pub(crate) fn delivery_credits(&self) -> usize {
        self.delivery_credits.unwrap_or(DELIVERY_CREDITS)
    }

    pub(crate) fn max_sends_in_flight(&self) -> usize {
        self.max_sends_in_flight.unwrap_or(MAX_SENDS_IN_FLIGHT)
    }

    pub(crate) fn into_channel_config(self, endpoint: Uri) -> GrpcChannelConfig {
        let mut transport = TransportConfig::new(endpoint);
        if let Some(millis) = self.connect_timeout_ms {
            transport.connect_timeout = Duration::from_millis(millis);
        }

        let mut config = GrpcChannelConfig::new(transport);
        config.max_sends_in_flight = self.max_sends_in_flight();
        config.user_agent = self.user_agent;
        if let Some(max) = self.max_recv_message_size {
            config.max_recv_message_size = max;
        }
        config
    }
}

pub(crate) fn parse(json: &[u8]) -> Option<(ChannelSettings, Uri)> {
    let settings: ChannelSettings = serde_json::from_slice(json).ok()?;
    let endpoint = settings.endpoint.parse::<Uri>().ok()?;
    // Strictly below, because the send window's command queue is built one deeper than the
    // window and tokio refuses a queue past MAX_PERMITS. One rule for both windows: the delivery
    // side could take MAX_PERMITS itself, and a second bound differing by one would only make a
    // reader wonder which of the two was the mistake.
    let admits = |window: Option<usize>| {
        window.is_none_or(|window| window > 0 && window < Semaphore::MAX_PERMITS)
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
        // The boundary itself, which is where the queue built one deeper than the window lands
        // on the value tokio asserts against. One less is the largest that can work.
        let edge = Semaphore::MAX_PERMITS;
        assert!(parse(
            format!(r#"{{"endpoint":"http://h:1","max_sends_in_flight":{edge}}}"#).as_bytes()
        )
        .is_none());
        assert!(parse(
            format!(
                r#"{{"endpoint":"http://h:1","max_sends_in_flight":{}}}"#,
                edge - 1
            )
            .as_bytes()
        )
        .is_some());

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
