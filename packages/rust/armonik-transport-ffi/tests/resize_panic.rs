//! A panic inside a resize leaves the buffer lent, charged and the host's, as every refusal does:
//! the answer is INTERNAL, which a host may retry or give the buffer back after.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use armonik_transport_ffi::hooks::{self, ChargeStep, ResizeStep};
use armonik_transport_ffi::*;
use support::host::*;
use support::COLLECT;

/// The hooks are process-wide, so the tests of this binary take turns.
static TURN: Mutex<()> = Mutex::new(());

/// Holds the turn for the whole test: a resize of another test would meet this one's hook.
fn take_turn() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Takes the hook away however the test ends.
struct Panicking;

impl Panicking {
    /// Makes every resize panic on reaching `step`.
    fn at(step: ResizeStep) -> Self {
        hooks::at_each_resize_step(Some(Arc::new(move |reached| {
            if reached == step {
                panic!("injected at {reached:?}");
            }
        })));
        Self
    }

    /// Makes every charge panic on reaching `step`.
    fn in_charge_at(step: ChargeStep) -> Self {
        hooks::at_each_charge_step(Some(Arc::new(move |reached| {
            if reached == step {
                panic!("injected at {reached:?}");
            }
        })));
        Self
    }
}

impl Drop for Panicking {
    fn drop(&mut self) {
        hooks::at_each_resize_step(None);
        hooks::at_each_charge_step(None);
    }
}

fn write(buffer: ak_buffer, bytes: &[u8]) {
    assert!(bytes.len() <= buffer.len);
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.ptr, bytes.len()) };
}

fn read(buffer: ak_buffer, len: usize) -> Vec<u8> {
    assert!(len <= buffer.len);
    unsafe { std::slice::from_raw_parts(buffer.ptr, len) }.to_vec()
}

