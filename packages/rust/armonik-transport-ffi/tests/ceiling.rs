//! Received messages count against the runtime's ceiling: reads wait past the first threshold, a
//! message past the second ends its call, and a send refused for room is woken by a release. A
//! call whose reads wait still ends at its deadline. A lend of no bytes is refused: an empty
//! message takes none.

mod support;

use std::time::{Duration, Instant};

use armonik_transport_ffi::*;
use support::host::{lend, memory_usage, start_call, start_call_flagged, start_call_within, Host};
use support::{blob, empty_buffer, TestServer, ECHO, SLOW};

/// Answers with the messages `x-sizes` lists, so a response can outweigh its request.
const SIZED: &str = "/raw/Sized";

/// Room for the head and every message of a call: what the host holds is then what the ceiling
/// decides, not the delivery window.
const CREDITS: &str = r#"{"Grpc":{"Host":{"Receive":{"Window":8}}}}"#;

const DEADLINE_EXCEEDED: i32 = 4;
const RESOURCE_EXHAUSTED: i32 = 8;

fn sized(sizes: &str) -> Vec<u8> {
    blob(&[(b"x-sizes", sizes.as_bytes())])
}

fn status_count(host: &Host) -> usize {
    host.recorder
        .kinds()
        .iter()
        .filter(|kind| **kind == ak_event_kind::AK_EVENT_STATUS)
        .count()
}

/// Until no event has arrived for a while: the reads admitted have landed, the others wait.
fn settle(host: &Host) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = host.recorder.len();
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now = host.recorder.len();
        if now == seen || Instant::now() > deadline {
            return;
        }
        seen = now;
    }
}

#[test]
fn a_received_message_counts_against_the_ceiling_until_it_is_consumed() {
    let server = TestServer::start();
    let host = Host::with_ceiling(64);
    host.recorder.hold_payloads();
    let channel = host.channel_with(&server.endpoint, CREDITS);

    start_call(channel, SIZED, &sized("40"));
    host.recorder.await_terminal();
    assert_eq!(
        memory_usage(host.runtime).bytes_used,
        40,
        "the message the host holds"
    );

    host.recorder.consume_all();
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);

    ak_channel_release(channel);
    host.stop();
}

/// A one-response call whose message takes the count to the first threshold still gets its status
/// while the host holds the message: its status is read without the admission, which would wait
/// for the message to come back - and a host that gives it back with the status never would.
#[test]
fn a_one_response_message_at_the_threshold_still_gets_its_status() {
    let server = TestServer::start();
    let host = Host::with_ceiling(64);
    host.recorder.hold_payloads();
    let channel = host.channel_with(&server.endpoint, CREDITS);

    start_call_flagged(channel, SIZED, &sized("64"), AK_CALL_ONE_RESPONSE);
    let seen = host.recorder.await_terminal();

    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(seen.message_payloads().len(), 1);
    assert_eq!(
        memory_usage(host.runtime).bytes_used,
        64,
        "the message the host holds"
    );

    host.recorder.consume_all();
    ak_channel_release(channel);
    host.stop();
}

#[test]
fn a_message_past_the_second_threshold_ends_its_call_with_resource_exhausted() {
    let server = TestServer::start();
    let host = Host::with_ceilings(64, 64);
    host.recorder.hold_payloads();
    let channel = host.channel_with(&server.endpoint, CREDITS);

    start_call(channel, SIZED, &sized("40,40"));
    let seen = host.recorder.await_terminal();

    assert_eq!(seen.status_code(), Some(RESOURCE_EXHAUSTED));
    assert!(
        seen.status_message()
            .contains("ceiling on received messages"),
        "{}",
        seen.status_message()
    );
    assert_eq!(
        seen.message_payloads().len(),
        1,
        "the first message was admitted and kept"
    );
    assert_eq!(
        memory_usage(host.runtime).bytes_used,
        40,
        "the second was freed at once"
    );

    host.recorder.consume_all();
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    ak_channel_release(channel);
    host.stop();
}

