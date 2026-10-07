//! The delivery window a channel reports: the one its own document and the runtime's channel
//! defaults settle between them, so a host need not know the defaults to size what it holds.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport_ffi::*;
use support::host::Host;

const NOWHERE: &str = "http://127.0.0.1:1";

fn window(channel: ak_handle) -> u32 {
    let mut window = 0;
    let status = unsafe { ak_channel_delivery_window(channel, &mut window, std::ptr::null_mut()) };
    assert_eq!(status, ak_status::AK_STATUS_OK);
    window
}

#[test]
fn a_channel_that_states_nothing_has_the_librarys_window() {
    let host = Host::start();
    assert_eq!(window(host.channel(NOWHERE)), 4);
}

#[test]
fn a_channel_takes_the_runtimes_default_where_its_own_document_is_silent() {
    let host = Host::with_channel_defaults(r#"{"Grpc":{"Host":{"Receive":{"Window":7}}}}"#);
    assert_eq!(window(host.channel(NOWHERE)), 7);
}

#[test]
fn a_channels_own_window_wins_over_the_runtimes_default() {
    let host = Host::with_channel_defaults(r#"{"Grpc":{"Host":{"Receive":{"Window":7}}}}"#);
    let channel = host.channel_with(NOWHERE, r#"{"Grpc":{"Host":{"Receive":{"Window":2}}}}"#);
    assert_eq!(window(channel), 2);
}

#[test]
fn a_stale_handle_and_a_null_out_are_refused_and_leave_the_out_alone() {
    let host = Host::start();
    let channel = host.channel(NOWHERE);

    let mut written = 99;
    let status =
        unsafe { ak_channel_delivery_window(AK_HANDLE_NONE, &mut written, std::ptr::null_mut()) };
    assert_eq!(status, ak_status::AK_STATUS_HANDLE_STALE);
    assert_eq!(written, 99);

    let status =
        unsafe { ak_channel_delivery_window(channel, std::ptr::null_mut(), std::ptr::null_mut()) };
    assert_eq!(status, ak_status::AK_STATUS_INVALID_ARG);
}
