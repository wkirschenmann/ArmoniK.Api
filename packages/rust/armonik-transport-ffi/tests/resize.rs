//! A lent buffer exchanged for another size: what the host wrote is kept, the ceiling sees the
//! difference alone, and a refusal leaves the buffer the host's.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport_ffi::*;
use support::host::*;
use support::{empty_buffer, COLLECT, ECHO};

fn write(buffer: ak_buffer, at: usize, bytes: &[u8]) {
    assert!(at + bytes.len() <= buffer.len);
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.ptr.add(at), bytes.len()) };
}

fn read(buffer: ak_buffer, len: usize) -> Vec<u8> {
    assert!(len <= buffer.len);
    unsafe { std::slice::from_raw_parts(buffer.ptr, len) }.to_vec()
}

fn commit(call: ak_handle, buffer: ak_buffer, written: usize) -> ak_status {
    unsafe { ak_call_send_message(call, buffer, written, std::ptr::null_mut()) }
}

/// A buffer grown keeps the bytes the host wrote, takes the rest as the host writes them, and
/// commits as any other: the message is the one written across both.
#[test]
fn a_grown_buffer_keeps_what_the_host_wrote() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);

    let (status, small) = lend(call, 8);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    write(small, 0, b"hello");

    let (status, large) = resize(small, 4096, 5);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(large.len, 4096);
    assert_eq!(read(large, 5), b"hello");
    assert_eq!(large.ptr as usize % 8, 0, "aligned as a lend is");
    assert_eq!(memory_usage(host.runtime).bytes_used, 4096);

    write(large, 5, b" and a good deal more than eight bytes");
    assert_eq!(commit(call, large, 5 + 38), ak_status::AK_STATUS_OK);

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(
        seen.message_payloads(),
        vec![b"hello and a good deal more than eight bytes".to_vec()]
    );
    support::await_call_reclaimed(call);
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    fixture.close();
}

/// A buffer shrunk keeps what was written and is charged what it is now, so a host that lent more
/// than it needed gives the rest back before it commits.
#[test]
fn a_shrunk_buffer_is_charged_what_it_is_now() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);

    let (_, large) = lend(call, 4096);
    write(large, 0, b"abc");
    assert_eq!(memory_usage(host.runtime).bytes_used, 4096);

    let (status, small) = resize(large, 16, 3);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(small.len, 16);
    assert_eq!(read(small, 3), b"abc");
    assert_eq!(memory_usage(host.runtime).bytes_used, 16);

    assert_eq!(commit(call, small, 3), ak_status::AK_STATUS_OK);
    let seen = host.recorder.await_terminal();
    assert_eq!(seen.message_payloads(), vec![b"abc".to_vec()]);
    fixture.close();
}

/// Nothing is kept when the host says so, and the same length is a buffer like any other: the
/// operation is an exchange, whatever the sizes.
#[test]
fn a_buffer_is_exchanged_whatever_the_sizes() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);

    let (_, buffer) = lend(call, 32);
    write(buffer, 0, b"discarded");
    let (status, same) = resize(buffer, 32, 0);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(same.len, 32);
    assert_eq!(memory_usage(host.runtime).bytes_used, 32);

    write(same, 0, b"kept");
    let (status, other) = resize(same, 1, 1);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(read(other, 1), b"k");
    assert_eq!(commit(call, other, 1), ak_status::AK_STATUS_OK);

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.message_payloads(), vec![b"k".to_vec()]);
    fixture.close();
}

/// A stream's message resized is acquitted as any other, and what it was charged comes back with
/// the acquittal: the new buffer's, not the old one's.
#[test]
fn a_resized_message_on_a_stream_is_sent_and_acquitted() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    let (_, buffer) = lend(call, 4);
    write(buffer, 0, b"one");
    let (status, buffer) = resize(buffer, 64, 3);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(commit(call, buffer, 3), ak_status::AK_STATUS_OK);
    host.recorder.await_write_done();
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);

    assert_eq!(
        unsafe { ak_call_end_send(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    let seen = host.recorder.await_terminal();
    assert_eq!(seen.message_payloads(), vec![b"1:one".to_vec()]);
    fixture.close();
}

/// The buffer the host holds is one for the call's count, whatever it has been exchanged for.
#[test]
fn the_call_owes_one_buffer_through_every_resize() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    let (_, mut buffer) = lend(call, 8);
    assert_eq!(debt_of(call).buffers_lent, 1);
    for len in [64, 4, 4096] {
        buffer = resize(buffer, len, 0).1;
        assert_eq!(buffer.len, len);
        assert_eq!(debt_of(call).buffers_lent, 1);
    }
    unsafe { ak_return_call_buffer(buffer) };
    assert_eq!(debt_of(call).buffers_lent, 0);
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);

    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminal();
    fixture.close();
}

