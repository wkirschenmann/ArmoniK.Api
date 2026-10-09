//! A panic inside the commit or the return of a lent buffer leaves the buffer as the answer says it
//! is: the host's, lent and charged, when the answer is a refusal; the message's, when the answer is
//! OK; and gone, with the runtime shutting down, when the answer is CORRUPTED. A return has no
//! answer, and pays its debt whatever it meets.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use armonik_transport_ffi::hooks::{self, RepayStep, ReturnStep, SendStep};
use armonik_transport_ffi::*;
use support::host::*;
use support::{COLLECT, ECHO};

/// The hooks are process-wide, so the tests of this binary take turns.
static TURN: Mutex<()> = Mutex::new(());

/// Holds the turn for the whole test: a send of another test would meet this one's hook.
fn take_turn() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Takes every hook away however the test ends.
struct Panicking;

impl Panicking {
    /// Makes every send panic on reaching `step`.
    fn in_send_at(step: SendStep) -> Self {
        hooks::at_each_send_step(Some(Arc::new(move |reached| {
            if reached == step {
                panic!("injected at {reached:?}");
            }
        })));
        Self
    }

    /// Makes every return panic on reaching `step`.
    fn in_return_at(step: ReturnStep) -> Self {
        hooks::at_each_return_step(Some(Arc::new(move |reached| {
            if reached == step {
                panic!("injected at {reached:?}");
            }
        })));
        Self
    }

    /// Makes the payment of every buffer that is over for the host panic on reaching `step`.
    fn in_repay_at(step: RepayStep) -> Self {
        hooks::at_each_repay_step(Some(Arc::new(move |reached| {
            if reached == step {
                panic!("injected at {reached:?}");
            }
        })));
        Self
    }
}

impl Drop for Panicking {
    fn drop(&mut self) {
        hooks::at_each_send_step(None);
        hooks::at_each_return_step(None);
        hooks::at_each_repay_step(None);
    }
}

const REPAY_STEPS: [RepayStep; 4] = [
    RepayStep::Begun,
    RepayStep::Released,
    RepayStep::Permitted,
    RepayStep::Counted,
];

/// The two ways a call takes its messages, which commit by different code.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// Messages queued for a writer, and an end of the sending.
    Stream,
    /// The one request, whose commit ends the sending.
    OneRequest,
}

const SHAPES: [Shape; 2] = [Shape::Stream, Shape::OneRequest];

