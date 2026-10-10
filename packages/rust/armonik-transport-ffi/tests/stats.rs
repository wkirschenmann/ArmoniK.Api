//! What `ak_runtime_stats` and `ak_channel_stats` answer: the counters of a runtime's calls, and of
//! the calls to one endpoint, when the library counts, an empty structure when it does not, and the
//! refusals of a record that is not as it asks.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::mem::{offset_of, size_of};

use armonik_transport_ffi::*;
use support::host::*;
use support::{blob, TestServer, ECHO};

/// Whether the library under test was built to count.
const COUNTING: bool = cfg!(feature = "metrics");

fn stats_into(runtime: ak_handle, stats: &mut ak_stats) -> ak_status {
    unsafe { ak_runtime_stats(runtime, stats, std::ptr::null_mut()) }
}

fn channel_stats(channel: ak_handle) -> ak_stats {
    let mut stats = asking(size_of::<ak_stats>());
    let status = unsafe { ak_channel_stats(channel, &mut stats, std::ptr::null_mut()) };
    assert_eq!(status, ak_status::AK_STATUS_OK);
    stats
}

fn endpoint_of(channel: ak_handle) -> String {
    let mut length = 0usize;
    let status = unsafe {
        ak_channel_endpoint(
            channel,
            std::ptr::null_mut(),
            0,
            &mut length,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, ak_status::AK_STATUS_OK);
    let mut buffer = vec![0u8; length];
    unsafe {
        ak_channel_endpoint(
            channel,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut length,
            std::ptr::null_mut(),
        )
    };
    String::from_utf8(buffer).expect("an endpoint is UTF-8")
}

fn asking(size: usize) -> ak_stats {
    // SAFETY: integers and floats, for which all-zero bytes are zero.
    let mut stats: ak_stats = unsafe { std::mem::zeroed() };
    stats.struct_size = size as u32;
    stats
}

fn read(runtime: ak_handle) -> ak_stats {
    let mut stats = asking(size_of::<ak_stats>());
    assert_eq!(stats_into(runtime, &mut stats), ak_status::AK_STATUS_OK);
    stats
}

/// A call that sends one message and reads its echo, to its end.
fn echo_once(host: &Host, channel: ak_handle, message: &[u8]) {
    echo_nth(host, channel, message, 1);
}

/// The `nth` call of the host's run to send one message and read its echo, to its end: the
/// recorder keeps every event, so it waits for the terminals of all the calls so far.
fn echo_nth(host: &Host, channel: ak_handle, message: &[u8], nth: usize) {
    let call = start_call(channel, ECHO, &blob(&[]));
    send_one(call, message);
    host.recorder.await_terminals(nth);
    host.recorder.consume_all();
    support::await_call_reclaimed(call);
}

#[test]
fn a_library_that_counts_says_so_and_one_that_does_not_answers_empty() {
    let host = Host::start();
    let stats = read(host.runtime);

    assert_eq!(stats.flags & AK_STATS_COUNTING != 0, COUNTING);
    assert_eq!(stats.struct_size as usize, size_of::<ak_stats>());
    assert_eq!(stats.calls_started, 0);
    assert_eq!(stats.calls_ended, [0; 17]);
}

#[test]
fn the_calls_of_a_runtime_are_counted_across_the_abi() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    echo_once(host, channel, b"hello");

    let stats = read(host.runtime);
    if COUNTING {
        assert_eq!(stats.calls_started, 1);
        assert_eq!(stats.calls_ended[0], 1, "ended OK");
        assert_eq!((stats.messages_sent, stats.messages_received), (1, 1));
        assert_eq!((stats.message_bytes_raw, stats.message_bytes_sent), (5, 5));
        assert_eq!(stats.dials_succeeded, 1);
        assert!(stats.wire_bytes_sent > 0 && stats.wire_bytes_received > 0);
        assert_eq!(stats.calls_waiting_for_stream, 0);
    } else {
        assert_eq!(stats.calls_started, 0);
        assert_eq!(stats.calls_ended, [0; 17]);
        assert_eq!(stats.messages_sent, 0);
        assert_eq!(stats.wire_bytes_sent, 0);
    }
    fixture.close();
}

/// The memory ceiling's refusals and waits, counted by the runtime and not by a channel.
#[test]
fn a_lend_refused_at_the_ceiling_is_counted() {
    let fixture = Connected::with_ceiling(1024);
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, ECHO, &blob(&[]));

    let (first, held) = lend(call, 1000);
    assert_eq!(first, ak_status::AK_STATUS_OK);
    // A second call asks for more than the ceiling has left.
    let other = start_call(channel, ECHO, &blob(&[]));
    let (second, _) = lend(other, 1000);
    assert_eq!(second, ak_status::AK_STATUS_BUDGET_BUSY);

    let stats = read(host.runtime);
    assert_eq!(stats.host_memory_refusals, u64::from(COUNTING));
    assert_eq!(stats.host_memory_waits, u64::from(COUNTING));

    unsafe { ak_return_call_buffer(held) };
    for call in [call, other] {
        let _ = unsafe { ak_call_cancel(call, std::ptr::null_mut()) };
    }
    host.recorder.await_terminals(2);
    host.recorder.consume_all();
    fixture.close();
}