/// The ceiling sees the difference: a buffer that grows within the room the host already holds is
/// admitted though the old and the new together would pass it, and one that grows past it is not.
#[test]
fn the_ceiling_sees_the_difference_alone() {
    let fixture = Connected::with_ceiling(64);
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    let (_, buffer) = lend(call, 40);
    write(buffer, 0, b"kept");
    let (status, buffer) = resize(buffer, 64, 4);
    assert_eq!(status, ak_status::AK_STATUS_OK, "40 and 64 are never held");
    assert_eq!(read(buffer, 4), b"kept");
    assert_eq!(memory_usage(host.runtime).bytes_used, 64);

    let (status, buffer) = resize(buffer, 65, 4);
    assert_eq!(status, ak_status::AK_STATUS_MESSAGE_TOO_LARGE);
    assert_eq!(buffer.len, 64, "the buffer is the one it was");
    assert_eq!(memory_usage(host.runtime).bytes_used, 64);

    let (status, buffer) = resize(buffer, 8, 4);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(memory_usage(host.runtime).bytes_used, 8);

    unsafe { ak_return_call_buffer(buffer) };
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminal();
    fixture.close();
}

/// A resize the ceiling has no room for leaves the buffer lent and written as it was, records no
/// wait, and can be asked again once the room is back.
#[test]
fn a_resize_refused_for_room_leaves_the_buffer_and_owes_no_wake_up() {
    for flags in [0, AK_CALL_ONE_REQUEST] {
        refused_for_room(flags);
    }
}

fn refused_for_room(flags: u32) {
    let fixture = Connected::with_ceiling(64);
    let (host, channel) = (&fixture.host, fixture.channel);
    let other = start_call(channel, COLLECT, &[]);
    let (_, held) = lend(other, 24);

    let call = start_call_flagged(channel, ECHO, &[], flags);
    let (_, buffer) = lend(call, 24);
    write(buffer, 0, b"written");

    let (status, same) = resize(buffer, 50, 7);
    assert_eq!(status, ak_status::AK_STATUS_BUDGET_BUSY);
    assert_eq!(same.ptr, buffer.ptr);
    assert_eq!(same.len, 24);
    assert_eq!(read(same, 7), b"written", "the bytes are where they were");
    assert_eq!(memory_usage(host.runtime).bytes_used, 48);
    assert_eq!(debt_of(call).buffers_lent, 1);
    assert!(
        !host
            .recorder
            .kinds()
            .contains(&ak_event_kind::AK_EVENT_BUDGET_WAKE),
        "nothing was given back yet"
    );

    // Nothing waits for the room: it is given back, and the host asks again.
    unsafe { ak_return_call_buffer(held) };
    let (status, buffer) = resize(same, 50, 7);
    assert_eq!(status, ak_status::AK_STATUS_OK, "the room came back");
    assert_eq!(read(buffer, 7), b"written");
    assert_eq!(memory_usage(host.runtime).bytes_used, 50);

    unsafe { ak_return_call_buffer(buffer) };
    for call in [call, other] {
        assert_eq!(
            unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
    }
    host.recorder.await_terminals(2);
    assert!(
        !host
            .recorder
            .kinds()
            .contains(&ak_event_kind::AK_EVENT_BUDGET_WAKE),
        "a refused resize owes no wake-up"
    );
    fixture.close();
}

/// A host that waits for room gives its buffer back and lends: that wait is the lend's, and
/// the room it needs wakes it.
#[test]
fn a_host_that_waits_for_the_room_a_resize_needs_lends_and_is_woken() {
    let fixture = Connected::with_ceiling(64);
    let (host, channel) = (&fixture.host, fixture.channel);
    let other = start_call(channel, COLLECT, &[]);
    let (_, held) = lend(other, 24);
    let call = start_call(channel, COLLECT, &[]);
    let (_, buffer) = lend(call, 24);

    let (status, buffer) = resize(buffer, 50, 0);
    assert_eq!(status, ak_status::AK_STATUS_BUDGET_BUSY);
    unsafe { ak_return_call_buffer(buffer) };
    assert_eq!(lend(call, 50).0, ak_status::AK_STATUS_BUDGET_BUSY);

    unsafe { ak_return_call_buffer(held) };
    host.recorder.await_budget_wake();
    let (status, buffer) = lend(call, 50);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { ak_return_call_buffer(buffer) };

    for call in [call, other] {
        assert_eq!(
            unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
    }
    host.recorder.await_terminals(2);
    fixture.close();
}

/// A refused resize leaves a buffer that still commits: nothing of it was given up.
#[test]
fn a_buffer_whose_resize_was_refused_still_commits() {
    let fixture = Connected::with_ceiling(64);
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);

    let (_, buffer) = lend(call, 8);
    write(buffer, 0, b"hello");
    let (status, buffer) = resize(buffer, 65, 5);
    assert_eq!(status, ak_status::AK_STATUS_MESSAGE_TOO_LARGE);
    assert_eq!(commit(call, buffer, 5), ak_status::AK_STATUS_OK);

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.message_payloads(), vec![b"hello".to_vec()]);
    fixture.close();
}