impl Shape {
    fn start(self, channel: ak_handle) -> ak_handle {
        match self {
            Shape::Stream => start_call(channel, COLLECT, &[]),
            Shape::OneRequest => start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST),
        }
    }

    /// The points of a commit at which the buffer is still the host's.
    fn steps_before_the_arena(self) -> Vec<SendStep> {
        let mut steps = vec![
            SendStep::Taken,
            SendStep::Sealed,
            SendStep::Resolved,
            SendStep::Admitted,
        ];
        if let Shape::OneRequest = self {
            steps.push(SendStep::Ending);
        }
        steps
    }

    /// Lets a call whose message `hello` was sent end, and answers what it received.
    fn finish(self, host: &Host, call: ak_handle) -> Vec<Vec<u8>> {
        if let Shape::Stream = self {
            host.recorder.await_write_done();
            assert_eq!(
                unsafe { ak_call_end_send(call, std::ptr::null_mut()) },
                ak_status::AK_STATUS_OK
            );
        }
        host.recorder.await_terminal().message_payloads()
    }

    /// What `finish` answers for the message `hello`.
    fn received(self) -> Vec<Vec<u8>> {
        match self {
            Shape::Stream => vec![b"1:hello".to_vec()],
            Shape::OneRequest => vec![b"hello".to_vec()],
        }
    }

    /// Lets a call whose buffer was given back end.
    fn cancel(self, host: &Host, call: ak_handle) {
        assert_eq!(
            unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
        host.recorder.await_terminal();
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

fn send(call: ak_handle, buffer: ak_buffer, written: usize) -> ak_status {
    unsafe { ak_call_send_message(call, buffer, written, std::ptr::null_mut()) }
}

/// A buffer lent at 16 bytes with `hello` written to it.
fn lent_with_hello(call: ak_handle) -> ak_buffer {
    let (status, buffer) = lend(call, 16);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    write(buffer, b"hello");
    buffer
}

/// What a runtime that is shutting down because of a lost buffer comes to: quiescent, with nothing
/// owed by the host, nothing charged, and the call reclaimed.
fn assert_shut_down_clean(fixture: &Connected, call: ak_handle, context: &str) {
    let host = &fixture.host;
    assert_ne!(
        ak_runtime_status(host.runtime),
        ak_runtime_state::AK_RUNTIME_RUNNING,
        "{context}: the runtime is shutting down"
    );
    host.await_state(ak_runtime_state::AK_RUNTIME_QUIESCENT);
    assert_eq!(
        host.recorder.shutdown_debt(),
        Some(ak_host_debt::AK_HOST_NOTHING_TO_RETURN),
        "{context}: the shutdown found nothing the host still owes"
    );
    assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{context}");
    support::await_call_reclaimed(call);
}

/// A commit that panics before it has taken the arena is a refusal like the others: the buffer is
/// lent, charged and the host's, and the same commit goes through when the host retries.
#[test]
fn a_panic_before_the_arena_is_taken_leaves_the_buffer_lent_and_charged() {
    let _turn = take_turn();
    for shape in SHAPES {
        for step in shape.steps_before_the_arena() {
            let context = format!("{shape:?} at {step:?}");
            let fixture = Host::connected();
            let (host, channel) = (&fixture.host, fixture.channel);
            let call = shape.start(channel);
            let buffer = lent_with_hello(call);

            let panicking = Panicking::in_send_at(step);
            let status = send(call, buffer, 5);
            drop(panicking);

            assert_eq!(status, ak_status::AK_STATUS_INTERNAL, "{context}");
            assert_eq!(debt_of(call).buffers_lent, 1, "{context}");
            assert_eq!(memory_usage(host.runtime).bytes_used, 16, "{context}");
            assert_eq!(read(buffer, 5), b"hello", "{context}");
            assert_eq!(
                ak_runtime_status(host.runtime),
                ak_runtime_state::AK_RUNTIME_RUNNING,
                "{context}"
            );

            // The retry a host makes on a refusal: the buffer is still the one it holds.
            assert_eq!(send(call, buffer, 5), ak_status::AK_STATUS_OK, "{context}");
            assert_eq!(debt_of(call).buffers_lent, 0, "{context}");
            assert_eq!(shape.finish(host, call), shape.received(), "{context}");
            fixture.close();
        }
    }
}

/// The other half of what a host does on a refusal: it gives the buffer back, which the library
/// still owns and charges, and the call and the ceiling are as before the lend.
#[test]
fn a_buffer_is_returnable_after_a_panic_in_its_commit() {
    let _turn = take_turn();
    for shape in SHAPES {
        for step in shape.steps_before_the_arena() {
            let context = format!("{shape:?} at {step:?}");
            let fixture = Host::connected();
            let (host, channel) = (&fixture.host, fixture.channel);
            let call = shape.start(channel);
            let buffer = lent_with_hello(call);

            let panicking = Panicking::in_send_at(step);
            let status = send(call, buffer, 5);
            drop(panicking);
            assert_eq!(status, ak_status::AK_STATUS_INTERNAL, "{context}");

            unsafe { ak_return_call_buffer(buffer) };
            assert_eq!(debt_of(call).buffers_lent, 0, "{context}");
            assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{context}");

            // The window slot came back with it: another lend is admitted.
            let (status, again) = lend(call, 8);
            assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");
            unsafe { ak_return_call_buffer(again) };
            shape.cancel(host, call);
            fixture.close();
        }
    }
}

/// A panic once the arena is the message's cannot give the buffer back: it is gone, which is what
/// CORRUPTED says, and the runtime shuts down. The debt is paid, so the shutdown completes.
#[test]
fn a_panic_once_the_arena_is_taken_loses_the_buffer_and_shuts_the_runtime_down() {
    let _turn = take_turn();
    for shape in SHAPES {
        let context = format!("{shape:?}");
        let fixture = Host::connected();
        let call = shape.start(fixture.channel);
        let buffer = lent_with_hello(call);

        let panicking = Panicking::in_send_at(SendStep::Framing);
        let status = send(call, buffer, 5);
        drop(panicking);

        assert_eq!(status, ak_status::AK_STATUS_CORRUPTED, "{context}");
        assert_shut_down_clean(&fixture, call, &context);
        fixture.close();
    }
}

/// Once the message is queued the commit is made, and a panic in what is left of it does not
/// unmake it: the host is told it succeeded, and the buffer is the message's.
#[test]
fn a_panic_after_the_message_is_queued_does_not_refuse_it() {
    let _turn = take_turn();
    for shape in SHAPES {
        let context = format!("{shape:?}");
        let fixture = Host::connected();
        let (host, channel) = (&fixture.host, fixture.channel);
        let call = shape.start(channel);
        let buffer = lent_with_hello(call);

        let panicking = Panicking::in_send_at(SendStep::Queued);
        let status = send(call, buffer, 5);
        drop(panicking);

        assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");
        assert_eq!(debt_of(call).buffers_lent, 0, "{context}");
        assert_eq!(
            ak_runtime_status(host.runtime),
            ak_runtime_state::AK_RUNTIME_RUNNING,
            "{context}"
        );
        assert_eq!(shape.finish(host, call), shape.received(), "{context}");
        assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{context}");
        fixture.close();
    }
}

/// The accounting of a request that is given is a return's payment, and whatever a part of it does
/// the request is sent, the debt is paid and the call goes on.
#[test]
fn a_panic_in_a_part_of_the_accounting_of_a_given_request_does_not_refuse_it() {
    let _turn = take_turn();
    for step in REPAY_STEPS {
        let context = format!("{step:?}");
        let fixture = Host::connected();
        let (host, channel) = (&fixture.host, fixture.channel);
        let call = Shape::OneRequest.start(channel);
        let buffer = lent_with_hello(call);

        let panicking = Panicking::in_repay_at(step);
        let status = send(call, buffer, 5);
        drop(panicking);

        assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");
        assert_eq!(
            Shape::OneRequest.finish(host, call),
            Shape::OneRequest.received(),
            "{context}"
        );
        assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{context}");
        assert_eq!(
            ak_runtime_status(host.runtime),
            ak_runtime_state::AK_RUNTIME_RUNNING,
            "{context}"
        );
        fixture.close();
    }
}

/// A call whose task cannot be spawned never ends, so the runtime shuts down, which cancels it and
/// spawns the task for that; the request was given, and the answer is OK.
#[test]
fn a_panic_where_the_task_is_spawned_shuts_the_runtime_down_and_still_answers_ok() {
    let _turn = take_turn();
    let fixture = Host::connected();
    let call = Shape::OneRequest.start(fixture.channel);
    let buffer = lent_with_hello(call);

    let panicking = Panicking::in_send_at(SendStep::Spawning);
    let status = send(call, buffer, 5);
    drop(panicking);

    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_shut_down_clean(&fixture, call, "a one-request commit");
    fixture.close();
}

/// A return has no answer, and it is complete whatever a part of its payment does: the buffer is
/// no longer counted, its bytes are given back and its slot is the window's again.
#[test]
fn a_panic_in_a_part_of_a_return_leaves_the_rest_paid() {
    let _turn = take_turn();
    for step in REPAY_STEPS {
        let context = format!("{step:?}");
        let fixture = Host::connected();
        let (host, channel) = (&fixture.host, fixture.channel);
        let call = start_call(channel, COLLECT, &[]);
        let buffer = lent_with_hello(call);

        let panicking = Panicking::in_repay_at(step);
        unsafe { ak_return_call_buffer(buffer) };
        drop(panicking);

        assert_eq!(debt_of(call).buffers_lent, 0, "{context}");
        assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{context}");
        assert_eq!(
            ak_runtime_status(host.runtime),
            ak_runtime_state::AK_RUNTIME_RUNNING,
            "{context}: a buffer given back is not a fault"
        );
        let (status, again) = lend(call, 8);
        assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");
        unsafe { ak_return_call_buffer(again) };
        Shape::Stream.cancel(host, call);
        fixture.close();
    }
}

/// A return whose check of the memory panics cannot say the memory is sound: it is treated as an
/// overrun, the arena not freed and the runtime shut down, with its debt paid.
#[test]
fn a_panic_in_the_check_of_a_return_is_treated_as_an_overrun() {
    let _turn = take_turn();
    let fixture = Host::connected();
    let call = start_call(fixture.channel, COLLECT, &[]);
    let buffer = lent_with_hello(call);

    let panicking = Panicking::in_return_at(ReturnStep::Taken);
    unsafe { ak_return_call_buffer(buffer) };
    drop(panicking);

    assert_shut_down_clean(&fixture, call, "a return");
    fixture.close();
}

/// An overrun found by a commit is CORRUPTED whatever a part of taking it back does, and the
/// shutdown it begins completes.
#[test]
fn a_panic_while_a_commit_takes_back_an_overrun_answers_corrupted() {
    let _turn = take_turn();
    for shape in SHAPES {
        for step in REPAY_STEPS {
            let context = format!("{shape:?} at {step:?}");
            let fixture = Host::connected();
            let call = shape.start(fixture.channel);
            let (status, buffer) = lend(call, 8);
            assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");

            let panicking = Panicking::in_repay_at(step);
            let status = send(call, buffer, 9);
            drop(panicking);

            assert_eq!(status, ak_status::AK_STATUS_CORRUPTED, "{context}");
            assert_shut_down_clean(&fixture, call, &context);
            fixture.close();
        }
    }
}

/// An overrun found by a return shuts the runtime down whatever a part of taking it back does.
#[test]
fn a_panic_while_a_return_takes_back_an_overrun_does_not_hold_the_shutdown() {
    let _turn = take_turn();
    for step in REPAY_STEPS {
        let context = format!("{step:?}");
        let fixture = Host::connected();
        let call = start_call(fixture.channel, COLLECT, &[]);
        let (status, buffer) = lend(call, 8);
        assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");
        // One byte past the lend, onto what this library put after it.
        unsafe { std::ptr::write_bytes(buffer.ptr, 0x41, 9) };

        let panicking = Panicking::in_repay_at(step);
        unsafe { ak_return_call_buffer(buffer) };
        drop(panicking);

        assert_shut_down_clean(&fixture, call, &context);
        fixture.close();
    }
}