/// A resize the ceiling has no room for is counted as a lend refused there is, and records no
/// wait: the host holds a buffer, and a wait is a lend's.
#[test]
fn a_resize_refused_at_the_ceiling_is_counted() {
    let fixture = Connected::with_ceiling(1024);
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, ECHO, &blob(&[]));
    let other = start_call(channel, ECHO, &blob(&[]));
    let (status, small) = lend(call, 100);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    let (status, large) = lend(other, 900);
    assert_eq!(status, ak_status::AK_STATUS_OK);

    let (status, small) = resize(small, 200, 0);
    assert_eq!(status, ak_status::AK_STATUS_BUDGET_BUSY);

    let stats = read(host.runtime);
    assert_eq!(stats.host_memory_refusals, u64::from(COUNTING));
    assert_eq!(stats.host_memory_waits, 0);

    for buffer in [small, large] {
        unsafe { ak_return_call_buffer(buffer) };
    }
    for call in [call, other] {
        let _ = unsafe { ak_call_cancel(call, std::ptr::null_mut()) };
    }
    host.recorder.await_terminals(2);
    host.recorder.consume_all();
    fixture.close();
}

#[test]
fn a_record_of_the_head_alone_learns_whether_the_library_counts() {
    let host = Host::start();
    let mut head = asking(offset_of!(ak_stats, calls_started));
    head.calls_started = 99;
    head.calls_waiting_for_stream = 99;

    assert_eq!(stats_into(host.runtime, &mut head), ak_status::AK_STATUS_OK);
    assert_eq!(
        head.struct_size as usize,
        offset_of!(ak_stats, calls_started)
    );
    assert_eq!(head.flags & AK_STATS_COUNTING != 0, COUNTING);
    assert_eq!(
        head.calls_started, 99,
        "a field past the record is left alone"
    );
    assert_eq!(head.calls_waiting_for_stream, 99);
}

/// Whole fields that lie within the host's size, and nothing past it.
#[test]
fn a_shorter_record_is_filled_as_far_as_it_goes() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    echo_once(host, channel, b"hello");

    // A host built before the fields past the first array: sixteen, one counter and the array.
    let size = offset_of!(ak_stats, messages_sent) + 3;
    let mut stats = asking(size);
    stats.messages_sent = 99;
    assert_eq!(
        stats_into(host.runtime, &mut stats),
        ak_status::AK_STATUS_OK
    );

    assert_eq!(
        stats.struct_size as usize,
        offset_of!(ak_stats, messages_sent)
    );
    assert_eq!(stats.calls_started, u64::from(COUNTING));
    assert_eq!(
        stats.messages_sent, 99,
        "a field cut by the size is not written"
    );
    fixture.close();
}

#[test]
fn a_record_larger_than_the_librarys_is_filled_as_far_as_the_library_goes() {
    let host = Host::start();
    let mut bigger = vec![0xAAu8; size_of::<ak_stats>() + 16];
    let len = bigger.len() as u32;
    bigger[..4].copy_from_slice(&len.to_ne_bytes());
    bigger[4..16].fill(0);

    let status = unsafe {
        ak_runtime_stats(
            host.runtime,
            bigger.as_mut_ptr().cast(),
            std::ptr::null_mut(),
        )
    };

    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(
        u32::from_ne_bytes(bigger[..4].try_into().expect("four bytes")) as usize,
        size_of::<ak_stats>()
    );
    assert!(bigger[size_of::<ak_stats>()..]
        .iter()
        .all(|byte| *byte == 0xAA));
}

#[test]
fn a_record_the_library_cannot_read_is_refused() {
    let host = Host::start();

    let mut too_short = asking(8);
    assert_eq!(
        stats_into(host.runtime, &mut too_short),
        ak_status::AK_STATUS_INVALID_ARG
    );
    for set in [
        |stats: &mut ak_stats| stats.version = 1,
        |stats: &mut ak_stats| stats.flags = 1,
        |stats: &mut ak_stats| stats.reserved = 1,
    ] {
        let mut stats = asking(size_of::<ak_stats>());
        set(&mut stats);
        assert_eq!(
            stats_into(host.runtime, &mut stats),
            ak_status::AK_STATUS_INVALID_ARG
        );
    }
    assert_eq!(
        unsafe { ak_runtime_stats(host.runtime, std::ptr::null_mut(), std::ptr::null_mut()) },
        ak_status::AK_STATUS_INVALID_ARG
    );
    let mut stats = asking(size_of::<ak_stats>());
    assert_eq!(
        stats_into(AK_HANDLE_NONE, &mut stats),
        ak_status::AK_STATUS_HANDLE_STALE
    );
}

