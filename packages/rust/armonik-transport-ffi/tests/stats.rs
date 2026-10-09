//! What `ak_runtime_stats` answers: the counters of a runtime's calls when the library counts, an
//! empty structure when it does not, and the refusals of a record that is not as it asks.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::mem::{offset_of, size_of};

use armonik_transport_ffi::*;
use support::host::*;
use support::{blob, ECHO};

/// Whether the library under test was built to count.
const COUNTING: bool = cfg!(feature = "metrics");

fn stats_into(runtime: ak_handle, stats: &mut ak_stats) -> ak_status {
    unsafe { ak_runtime_stats(runtime, stats, std::ptr::null_mut()) }
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
    let call = start_call(channel, ECHO, &blob(&[]));
    send_one(call, message);
    host.recorder.await_terminal();
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