#[test]
fn calls_the_host_does_not_read_wait_and_none_passes_the_second_threshold() {
    // Room past the first threshold for one message of every call: calls admitted together below
    // it may each take one more, and with that room none is cut off.
    let calls = 6;
    let hard = 64 + calls as u64 * 30;
    let server = TestServer::start();
    let host = Host::with_ceilings(64, hard);
    host.recorder.hold_payloads();
    let channel = host.channel_with(&server.endpoint, CREDITS);

    for _ in 0..calls {
        start_call(channel, SIZED, &sized("30,30,30"));
    }
    settle(&host);

    let usage = memory_usage(host.runtime);
    assert!(usage.bytes_used <= hard, "{usage:?}");
    assert!(
        status_count(&host) < calls,
        "some calls wait for the host to give back what it holds"
    );

    // Given back as it arrives, every call ends: each release admits the reads it held back.
    let deadline = Instant::now() + Duration::from_secs(10);
    while status_count(&host) < calls {
        assert!(Instant::now() < deadline, "saw {:?}", host.recorder.kinds());
        host.recorder.consume_all();
        assert!(memory_usage(host.runtime).bytes_used <= hard);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        host.recorder
            .kinds()
            .iter()
            .filter(|kind| **kind == ak_event_kind::AK_EVENT_MESSAGE)
            .count()
            == calls * 3,
        "every message arrived: no call was cut off"
    );

    host.recorder.consume_all();
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    ak_channel_release(channel);
    host.stop();
}

#[test]
fn a_send_refused_for_room_is_woken_when_a_received_message_is_given_back() {
    // A call that sends one request has no task until something needs it: the refusal is what
    // spawns it, since the wake-up is the task's to raise.
    for flags in [0, AK_CALL_ONE_REQUEST] {
        woken_for_room(flags);
    }
}

fn woken_for_room(flags: u32) {
    let server = TestServer::start();
    let host = Host::with_ceiling(64);
    host.recorder.hold_payloads();
    let channel = host.channel_with(&server.endpoint, CREDITS);

    start_call(channel, SIZED, &sized("40"));
    host.recorder.await_terminal();

    let writer = start_call_flagged(channel, ECHO, &[], flags);
    assert_eq!(lend(writer, 40).0, ak_status::AK_STATUS_BUDGET_BUSY);
    assert!(
        !host
            .recorder
            .kinds()
            .contains(&ak_event_kind::AK_EVENT_BUDGET_WAKE),
        "nothing was given back yet"
    );

    host.recorder.consume_all();
    host.recorder.await_budget_wake();

    let (status, buffer) = lend(writer, 40);
    assert_eq!(
        status,
        ak_status::AK_STATUS_OK,
        "the wake-up found the room"
    );
    unsafe { ak_return_call_buffer(buffer) };

    assert_eq!(
        unsafe { ak_call_cancel(writer, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminals(2);
    host.recorder.consume_all();
    ak_channel_release(channel);
    host.stop();
}

/// The ceiling held whole, so no call reads: the deadline still ends the one that waits.
#[test]
fn a_call_whose_reads_wait_still_ends_at_its_deadline() {
    let server = TestServer::start();
    let host = Host::with_ceiling(64);
    let channel = host.channel_with(&server.endpoint, CREDITS);

    let holder = start_call(channel, ECHO, &[]);
    let (status, buffer) = lend(holder, 64);
    assert_eq!(status, ak_status::AK_STATUS_OK);

    start_call_within(channel, SLOW, &[], Duration::from_millis(300));
    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(DEADLINE_EXCEEDED));

    unsafe { ak_return_call_buffer(buffer) };
    assert_eq!(
        unsafe { ak_call_cancel(holder, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminals(2);
    host.recorder.consume_all();
    ak_channel_release(channel);
    host.stop();
}

/// No lend is of no bytes: an empty message needs no buffer, and the send takes none.
#[test]
fn an_empty_message_is_sent_with_no_buffer() {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel(&server.endpoint);
    let call = start_call(channel, ECHO, &[]);

    assert_eq!(lend(call, 0).0, ak_status::AK_STATUS_INVALID_ARG);
    assert_eq!(
        unsafe { ak_call_send_message(call, empty_buffer(), std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    assert_eq!(
        unsafe { ak_call_end_send(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0));
    assert_eq!(seen.message_payloads(), vec![Vec::<u8>::new()]);
    assert!(
        seen.kinds().contains(&ak_event_kind::AK_EVENT_WRITE_DONE),
        "the empty send is acquitted like any other"
    );

    ak_channel_release(channel);
    host.stop();
}
