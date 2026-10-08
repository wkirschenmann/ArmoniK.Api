//! A call that declares AK_CALL_WAIT_FOR_READY: it waits for a server that is down rather than end
//! UNAVAILABLE, and the wait ends with the call's deadline, its cancellation, or its channel.

mod support;

use std::time::Duration;

use armonik_transport_ffi::*;
use support::host::*;
use support::{TestServer, ECHO};

const OK: i32 = 0;
const CANCELLED: i32 = 1;
const DEADLINE_EXCEEDED: i32 = 4;
const UNAVAILABLE: i32 = 14;

/// An endpoint nothing listens at.
fn closed_endpoint() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("its address");
    format!("http://{address}")
}

/// One attempt, so that a call that fails fast does so at once.
const NO_RETRY: &str = r#"{"Grpc":{"OutboundTraffic":{"Retry":{"None":true}}}}"#;

#[test]
fn a_call_that_declares_it_waits_for_a_server_that_comes_up() {
    let host = Host::start();
    let endpoint = closed_endpoint();
    let channel = host.channel_with(&endpoint, NO_RETRY);

    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_WAIT_FOR_READY);
    send_one(call, b"hello");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !host
            .recorder
            .kinds()
            .contains(&ak_event_kind::AK_EVENT_STATUS),
        "the call waits where it would fail"
    );

    let _server = TestServer::start_at(&endpoint);
    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(OK), "{}", seen.status_message());
    assert_eq!(seen.message_payloads(), vec![b"hello".to_vec()]);

    ak_channel_release(channel);
    host.stop();
}

#[test]
fn a_call_that_does_not_declare_it_ends_unavailable() {
    let host = Host::start();
    let channel = host.channel_with(&closed_endpoint(), NO_RETRY);

    let call = start_call(channel, ECHO, &[]);
    send_one(call, b"hello");
    let seen = host.recorder.await_terminal();
    assert_eq!(
        seen.status_code(),
        Some(UNAVAILABLE),
        "{}",
        seen.status_message()
    );

    ak_channel_release(channel);
    host.stop();
}

#[test]
fn a_waiting_call_ends_deadline_exceeded_at_its_deadline() {
    let host = Host::start();
    let channel = host.channel_with(&closed_endpoint(), NO_RETRY);

    let call = start_call_flagged_within(
        channel,
        ECHO,
        &[],
        AK_CALL_WAIT_FOR_READY,
        Duration::from_millis(500),
    );
    send_one(call, b"hello");
    let seen = host.recorder.await_terminal();
    assert_eq!(
        seen.status_code(),
        Some(DEADLINE_EXCEEDED),
        "{}",
        seen.status_message()
    );

    ak_channel_release(channel);
    host.stop();
}

#[test]
fn a_waiting_call_ends_cancelled_when_its_channel_is_released() {
    let host = Host::start();
    let channel = host.channel_with(&closed_endpoint(), NO_RETRY);

    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_WAIT_FOR_READY);
    send_one(call, b"hello");
    std::thread::sleep(Duration::from_millis(300));
    ak_channel_release(channel);

    let seen = host.recorder.await_terminal();
    assert_eq!(
        seen.status_code(),
        Some(CANCELLED),
        "{}",
        seen.status_message()
    );

    host.stop();
}

#[test]
fn a_waiting_call_ends_cancelled_when_it_is_cancelled() {
    let host = Host::start();
    let channel = host.channel_with(&closed_endpoint(), NO_RETRY);

    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_WAIT_FOR_READY);
    send_one(call, b"hello");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    let seen = host.recorder.await_terminal();
    assert_eq!(
        seen.status_code(),
        Some(CANCELLED),
        "{}",
        seen.status_message()
    );

    ak_channel_release(channel);
    host.stop();
}
