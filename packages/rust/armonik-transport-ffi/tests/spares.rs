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

/// The arena a resize leaves is kept like one a send is done with, and the one it takes is a
/// spare if one fits: a message that outgrows its buffer costs the channel one arena, not two.
#[test]
fn a_resize_keeps_the_arena_it_leaves_and_takes_a_spare() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
    let (status, small) = lend(call, LARGE);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { std::ptr::copy_nonoverlapping(b"kept".as_ptr(), small.ptr, 4) };

    let kept = hooks::spares_kept();
    let (status, large) = resize(small, 2 * LARGE, 4);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(hooks::spares_kept() - kept, 1, "the arena it left is kept");
    assert_eq!(
        memory_usage(host.runtime).bytes_used,
        2 * LARGE as u64,
        "and is not the host's debt"
    );

    // The arena kept is the next lend's.
    let other = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
    let before = hooks::new_arenas();
    let (status, again) = lend(other, LARGE);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(hooks::new_arenas() - before, 0, "a spare was lent");
    unsafe { ak_return_call_buffer(again) };

    assert_eq!(
        unsafe { ak_call_send_message(call, large, 4, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    let seen = host.recorder.await_terminal();
    assert_eq!(seen.message_payloads(), vec![b"kept".to_vec()]);
    assert_eq!(
        unsafe { ak_call_cancel(other, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminals(2);
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    fixture.close();
}

/// A resize that takes a spare larger than it asks for is charged what backs it, the slack too, and
/// gives it all back.
#[test]
fn a_resize_onto_a_spare_is_charged_what_backs_it() {
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);

    let kept = hooks::spares_kept();
    request(host, channel, LARGE, 1);
    await_spare_since(kept);

    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
    let (status, small) = lend(call, 16);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    let before = hooks::new_arenas();
    let (status, large) = resize(small, LARGE - 1024, 0);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(hooks::new_arenas() - before, 0, "the spare was taken");
    assert!(
        memory_usage(host.runtime).bytes_used >= LARGE as u64,
        "the slack of the spare is charged with the request"
    );

    let (status, small) = resize(large, 16, 0);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(memory_usage(host.runtime).bytes_used, 16);
    unsafe { ak_return_call_buffer(small) };
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    fixture.close();
}

/// A spare whose slack the ceiling has no room for, once another call has charged its own, is not
/// the buffer: the resize takes an arena of its own, charged the request alone, and the spare is
/// not kept beside the charges that have no room for it.
#[test]
fn a_resize_whose_spare_has_no_room_for_its_slack_takes_an_arena_of_its_own() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    let ceiling = 3 * LARGE;
    let fixture = Connected::with_ceiling(ceiling as u64);
    let (host, channel) = (&fixture.host, fixture.channel);

    let kept = hooks::spares_kept();
    request(host, channel, LARGE, 1);
    await_spare_since(kept);

    let other = start_call(channel, COLLECT, &[]);
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
    let (status, small) = lend(call, 16);
    assert_eq!(status, ak_status::AK_STATUS_OK);

    // Between the spare taken and the charge, another call charges what leaves the request room
    // and the slack none.
    let held = ceiling - LARGE + 512;
    let taken = Arc::new(Mutex::new(None));
    let once = Arc::new(AtomicBool::new(false));
    let hook_taken = Arc::clone(&taken);
    hooks::before_charge(Some(Arc::new(move || {
        if !once.swap(true, Ordering::SeqCst) {
            let (status, buffer) = lend(other, held);
            assert_eq!(status, ak_status::AK_STATUS_OK);
            *hook_taken.lock().unwrap() =
                Some((buffer.ptr as usize, buffer.len, buffer.owner as usize));
        }
    })));
    let (status, large) = resize(small, LARGE - 1024, 0);
    hooks::before_charge(None);

    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(
        memory_usage(host.runtime).bytes_used,
        (held + LARGE - 1024) as u64,
        "the request alone is charged beside the other call's"
    );

    unsafe { ak_return_call_buffer(large) };
    let (ptr, len, owner) = taken.lock().unwrap().take().expect("the hook ran");
    unsafe {
        ak_return_call_buffer(ak_buffer {
            ptr: ptr as *mut u8,
            len,
            owner: owner as *mut std::ffi::c_void,
        })
    };
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    for call in [call, other] {
        assert_eq!(
            unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
    }
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
