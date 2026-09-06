use armonik_transport::grpc::GrpcChannelConfig;
use armonik_transport::http2::TransportConfig;
use armonik_transport::options::{ChannelOptions, LARGEST_WINDOW};
use armonik_transport::reexports::http::Uri;

// What a configuration that names neither gets. One each, because the header promises a host that
// asks for nothing a channel it can drive without ever holding two of anything.
const MAX_SENDS_IN_FLIGHT: i32 = 1;
const DELIVERY_CREDITS: i32 = 1;

/// The options a host sent, read and found admissible.
pub(crate) struct ChannelSettings(ChannelOptions);

impl ChannelSettings {
    pub(crate) fn delivery_credits(&self) -> usize {
        self.0.delivery_credits.unwrap_or(DELIVERY_CREDITS) as usize
    }

    pub(crate) fn max_sends_in_flight(&self) -> usize {
        self.0.max_sends_in_flight.unwrap_or(MAX_SENDS_IN_FLIGHT) as usize
    }

    pub(crate) fn into_channel_config(self, endpoint: Uri) -> GrpcChannelConfig {
        let mut transport = TransportConfig::new(endpoint);
        if let Some(seconds) = self.0.transport.connect_timeout {
            transport.connect_timeout = seconds.into();
        }

        let mut config = GrpcChannelConfig::new(transport);
        config.max_sends_in_flight = self.max_sends_in_flight();
        config.user_agent = self.0.user_agent;
        if let Some(max) = self.0.max_receive_message_size {
            config.max_recv_message_size = max as usize;
        }
        config
    }
}

/// Reads the document, refusing exactly what the schema refuses.
///
/// Every bound checked here is stated in the schema the options type derives - a minimum, a
/// maximum, a minimum length - so a document a validator would reject is one this refuses too.
/// They are checked again rather than trusted: nothing obliges a host to have validated, and the
/// engine is what a bad value would break.
pub(crate) fn parse(json: &[u8]) -> Option<ChannelSettings> {
    let options: ChannelOptions = serde_json::from_slice(json).ok()?;

    let window =
        |asked: Option<i32>| asked.is_none_or(|asked| (1..=LARGEST_WINDOW).contains(&asked));
    if !window(options.delivery_credits) || !window(options.max_sends_in_flight) {
        return None;
    }

    // Zero is the size that means something, and it means no message can ever be received.
    if options.max_receive_message_size.is_some_and(|max| max < 1) {
        return None;
    }

    if options.user_agent.as_deref().is_some_and(str::is_empty) {
        return None;
    }

    // No dial could beat a timeout of zero, so it names a channel that can never connect.
    if options
        .transport
        .connect_timeout
        .is_some_and(|timeout| timeout.0 <= 0.0)
    {
        return None;
    }

    Some(ChannelSettings(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_of(json: &[u8]) -> GrpcChannelConfig {
        let settings = parse(json).expect("valid");
        settings.into_channel_config("http://127.0.0.1:5000".parse().expect("an endpoint"))
    }

    #[test]
    fn a_channel_that_could_receive_no_message_is_refused() {
        // Every other size is a channel that refuses some messages; zero refuses all of them,
        // which is a configuration with no use and a call that can only ever fail.
        assert!(parse(br#"{"MaxReceiveMessageSize":0}"#).is_none());
        assert!(parse(br#"{"MaxReceiveMessageSize":1}"#).is_some());
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
        assert!(parse(br#"{"UserAgnt":"typo"}"#).is_none());
    }

    #[test]
    fn an_option_spelled_as_the_wrong_type_is_refused() {
        // The schema says a number, so a string spelled like one is not the same document. A
        // reader that took it would make the schema a suggestion.
        assert!(parse(br#"{"DeliveryCredits":"2"}"#).is_none());
        assert!(parse(br#"{"DeliveryCredits":2}"#).is_some());
    }

    #[test]
    fn a_nested_unit_is_read_as_an_object() {
        let config = config_of(br#"{"Transport":{"ConnectTimeout":2.5}}"#);

        assert_eq!(
            config.transport.connect_timeout,
            std::time::Duration::from_millis(2500)
        );
    }

    #[test]
    fn a_bound_the_schema_states_is_a_bound_this_refuses() {
        for refused in [
            &br#"{"DeliveryCredits":0}"#[..],
            &br#"{"MaxSendsInFlight":0}"#[..],
            &br#"{"UserAgent":""}"#[..],
            &br#"{"Transport":{"ConnectTimeout":0.0}}"#[..],
        ] {
            assert!(
                parse(refused).is_none(),
                "{}",
                String::from_utf8_lossy(refused)
            );
        }
    }

    #[test]
    fn a_window_past_what_the_schema_admits_is_refused() {
        let past = format!(r#"{{"DeliveryCredits":{}}}"#, LARGEST_WINDOW as i64 + 1);
        assert!(parse(past.as_bytes()).is_none());
    }
}