/// Two channels to two endpoints are two series, and the runtime reads their sum.
#[test]
fn the_channels_of_two_endpoints_are_read_apart_and_the_runtime_reads_both() {
    let (first, second) = (TestServer::start(), TestServer::start());
    let host = Host::start();
    let (a, b) = (
        host.channel(&first.endpoint),
        host.channel(&second.endpoint),
    );
    echo_nth(&host, a, b"one", 1);
    echo_nth(&host, a, b"two", 2);
    echo_nth(&host, b, b"three", 3);

    let (of_a, of_b, all) = (channel_stats(a), channel_stats(b), read(host.runtime));
    if COUNTING {
        assert_eq!((of_a.calls_started, of_b.calls_started), (2, 1));
        assert_eq!((of_a.messages_sent, of_b.messages_sent), (2, 1));
        assert_eq!((of_a.message_bytes_raw, of_b.message_bytes_raw), (6, 5));
        assert_eq!((of_a.dials_succeeded, of_b.dials_succeeded), (1, 1));
        assert_eq!(all.calls_started, 3);
        assert_eq!(all.message_bytes_raw, 11);
        assert_eq!(all.dials_succeeded, 2);
    } else {
        for stats in [&of_a, &of_b, &all] {
            assert_eq!(stats.flags & AK_STATS_COUNTING, 0);
            assert_eq!(stats.calls_started, 0);
        }
    }
    assert_eq!(
        endpoint_of(a),
        first
            .endpoint
            .trim_end_matches('/')
            .trim_start_matches("http://")
    );
    assert_ne!(endpoint_of(a), endpoint_of(b));
    ak_channel_release(a);
    ak_channel_release(b);
    host.stop();
}

/// What a closed channel counted stays with its endpoint, which a channel opened after it reads.
#[test]
fn the_counts_of_a_closed_channel_stay_with_its_endpoint() {
    let server = TestServer::start();
    let host = Host::start();
    let first = host.channel(&server.endpoint);
    let beside = host.channel(&server.endpoint);
    echo_nth(&host, first, b"one", 1);
    assert_eq!(
        channel_stats(beside).calls_started,
        u64::from(COUNTING),
        "two channels to one endpoint read one registry"
    );

    ak_channel_release(first);
    support::poll_until(
        || ak_channel_status(first) == ak_channel_state::AK_CHANNEL_NONE,
        || "the channel was not reclaimed".to_owned(),
    );
    let mut stats = asking(size_of::<ak_stats>());
    assert_eq!(
        unsafe { ak_channel_stats(first, &mut stats, std::ptr::null_mut()) },
        ak_status::AK_STATUS_HANDLE_STALE
    );

    let later = host.channel(&server.endpoint);
    echo_nth(&host, later, b"two", 2);
    assert_eq!(channel_stats(later).calls_started, 2 * u64::from(COUNTING));
    assert_eq!(read(host.runtime).calls_started, 2 * u64::from(COUNTING));
    ak_channel_release(beside);
    ak_channel_release(later);
    host.stop();
}

#[test]
fn an_endpoint_is_read_with_a_buffer_that_may_be_short() {
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel(&server.endpoint);
    let whole = endpoint_of(channel);

    let mut buffer = [0xAAu8; 4];
    let mut length = 0usize;
    let status = unsafe {
        ak_channel_endpoint(
            channel,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut length,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(length, whole.len(), "the whole length, whatever fits");
    assert_eq!(&buffer[..], &whole.as_bytes()[..4]);

    assert_eq!(
        unsafe {
            ak_channel_endpoint(
                channel,
                std::ptr::null_mut(),
                4,
                &mut length,
                std::ptr::null_mut(),
            )
        },
        ak_status::AK_STATUS_INVALID_ARG
    );
    assert_eq!(
        unsafe {
            ak_channel_endpoint(
                channel,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        ak_status::AK_STATUS_INVALID_ARG
    );
    assert_eq!(
        unsafe {
            ak_channel_endpoint(
                AK_HANDLE_NONE,
                std::ptr::null_mut(),
                0,
                &mut length,
                std::ptr::null_mut(),
            )
        },
        ak_status::AK_STATUS_HANDLE_STALE
    );
    ak_channel_release(channel);
    host.stop();
}
