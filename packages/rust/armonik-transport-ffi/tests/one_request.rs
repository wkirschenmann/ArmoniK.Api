//! A call that declares one request: its commit ends the sending, nothing is sent before it, no
//! WRITE_DONE comes, and the engine settles the send itself.

mod support;

use std::time::Duration;

use armonik_transport_ffi::*;
use support::host::*;
use support::{blob, flaky_seen, TestServer, ECHO, FLAKY};

const CANCELLED: i32 = 1;
const DEADLINE_EXCEEDED: i32 = 4;

fn commit(call: ak_handle, message: &[u8]) -> ak_status {
    let (status, buffer) = lend(call, message.len());
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { std::ptr::copy_nonoverlapping(message.as_ptr(), buffer.ptr, message.len()) };
    unsafe { ak_call_send_message(call, buffer, buffer.len, std::ptr::null_mut()) }
}

fn end_send(call: ak_handle) -> ak_status {
    unsafe { ak_call_end_send(call, std::ptr::null_mut()) }
}

/// The commit sends the request and ends the sending: the call is answered with no end of the
/// sending and no acquittal, and what the send held is given back.
#[test]
fn the_commit_sends_the_request_and_ends_the_sending() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
    assert_eq!(
        end_send(call),
        ak_status::AK_STATUS_INVALID_STATE,
        "no end before the request: a call with none is not a call gRPC has"
    );
    assert_eq!(commit(call, b"hello"), ak_status::AK_STATUS_OK);
    assert_eq!(end_send(call), ak_status::AK_STATUS_INVALID_STATE);
    let (status, _) = lend(call, 1);
    assert_eq!(
        status,
        ak_status::AK_STATUS_INVALID_STATE,
        "no second lend, and no SLOT_BUSY waiting for an acquittal that never comes"
    );

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(seen.message_payloads(), vec![b"hello".to_vec()]);
    assert!(
        !seen.kinds().contains(&ak_event_kind::AK_EVENT_WRITE_DONE),
        "{seen:?}"
    );
    support::await_call_reclaimed(call);
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    fixture.close();
}

/// The empty request is the prefix alone, and is answered as one.
#[test]
fn the_empty_request_is_sent_with_no_buffer() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
    assert_eq!(
        unsafe { ak_call_send_message(call, support::empty_buffer(), 0, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(seen.message_payloads(), vec![Vec::<u8>::new()]);
    fixture.close();
}

/// A call cancelled before its commit ends without a response, and the buffer the host still
/// holds stays the host's: the commit is refused, and the buffer is given back.
#[test]
fn a_call_cancelled_before_its_commit_ends_and_refuses_the_commit() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
    let (status, buffer) = lend(call, 5);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(CANCELLED));
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, buffer.len, std::ptr::null_mut()) },
        ak_status::AK_STATUS_INVALID_STATE
    );
    unsafe { ak_return_call_buffer(buffer) };
    support::await_call_reclaimed(call);
    fixture.close();
}

/// Nothing watches the deadline of a call not yet committed: the commit past it is accepted, and
/// the call ends DEADLINE_EXCEEDED without sending.
#[test]
fn a_commit_past_the_deadline_is_accepted_and_the_call_ends_deadline_exceeded() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let call = start_call_flagged_within(
        channel,
        ECHO,
        &[],
        AK_CALL_ONE_REQUEST,
        Duration::from_millis(1),
    );
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        host.recorder.len(),
        0,
        "no task watched the deadline before the commit"
    );
    assert_eq!(commit(call, b"late"), ak_status::AK_STATUS_OK);

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(DEADLINE_EXCEEDED));
    assert!(seen.message_payloads().is_empty(), "nothing was sent");
    support::await_call_reclaimed(call);
    fixture.close();
}

/// A retried call sends its one request again, the same framed buffer, on each attempt.
#[test]
fn a_retried_call_sends_its_request_again() {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel_with(
        &server.endpoint,
        r#"{"Grpc":{"Retry":{"InitialBackoffSeconds":0.01,"MaxBackoffSeconds":0.05}}}"#,
    );
    let metadata = blob(&[
        (b"x-flaky-key", b"one-request-again"),
        (b"x-fail-times", b"1"),
    ]);
    let call = start_call_flagged(channel, FLAKY, &metadata, AK_CALL_ONE_REQUEST);
    assert_eq!(commit(call, b"hello"), ak_status::AK_STATUS_OK);

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(seen.message_payloads(), vec![b"hello".to_vec()]);
    assert_eq!(flaky_seen("one-request-again").len(), 2);
    ak_channel_release(channel);
    host.stop();
}