/// A call that is over, or whose cancellation is requested, resizes nothing, and the buffer is
/// given back as a refused commit leaves it to be.
#[test]
fn a_cancelled_call_resizes_nothing() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    let (_, buffer) = lend(call, 8);
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    let (status, buffer) = resize(buffer, 16, 0);
    assert_eq!(status, ak_status::AK_STATUS_INVALID_STATE);
    assert_eq!(buffer.len, 8);
    unsafe { ak_return_call_buffer(buffer) };

    host.recorder.await_terminal();
    support::await_call_reclaimed(call);
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    assert_eq!(
        ak_runtime_status(host.runtime),
        ak_runtime_state::AK_RUNTIME_RUNNING
    );
    fixture.close();
}

/// A length of no bytes, bytes to keep that the new length cannot hold, no place to write the
/// answer and a buffer that was never lent are the host's mistakes, and none takes the buffer.
#[test]
fn a_resize_that_asks_the_impossible_is_refused() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);
    let (_, buffer) = lend(call, 16);
    write(buffer, 0, b"0123456789");

    assert_eq!(
        resize(buffer, 0, 0).0,
        ak_status::AK_STATUS_INVALID_ARG,
        "an empty message needs no buffer"
    );
    assert_eq!(
        resize(buffer, 4, 5).0,
        ak_status::AK_STATUS_INVALID_ARG,
        "five bytes do not keep in four"
    );
    assert_eq!(
        unsafe {
            ak_resize_call_buffer(buffer, 32, 10, std::ptr::null_mut(), std::ptr::null_mut())
        },
        ak_status::AK_STATUS_INVALID_ARG
    );
    assert_eq!(
        resize(empty_buffer(), 32, 0).0,
        ak_status::AK_STATUS_INVALID_ARG,
        "no owner, so not lent"
    );

    let (status, buffer) = resize(buffer, 32, 10);
    assert_eq!(status, ak_status::AK_STATUS_OK, "the buffer was left alone");
    assert_eq!(read(buffer, 10), b"0123456789");
    unsafe { ak_return_call_buffer(buffer) };

    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminal();
    fixture.close();
}

/// Keeping more than was lent is an overrun, as committing more is: the buffer is taken back,
/// nothing is carried over, and the runtime shuts down.
#[test]
fn keeping_more_than_was_lent_shuts_the_runtime_down() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    let (_, buffer) = lend(call, 8);
    let mut out = empty_buffer();
    assert_eq!(
        unsafe { ak_resize_call_buffer(buffer, 64, 9, &mut out, std::ptr::null_mut()) },
        ak_status::AK_STATUS_CORRUPTED
    );
    assert!(out.owner.is_null(), "no buffer came of it");
    assert_ne!(
        ak_runtime_status(host.runtime),
        ak_runtime_state::AK_RUNTIME_RUNNING
    );
    fixture.close();
}

/// Writing past the end is an overrun whatever the host then says it kept: the sentinel is
/// checked before anything is carried over.
#[test]
fn a_write_past_the_end_shuts_the_runtime_down_on_a_resize_too() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    let (_, buffer) = lend(call, 8);
    // One byte past the lend, onto what this library put after it.
    unsafe { std::ptr::write_bytes(buffer.ptr, 0x41, 9) };
    let mut out = empty_buffer();
    assert_eq!(
        unsafe { ak_resize_call_buffer(buffer, 64, 8, &mut out, std::ptr::null_mut()) },
        ak_status::AK_STATUS_CORRUPTED
    );
    assert!(out.owner.is_null());
    assert_ne!(
        ak_runtime_status(host.runtime),
        ak_runtime_state::AK_RUNTIME_RUNNING
    );
    fixture.close();
}