/// A resize that panics before the exchange is made is a refusal: the old buffer is as the host
/// left it, lent and charged, and a retry succeeds.
#[test]
fn a_panic_before_the_exchange_leaves_the_old_buffer_lent_and_charged() {
    let _turn = take_turn();
    for step in [
        ResizeStep::Taken,
        ResizeStep::Admitted,
        ResizeStep::Allocated,
        ResizeStep::Copied,
    ] {
        let fixture = Host::connected();
        let (host, channel) = (&fixture.host, fixture.channel);
        let call = start_call(channel, COLLECT, &[]);
        let (_, old) = lend(call, 16);
        write(old, b"hello");

        let panicking = Panicking::at(step);
        let (status, answered) = resize(old, 4096, 5);
        drop(panicking);

        assert_eq!(status, ak_status::AK_STATUS_INTERNAL, "{step:?}");
        assert_eq!(answered.owner, old.owner, "{step:?}: the host's buffer");
        assert_eq!(debt_of(call).buffers_lent, 1, "{step:?}");
        assert_eq!(memory_usage(host.runtime).bytes_used, 16, "{step:?}");
        assert_eq!(read(old, 5), b"hello", "{step:?}: what was written");

        // The retry a host makes on INTERNAL: the buffer is still the one it holds.
        let (status, grown) = resize(old, 4096, 5);
        assert_eq!(status, ak_status::AK_STATUS_OK, "{step:?}");
        assert_eq!(read(grown, 5), b"hello", "{step:?}");
        assert_eq!(debt_of(call).buffers_lent, 1, "{step:?}");
        assert_eq!(memory_usage(host.runtime).bytes_used, 4096, "{step:?}");

        unsafe { ak_return_call_buffer(grown) };
        assert_eq!(debt_of(call).buffers_lent, 0, "{step:?}");
        assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{step:?}");
        assert_eq!(
            unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
        host.recorder.await_terminal();
        fixture.close();
    }
}

/// The other half of what a host does after INTERNAL: it gives the buffer back, which the library
/// still owns and charges, and the call and the ceiling are as before the lend.
#[test]
fn a_buffer_is_returnable_after_a_panic_in_its_resize() {
    let _turn = take_turn();
    for step in [
        ResizeStep::Taken,
        ResizeStep::Admitted,
        ResizeStep::Allocated,
        ResizeStep::Copied,
    ] {
        let fixture = Host::connected();
        let (host, channel) = (&fixture.host, fixture.channel);
        let call = start_call(channel, COLLECT, &[]);
        let (_, old) = lend(call, 16);

        let panicking = Panicking::at(step);
        let (status, _) = resize(old, 4096, 0);
        drop(panicking);
        assert_eq!(status, ak_status::AK_STATUS_INTERNAL, "{step:?}");

        unsafe { ak_return_call_buffer(old) };
        assert_eq!(debt_of(call).buffers_lent, 0, "{step:?}");
        assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{step:?}");

        // The window slot came back with it: another lend is admitted.
        let (status, again) = lend(call, 8);
        assert_eq!(status, ak_status::AK_STATUS_OK, "{step:?}");
        unsafe { ak_return_call_buffer(again) };
        assert_eq!(
            unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
        host.recorder.await_terminal();
        fixture.close();
    }
}

/// A buffer whose resize panicked is committed as it was: the message is the bytes the host wrote.
#[test]
fn a_buffer_is_committable_after_a_panic_in_its_resize() {
    let _turn = take_turn();
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);
    let (_, old) = lend(call, 16);
    write(old, b"hello");

    let panicking = Panicking::at(ResizeStep::Copied);
    let (status, _) = resize(old, 4096, 5);
    drop(panicking);
    assert_eq!(status, ak_status::AK_STATUS_INTERNAL);

    assert_eq!(
        unsafe { ak_call_send_message(call, old, 5, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_write_done();
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    assert_eq!(
        unsafe { ak_call_end_send(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    let seen = host.recorder.await_terminal();
    assert_eq!(seen.message_payloads(), vec![b"1:hello".to_vec()]);
    fixture.close();
}

/// Once the charge has moved the exchange is made, and a panic in parking the old arena does not
/// unmake it: the host is told it succeeded, and holds the new buffer.
#[test]
fn a_panic_after_the_exchange_is_made_does_not_refuse_it() {
    let _turn = take_turn();
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    let call = start_call(channel, COLLECT, &[]);
    let (_, old) = lend(call, 16);
    write(old, b"hello");

    let panicking = Panicking::at(ResizeStep::Exchanged);
    let (status, grown) = resize(old, 4096, 5);
    drop(panicking);

    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(grown.len, 4096);
    assert_eq!(read(grown, 5), b"hello");
    assert_eq!(debt_of(call).buffers_lent, 1);
    assert_eq!(memory_usage(host.runtime).bytes_used, 4096);

    unsafe { ak_return_call_buffer(grown) };
    assert_eq!(debt_of(call).buffers_lent, 0);
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminal();
    fixture.close();
}

/// Past the size a channel keeps its arenas from.
const LARGE: usize = 128 * 1024;

/// One request of `len` bytes, answered and reclaimed: its arena is kept as a spare.
fn request(host: &Host, channel: ak_handle, len: usize) {
    let kept = hooks::spares_kept();
    let call = start_call_flagged(channel, support::ECHO, &[], AK_CALL_ONE_REQUEST);
    let (status, buffer) = lend(call, len);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { std::ptr::write_bytes(buffer.ptr, 0x5a, len) };
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, len, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminal();
    support::await_call_reclaimed(call);
    support::poll_until(
        || hooks::spares_kept() > kept,
        || "no arena was kept".to_string(),
    );
}

/// A resize that took a spare of the channel's and panics before the charge refuses as any other:
/// the spare is lost with the panic, and nothing of it is charged or owed.
#[test]
fn a_panic_after_taking_a_spare_leaves_the_old_buffer_lent_and_charged() {
    let _turn = take_turn();
    let fixture = Host::connected();
    let (host, channel) = (&fixture.host, fixture.channel);
    request(host, channel, LARGE);

    let call = start_call(channel, COLLECT, &[]);
    let (_, old) = lend(call, 16);
    write(old, b"hello");
    let new_len = LARGE - 1024;

    let before = hooks::new_arenas();
    let panicking = Panicking::at(ResizeStep::Copied);
    let (status, _) = resize(old, new_len, 5);
    drop(panicking);

    assert_eq!(status, ak_status::AK_STATUS_INTERNAL);
    assert_eq!(hooks::new_arenas() - before, 0, "the spare was the arena");
    assert_eq!(debt_of(call).buffers_lent, 1);
    assert_eq!(memory_usage(host.runtime).bytes_used, 16);
    assert_eq!(read(old, 5), b"hello");

    unsafe { ak_return_call_buffer(old) };
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    fixture.close();
}

/// A spare whose slack the ceiling has no room for gives way to an arena of its own, and a panic
/// at that arena refuses as the others do: the charge was not made, so the old one stands.
#[test]
fn a_panic_in_the_arena_taken_for_want_of_room_leaves_the_old_buffer_lent_and_charged() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let _turn = take_turn();
    let ceiling = 3 * LARGE;
    let fixture = Connected::with_ceiling(ceiling as u64);
    let (host, channel) = (&fixture.host, fixture.channel);
    request(host, channel, LARGE);

    let other = start_call(channel, COLLECT, &[]);
    let call = start_call(channel, COLLECT, &[]);
    let (_, old) = lend(call, 16);
    write(old, b"hello");

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
    let copies = Arc::new(AtomicUsize::new(0));
    hooks::at_each_resize_step(Some(Arc::new(move |reached| {
        if reached == ResizeStep::Copied && copies.fetch_add(1, Ordering::SeqCst) == 1 {
            panic!("injected at the second copy");
        }
    })));
    let (status, _) = resize(old, LARGE - 1024, 5);
    hooks::before_charge(None);
    hooks::at_each_resize_step(None);

    assert_eq!(status, ak_status::AK_STATUS_INTERNAL);
    assert_eq!(
        memory_usage(host.runtime).bytes_used,
        (held + 16) as u64,
        "the old charge stands beside the other call's"
    );
    assert_eq!(read(old, 5), b"hello");

    unsafe { ak_return_call_buffer(old) };
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

/// The ceiling's charge moves in one step: a panic before it leaves the old charge, a refusal like
/// the others, and the old buffer is lent and charged as the host left it. The same resize goes
/// through when the host asks again, and the runtime shuts down with nothing owed.
#[test]
fn a_panic_before_the_charge_moves_leaves_the_old_buffer_lent_and_charged() {
    let _turn = take_turn();
    for (from, to) in [(16, 4096), (4096, 16)] {
        let context = format!("{from} to {to}");
        let fixture = Host::connected();
        let (host, channel) = (&fixture.host, fixture.channel);
        let call = start_call(channel, COLLECT, &[]);
        let (_, old) = lend(call, from);
        write(old, b"hello");

        let panicking = Panicking::in_charge_at(ChargeStep::Begun);
        let (status, answered) = resize(old, to, 5);
        drop(panicking);

        assert_eq!(status, ak_status::AK_STATUS_INTERNAL, "{context}");
        assert_eq!(answered.owner, old.owner, "{context}: the host's buffer");
        assert_eq!(debt_of(call).buffers_lent, 1, "{context}");
        assert_eq!(
            memory_usage(host.runtime).bytes_used,
            from as u64,
            "{context}"
        );
        assert_eq!(read(old, 5), b"hello", "{context}");

        let (status, resized) = resize(old, to, 5);
        assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");
        assert_eq!(read(resized, 5), b"hello", "{context}");
        assert_eq!(
            memory_usage(host.runtime).bytes_used,
            to as u64,
            "{context}"
        );

        unsafe { ak_return_call_buffer(resized) };
        assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{context}");
        assert_eq!(
            unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
        host.recorder.await_terminal();
        fixture.close();
        assert_nothing_owed(host, &context);
    }
}

/// Once the charge has moved the exchange is made, whatever a panic does to what follows the step,
/// giving up spares for room and waking the sends that wait: the host is told it succeeded, holds
/// the new buffer, and the charge is the new one's.
#[test]
fn a_panic_after_the_charge_moves_does_not_refuse_the_exchange() {
    let _turn = take_turn();
    for (from, to) in [(16, 4096), (4096, 16)] {
        let context = format!("{from} to {to}");
        let fixture = Host::connected();
        let (host, channel) = (&fixture.host, fixture.channel);
        let call = start_call(channel, COLLECT, &[]);
        let (_, old) = lend(call, from);
        write(old, b"hello");

        let panicking = Panicking::in_charge_at(ChargeStep::Moved);
        let (status, resized) = resize(old, to, 5);
        drop(panicking);

        assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");
        assert_eq!(resized.len, to, "{context}");
        assert_eq!(read(resized, 5), b"hello", "{context}");
        assert_eq!(debt_of(call).buffers_lent, 1, "{context}");
        assert_eq!(
            memory_usage(host.runtime).bytes_used,
            to as u64,
            "{context}"
        );

        unsafe { ak_return_call_buffer(resized) };
        assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{context}");
        assert_eq!(
            unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
        host.recorder.await_terminal();
        fixture.close();
        assert_nothing_owed(host, &context);
    }
}

/// What a runtime that was shut down comes to: nothing owed by the host and nothing charged.
fn assert_nothing_owed(host: &Host, context: &str) {
    assert_eq!(
        host.recorder.shutdown_debt(),
        Some(ak_host_debt::AK_HOST_NOTHING_TO_RETURN),
        "{context}: the shutdown found nothing the host still owes"
    );
    assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{context}");
}

/// A spare whose slack the ceiling has no room for is charged again with an arena of its own, and a
/// panic in that second charge leaves the old charge as the first, refused one did.
#[test]
fn a_panic_in_the_second_charge_leaves_the_old_buffer_lent_and_charged() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let _turn = take_turn();
    let ceiling = 3 * LARGE;
    let fixture = Connected::with_ceiling(ceiling as u64);
    let (host, channel) = (&fixture.host, fixture.channel);
    request(host, channel, LARGE);

    let other = start_call(channel, COLLECT, &[]);
    let call = start_call(channel, COLLECT, &[]);
    let (_, old) = lend(call, 16);
    write(old, b"hello");

    // Between the spare taken and the charge, another call charges what leaves the request room
    // and the slack none. Its own charges are not the resize's.
    let held = ceiling - LARGE + 512;
    let taken = Arc::new(Mutex::new(None));
    let inside = Arc::new(AtomicBool::new(false));
    let (hook_taken, hook_inside) = (Arc::clone(&taken), Arc::clone(&inside));
    let once = Arc::new(AtomicBool::new(false));
    hooks::before_charge(Some(Arc::new(move || {
        if !once.swap(true, Ordering::SeqCst) {
            hook_inside.store(true, Ordering::SeqCst);
            let (status, buffer) = lend(other, held);
            hook_inside.store(false, Ordering::SeqCst);
            assert_eq!(status, ak_status::AK_STATUS_OK);
            *hook_taken.lock().unwrap() =
                Some((buffer.ptr as usize, buffer.len, buffer.owner as usize));
        }
    })));
    let charges = Arc::new(AtomicUsize::new(0));
    hooks::at_each_charge_step(Some(Arc::new(move |reached| {
        if reached == ChargeStep::Begun
            && !inside.load(Ordering::SeqCst)
            && charges.fetch_add(1, Ordering::SeqCst) == 1
        {
            panic!("injected in the second charge");
        }
    })));
    let (status, _) = resize(old, LARGE - 1024, 5);
    hooks::before_charge(None);
    hooks::at_each_charge_step(None);

    assert_eq!(status, ak_status::AK_STATUS_INTERNAL);
    assert_eq!(
        memory_usage(host.runtime).bytes_used,
        (held + 16) as u64,
        "the old charge stands beside the other call's"
    );
    assert_eq!(read(old, 5), b"hello");

    unsafe { ak_return_call_buffer(old) };
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
    assert_nothing_owed(host, "the second charge");
}
