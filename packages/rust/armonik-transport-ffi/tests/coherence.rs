//! Options that cannot hold together are checked once they are merged: a runtime is created with
//! channel defaults that are incoherent, since a channel can still override them, and a channel
//! whose merged options are incoherent is refused, naming the keys.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport_ffi::*;
use support::host::{refused_over, Host};

const NOWHERE: &str = "http://127.0.0.1:1";

/// A rate limit of five requests with no window: each option is valid, and the two are not.
const NO_WINDOW: &str = r#"{"Grpc":{"Rate":{"Limit":{"Calls":5}}}}"#;

#[test]
fn a_runtime_is_created_with_incoherent_defaults_and_a_channel_that_keeps_them_is_refused() {
    let host = Host::with_channel_defaults(NO_WINDOW);

    let refused = host
        .try_channel(NOWHERE, "{}")
        .expect_err("the defaults reach the channel as they are");
    assert_eq!(refused.status, ak_status::AK_STATUS_INVALID_ARG);
    assert_eq!(refused.kind, ak_error_kind::AK_ERROR_CONFIG);
    for key in ["Grpc.Rate.Limit.Calls", "Grpc.Rate.Limit.PerSeconds"] {
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
    let host = Host::with_channel_defaults(NO_WINDOW);
    host.channel_with(NOWHERE, r#"{"Grpc":{"Rate":{"Limit":{"PerSeconds":2}}}}"#);
    // Or turns the limit off.
    host.channel_with(NOWHERE, r#"{"Grpc":{"Rate":{"Limit":{"Calls":0}}}}"#);
}

#[test]
fn what_the_defaults_lack_may_come_from_the_channel_and_not_the_other_way() {
    // A maximum backoff of 60 over the initial one a channel states: coherent once merged, though
    // the channel's document alone is not, against the engine's maximum of 5.
    {
        let host = Host::with_channel_defaults(r#"{"Grpc":{"Retry":{"MaxBackoffSeconds":60}}}"#);
        host.channel_with(
            NOWHERE,
            r#"{"Grpc":{"Retry":{"InitialBackoffSeconds":10}}}"#,
        );
    }

    // One runtime at a time: the first is gone.
    let host = Host::start();
    let refused = host
        .try_channel(
            NOWHERE,
            r#"{"Grpc":{"Retry":{"InitialBackoffSeconds":10}}}"#,
        )
        .expect_err("an initial backoff above the engine's maximum");
    for key in [
        "Grpc.Retry.InitialBackoffSeconds",
        "Grpc.Retry.MaxBackoffSeconds",
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
