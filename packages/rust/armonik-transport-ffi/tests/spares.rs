//! The arenas a channel's sends are done with, lent again rather than allocated, counted with the
//! library's test hooks.
//!
//! The count is the process's. Each test holds the one runtime from its first lend to its
//! quiescence, so no other test of this binary lends while it counts.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use armonik_transport_ffi::hooks;
use armonik_transport_ffi::*;
use support::host::*;
use support::{COLLECT, ECHO};

/// Past the size a channel keeps its arenas from.
const LARGE: usize = 128 * 1024;

/// Waits for an arena to be kept as a spare since `kept` were: the engine drops a message's last
/// bytes on its own thread, after the terminal may have reached the host.
fn await_spare_since(kept: usize) {
    support::poll_until(
        || hooks::spares_kept() > kept,
        || format!("no arena kept since the {kept} before"),
    );
}

/// One request of `len` bytes, answered and reclaimed.
fn request(host: &Host, channel: ak_handle, len: usize, answered: usize) {
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
    let (status, buffer) = lend(call, len);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { std::ptr::write_bytes(buffer.ptr, 0x5a, len) };
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, len, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    let seen = host.recorder.await_terminals(answered);
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
    support::await_call_reclaimed(call);
}

/// A large request's arena is the next one's: the second lends what the first gave back.
#[test]
fn a_large_arena_is_lent_again_on_its_channel() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let kept = hooks::spares_kept();
    request(host, channel, LARGE, 1);
    await_spare_since(kept);
    let before = hooks::new_arenas();
    request(host, channel, LARGE, 2);
    assert_eq!(
        hooks::new_arenas() - before,
        0,
        "the second request lent a new arena"
    );
    assert_eq!(
        memory_usage(host.runtime).bytes_used,
        0,
        "a spare is not the host's debt"
    );
    fixture.close();
}

/// One streamed message of `len` bytes, the call ended and answered.
fn stream(host: &Host, channel: ak_handle, len: usize, answered: usize) {
    let call = start_call(channel, COLLECT, &[]);
    let (status, buffer) = lend(call, len);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { std::ptr::write_bytes(buffer.ptr, 0x5a, len) };
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, len, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    assert_eq!(
        unsafe { ak_call_end_send(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    let seen = host.recorder.await_terminals(answered);
    assert_eq!(seen.status_code(), Some(0), "{}", seen.status_message());
}

/// A streamed message's arena comes back too, once the engine has taken the message in.
#[test]
fn a_streamed_messages_arena_is_lent_again() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let kept = hooks::spares_kept();
    stream(host, channel, LARGE, 1);
    await_spare_since(kept);
    let before = hooks::new_arenas();
    stream(host, channel, LARGE, 2);
    assert_eq!(hooks::new_arenas() - before, 0);
    fixture.close();
}

/// A lend that takes a spare larger than it asks for is charged what backs it, the slack too, and
/// gives it all back.
#[test]
fn a_spare_is_charged_what_backs_it() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let kept = hooks::spares_kept();
    request(host, channel, LARGE, 1);
    await_spare_since(kept);
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
    let (status, buffer) = lend(call, LARGE - 1024);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert!(
        memory_usage(host.runtime).bytes_used >= LARGE as u64,
        "the slack of the spare is charged with the request"
    );
    unsafe { ak_return_call_buffer(buffer) };
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    fixture.close();
}

/// A small one is not kept: the allocator already serves it from memory it holds.
#[test]
fn a_small_arena_is_not_kept() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    request(host, channel, 1024, 1);
    let before = hooks::new_arenas();
    request(host, channel, 1024, 2);
    assert_eq!(hooks::new_arenas() - before, 1);
    fixture.close();
}
