//! A panic in the charge of a received message leaves the ledger as it was before the move of the
//! bytes, or with the message held after it, so a runtime still reaches quiescence with nothing
//! charged and nothing counted.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::ThreadId;

use armonik_transport_ffi::hooks::{self, ChargeStep};
use armonik_transport_ffi::*;
use support::host::*;
use support::ECHO;

/// The hooks are process-wide, so the tests of this binary take turns.
static TURN: Mutex<()> = Mutex::new(());

fn take_turn() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// gRPC's INTERNAL, which the driver ends a call with when its sink panics.
const INTERNAL: i32 = 13;

/// Makes the charge of every received message panic on reaching `step`: the charges not made on the
/// thread that runs the test, whose lend is the only other one in these tests.
struct PanickingOnReceive;

impl PanickingOnReceive {
    fn at(step: ChargeStep) -> Self {
        let host: ThreadId = std::thread::current().id();
        hooks::at_each_charge_step(Some(Arc::new(move |reached| {
            if reached == step && std::thread::current().id() != host {
                panic!("injected at {reached:?}");
            }
        })));
        Self
    }
}

impl Drop for PanickingOnReceive {
    fn drop(&mut self) {
        hooks::at_each_charge_step(None);
    }
}

/// The one request `hello`, which the server echoes.
fn echo_hello(channel: ak_handle) -> ak_handle {
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
    send_one_request(call, b"hello");
    call
}

fn send_one_request(call: ak_handle, message: &[u8]) {
    let (status, buffer) = lend(call, message.len());
    assert_eq!(status, ak_status::AK_STATUS_OK);
    unsafe { std::ptr::copy_nonoverlapping(message.as_ptr(), buffer.ptr, message.len()) };
    assert_eq!(
        unsafe { ak_call_send_message(call, buffer, message.len(), std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
}

/// What a runtime comes to once it has been shut down: quiescent, with nothing owed by the host,
/// nothing charged, and the call reclaimed.
fn assert_shuts_down_clean(fixture: &Connected, call: ak_handle, context: &str) {
    let host = &fixture.host;
    fixture.close();
    assert_eq!(
        host.recorder.shutdown_debt(),
        Some(ak_host_debt::AK_HOST_NOTHING_TO_RETURN),
        "{context}: the shutdown found nothing the host still owes"
    );
    assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{context}");
    support::await_call_reclaimed(call);
}

/// A panic before the bytes are charged is the sink's panic, which ends the call with INTERNAL: the
/// message is not delivered, and the count its charge raised has gone back with the panic.
#[test]
fn a_panic_before_a_received_message_is_charged_leaves_nothing_counted() {
    let _turn = take_turn();
    let fixture = Host::connected();
    let host = &fixture.host;

    let panicking = PanickingOnReceive::at(ChargeStep::Begun);
    let call = echo_hello(fixture.channel);
    let seen = host.recorder.await_terminal();
    drop(panicking);

    assert_eq!(seen.status_code(), Some(INTERNAL));
    assert!(seen.message_payloads().is_empty());
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    assert_shuts_down_clean(&fixture, call, "Begun");
}

/// Once the bytes are charged the message is held: a panic in giving up the spares for room does
/// not lose its charge, and the host's consuming it gives it back.
#[test]
fn a_panic_after_a_received_message_is_charged_still_holds_it() {
    let _turn = take_turn();
    let fixture = Host::connected();
    let host = &fixture.host;
    host.recorder.hold_payloads();

    let panicking = PanickingOnReceive::at(ChargeStep::Moved);
    let call = echo_hello(fixture.channel);
    let seen = host.recorder.await_messages(1);
    drop(panicking);

    assert_eq!(seen.message_payloads(), vec![b"hello".to_vec()]);
    assert_eq!(
        memory_usage(host.runtime).bytes_used,
        5,
        "the message is charged until the host consumes it"
    );

    let seen = host.recorder.await_terminal();
    assert_eq!(seen.status_code(), Some(0));
    host.recorder.consume_all();
    assert_eq!(memory_usage(host.runtime).bytes_used, 0);
    assert_shuts_down_clean(&fixture, call, "Moved");
}
