//! What the host says it wrote: only those bytes are sent, and an overrun, said or written,
//! shuts the runtime down.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport_ffi::*;
use support::host::*;
use support::{COLLECT, ECHO};

fn write(buffer: ak_buffer, message: &[u8]) {
    unsafe { std::ptr::copy_nonoverlapping(message.as_ptr(), buffer.ptr, message.len()) };
}

/// A buffer is lent at an upper bound: the message is the bytes the host says it wrote, and the
/// whole lend's charge is given back.
#[test]
fn only_the_bytes_the_host_wrote_are_sent() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);

    let (status, buffer) = lend(call, 64);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    write(buffer, b"hello");
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, 5, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(seen.message_payloads(), vec![b"hello".to_vec()]);
    support::await_call_reclaimed(call);
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    fixture.close();
}

/// The same on a stream, whose acquittal gives back the lend's charge rather than the message's.
#[test]
fn a_stream_sends_the_bytes_the_host_wrote() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    let (status, buffer) = lend(call, 64);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    write(buffer, b"one");
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, 3, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_write_done();
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    assert_eq!(
        unsafe { ak_call_end_send(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    assert_eq!(seen.message_payloads(), vec![b"1:one".to_vec()]);
    fixture.close();
}

/// A commit refused once the call is over leaves the buffer with the host, as it was, to give
/// back: nothing of it is charged once it is, and the runtime reaches quiescence.
#[test]
fn a_refused_commit_leaves_the_buffer_to_give_back() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);

    let (_, buffer) = lend(call, 8);
    write(buffer, b"one");
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, 3, std::ptr::null_mut()) },
        ak_status::AK_STATUS_INVALID_STATE
    );
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

/// Saying more than was lent is an overrun: nothing is sent, and the runtime shuts down.
#[test]
fn a_commit_longer_than_its_lend_shuts_the_runtime_down() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);

    let (_, buffer) = lend(call, 8);
    write(buffer, b"12345678");
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, 9, std::ptr::null_mut()) },
        ak_status::AK_STATUS_CORRUPTED
    );
    assert_ne!(
        ak_runtime_status(host.runtime),
        ak_runtime_state::AK_RUNTIME_RUNNING
    );

    let seen = host.recorder.await_terminal();
    assert!(seen.message_payloads().is_empty(), "{seen:?}");
    fixture.close();
}

/// Writing past the end is an overrun whatever the host then says it wrote.
#[test]
fn a_write_past_the_end_shuts_the_runtime_down() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);

    let (_, buffer) = lend(call, 8);
    // One byte past the lend, onto what this library put after it.
    write(buffer, b"123456789");
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, 8, std::ptr::null_mut()) },
        ak_status::AK_STATUS_CORRUPTED
    );
    assert_ne!(
        ak_runtime_status(host.runtime),
        ak_runtime_state::AK_RUNTIME_RUNNING
    );
    fixture.close();
}

/// A buffer given back overrun is one too: the return has no status to answer with, and the
/// shutdown is what says it.
#[test]
fn an_overrun_buffer_given_back_shuts_the_runtime_down() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);

    let (_, buffer) = lend(call, 8);
    write(buffer, b"123456789");
    unsafe { ak_return_call_buffer(buffer) };
    assert_ne!(
        ak_runtime_status(host.runtime),
        ak_runtime_state::AK_RUNTIME_RUNNING
    );
    fixture.close();
}
