//! Options that cannot hold together are checked once they are merged: a runtime is created with
//! channel defaults that are incoherent, since a channel can still override them, and a channel
//! whose merged options are incoherent is refused, naming the keys.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport_ffi::*;
use support::host::{refused_over, Host};

const NOWHERE: &str = "http://127.0.0.1:1";

/// A keepalive probing every five seconds with no idle time to start from: each option is valid,
/// and the two are not.
const NO_IDLE: &str = r#"{"Transport":{"TcpKeepalive":{"IntervalSeconds":5}}}"#;

/// An initial backoff of 500 seconds, above the 120 the backoff grows to by default.
const BACKOFF_ABOVE_MAXIMUM: &str = r#"{"Grpc":{"OutboundTraffic":{"Retry":{"ExponentialBackoff":{"InitialBackoffSeconds":500}}}}}"#;

#[test]
fn a_runtime_is_created_with_incoherent_defaults_and_a_channel_that_keeps_them_is_refused() {
    let host = Host::with_channel_defaults(NO_IDLE);

    let refused = host
        .try_channel(NOWHERE, "{}")
        .expect_err("the defaults reach the channel as they are");
    assert_eq!(refused.status, ak_status::AK_STATUS_INVALID_ARG);
    assert_eq!(refused.kind, ak_error_kind::AK_ERROR_CONFIG);
    for key in [
        "Transport.TcpKeepalive.IdleSeconds",
        "Transport.TcpKeepalive.IntervalSeconds",
    ] {
        assert!(refused.detail.contains(key), "{}", refused.detail);
    }
    assert!(refused.detail.contains("incoherent"), "{}", refused.detail);
    assert!(
        refused
            .detail
            .contains("once merged over the runtime's ChannelDefaults"),
        "{}",
        refused.detail
    );
}

#[test]
fn a_channel_that_completes_the_defaults_is_opened() {
    let host = Host::with_channel_defaults(NO_IDLE);
    host.channel_with(
        NOWHERE,
        r#"{"Transport":{"TcpKeepalive":{"IdleSeconds":30}}}"#,
    );
    // Or turns the keepalive off.
    host.channel_with(
        NOWHERE,
        r#"{"Transport":{"TcpKeepalive":{"IdleSeconds":0}}}"#,
    );
}

#[test]
fn what_the_defaults_lack_may_come_from_the_channel_and_not_the_other_way() {
    // A maximum backoff of 600 over the initial one a channel states: coherent once merged, though
    // the channel's document alone is not, against the engine's maximum of 120.
    {
        let host = Host::with_channel_defaults(
            r#"{"Grpc":{"OutboundTraffic":{"Retry":{"ExponentialBackoff":{"MaxBackoffSeconds":600}}}}}"#,
        );
        host.channel_with(NOWHERE, BACKOFF_ABOVE_MAXIMUM);
    }

    // One runtime at a time: the first is gone.
    let host = Host::start();
    let refused = host
        .try_channel(NOWHERE, BACKOFF_ABOVE_MAXIMUM)
        .expect_err("an initial backoff above the engine's maximum");
    for key in [
        "Grpc.OutboundTraffic.Retry.ExponentialBackoff.InitialBackoffSeconds",
        "Grpc.OutboundTraffic.Retry.ExponentialBackoff.MaxBackoffSeconds",
    ] {
        assert!(refused.detail.contains(key), "{}", refused.detail);
    }
}

#[test]
fn a_value_that_is_wrong_by_itself_is_refused_with_the_defaults_too() {
    // Not an incoherence: no channel's options can mend a value out of its bounds.
    assert_eq!(
        refused_over(r#"{"Transport":{"TcpKeepalive":{"IdleSeconds":-1}}}"#),
        ak_status::AK_STATUS_INVALID_ARG
    );
}
