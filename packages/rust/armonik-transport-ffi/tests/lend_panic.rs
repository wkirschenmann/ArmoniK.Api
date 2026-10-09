//! A panic inside a lend is a refused lend, as an allocator failure is: INTERNAL, no buffer, nothing
//! charged, no slot of the send window spent and the call's one buffer free, so the host may ask
//! again and the runtime still reaches quiescence with nothing owed.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use armonik_transport_ffi::hooks::{self, ChargeStep, LendStep, RepayStep};
use armonik_transport_ffi::*;
use support::host::*;
use support::{TestServer, COLLECT, ECHO};

/// The hooks are process-wide, so the tests of this binary take turns.
static TURN: Mutex<()> = Mutex::new(());

/// Holds the turn for the whole test: a lend of another test would meet this one's hook.
fn take_turn() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A window of one slot, so that a slot a refused lend kept is a lend that is never admitted again.
const ONE_SLOT: &str = r#"{"Grpc":{"Host":{"Send":{"Window":1}}}}"#;

const LEND_STEPS: [LendStep; 7] = [
    LendStep::Claimed,
    LendStep::Admitted,
    LendStep::Windowed,
    LendStep::Charged,
    LendStep::Allocated,
    LendStep::Backed,
    LendStep::Built,
];

const REPAY_STEPS: [RepayStep; 4] = [
    RepayStep::Begun,
    RepayStep::Released,
    RepayStep::Permitted,
    RepayStep::Counted,
];

/// Takes every hook away however the test ends.
struct Panicking;

impl Panicking {
    /// Makes every lend panic on reaching `step`.
    fn in_lend_at(step: LendStep) -> Self {
        hooks::at_each_lend_step(Some(Arc::new(move |reached| {
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

    /// Makes the payment of every buffer that is over for the host panic on reaching `step`.
    fn in_repay_at(self, step: RepayStep) -> Self {
        hooks::at_each_repay_step(Some(Arc::new(move |reached| {
            if reached == step {
                panic!("injected at {reached:?}");
            }
        })));
        self
    }
}

impl Drop for Panicking {
    fn drop(&mut self) {
        hooks::at_each_lend_step(None);
        hooks::at_each_charge_step(None);
        hooks::at_each_repay_step(None);
        hooks::before_charge(None);
    }
}

/// A runtime and a channel whose send window has one slot.
struct OneSlot {
    channel: ak_handle,
    host: Host,
    _server: TestServer,
}

impl OneSlot {
    fn start() -> Self {
        Self::with(Host::start())
    }

    fn with_ceiling(ceiling: u64) -> Self {
        Self::with(Host::with_ceiling(ceiling))
    }

    fn with(host: Host) -> Self {
        let server = TestServer::start();
        let channel = host.channel_with(&server.endpoint, ONE_SLOT);
        Self {
            channel,
            host,
            _server: server,
        }
    }

    fn close(&self) {
        ak_channel_release(self.channel);
        self.host.stop();
    }
}

/// The refusal a panic is: the host holds nothing, and the call and the ceiling are as they were.
fn assert_refused(fixture: &OneSlot, call: ak_handle, status: ak_status, context: &str) {
    assert_eq!(status, ak_status::AK_STATUS_INTERNAL, "{context}");
    assert_eq!(debt_of(call).buffers_lent, 0, "{context}");
    assert_eq!(
        memory_usage(fixture.host.runtime).bytes_used,
        0,
        "{context}"
    );
    assert_eq!(
        ak_runtime_status(fixture.host.runtime),
        ak_runtime_state::AK_RUNTIME_RUNNING,
        "{context}"
    );
}

/// What a host does on INTERNAL: it asks again. The call's one buffer and its only slot of the
/// window are free, and the buffer is charged what it asked for.
fn assert_lendable_again(fixture: &OneSlot, call: ak_handle, context: &str) {
    let (status, buffer) = lend(call, 16);
    assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");
    assert_eq!(debt_of(call).buffers_lent, 1, "{context}");
    assert_eq!(
        memory_usage(fixture.host.runtime).bytes_used,
        16,
        "{context}"
    );
    unsafe { ak_return_call_buffer(buffer) };
    assert_eq!(debt_of(call).buffers_lent, 0, "{context}");
    assert_eq!(
        memory_usage(fixture.host.runtime).bytes_used,
        0,
        "{context}"
    );
}

fn cancel(fixture: &OneSlot, call: ak_handle) {
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    fixture.host.recorder.await_terminal();
}

/// What a runtime comes to once it has been shut down: quiescent, with nothing owed by the host,
/// nothing charged, and the call reclaimed.
fn assert_shuts_down_clean(fixture: &OneSlot, call: ak_handle, context: &str) {
    let host = &fixture.host;
    host.stop();
    assert_eq!(
        host.recorder.shutdown_debt(),
        Some(ak_host_debt::AK_HOST_NOTHING_TO_RETURN),
        "{context}: the shutdown found nothing the host still owes"
    );
    assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{context}");
    support::await_call_reclaimed(call);
}

/// A panic at any step of a lend, before the host holds the buffer, refuses it: nothing is lent,
/// charged or spent, and the same lend is admitted when the host asks again.
#[test]
fn a_panic_before_the_host_holds_the_buffer_refuses_the_lend() {
    let _turn = take_turn();
    for step in LEND_STEPS {
        let context = format!("{step:?}");
        let fixture = OneSlot::start();
        let call = start_call(fixture.channel, COLLECT, &[]);

        let panicking = Panicking::in_lend_at(step);
        let (status, _) = lend(call, 16);
        drop(panicking);

        assert_refused(&fixture, call, status, &context);
        assert_lendable_again(&fixture, call, &context);
        cancel(&fixture, call);
        fixture.close();
    }
}

/// The same refusal is a runtime that can be shut down: the debt a panic left unpaid would be a
/// shutdown that waits for the host to give back a buffer nobody holds.
#[test]
fn a_runtime_whose_lend_panicked_reaches_quiescence_with_nothing_owed() {
    let _turn = take_turn();
    for step in LEND_STEPS {
        let context = format!("{step:?}");
        let fixture = OneSlot::start();
        let call = start_call(fixture.channel, COLLECT, &[]);

        let panicking = Panicking::in_lend_at(step);
        let (status, _) = lend(call, 16);
        drop(panicking);
        assert_eq!(status, ak_status::AK_STATUS_INTERNAL, "{context}");

        assert_shuts_down_clean(&fixture, call, &context);
    }
}

/// A panic in the charge of the bytes a lend asks for leaves them uncharged and uncounted: the
/// count a charge raises before it charges goes back with the panic.
#[test]
fn a_panic_before_the_bytes_are_charged_leaves_nothing_counted() {
    let _turn = take_turn();
    let fixture = OneSlot::start();
    let call = start_call(fixture.channel, COLLECT, &[]);

    let panicking = Panicking::in_charge_at(ChargeStep::Begun);
    let (status, _) = lend(call, 16);
    drop(panicking);

    assert_refused(&fixture, call, status, "Begun");
    assert_lendable_again(&fixture, call, "Begun");
    cancel(&fixture, call);
    assert_shuts_down_clean(&fixture, call, "Begun");
}

/// Once the count has moved the charge is made, and a panic in what follows it, giving up the
/// spares for room and waking the sends that wait, does not refuse the lend.
#[test]
fn a_panic_after_the_bytes_are_charged_does_not_refuse_the_lend() {
    let _turn = take_turn();
    let fixture = OneSlot::start();
    let call = start_call(fixture.channel, COLLECT, &[]);

    let panicking = Panicking::in_charge_at(ChargeStep::Moved);
    let (status, buffer) = lend(call, 16);
    drop(panicking);

    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(debt_of(call).buffers_lent, 1);
    assert_eq!(memory_usage(fixture.host.runtime).bytes_used, 16);
    unsafe { ak_return_call_buffer(buffer) };
    assert_eq!(memory_usage(fixture.host.runtime).bytes_used, 0);
    cancel(&fixture, call);
    assert_shuts_down_clean(&fixture, call, "Moved");
}

/// A refused lend pays its debt part by part, whatever another part does.
#[test]
fn a_panic_while_a_refused_lend_is_paid_leaves_the_rest_paid() {
    let _turn = take_turn();
    for step in REPAY_STEPS {
        let context = format!("repay {step:?}");
        let fixture = OneSlot::start();
        let call = start_call(fixture.channel, COLLECT, &[]);

        let panicking = Panicking::in_lend_at(LendStep::Charged).in_repay_at(step);
        let (status, _) = lend(call, 16);
        drop(panicking);

        assert_refused(&fixture, call, status, &context);
        assert_lendable_again(&fixture, call, &context);
        cancel(&fixture, call);
        assert_shuts_down_clean(&fixture, call, &context);
    }
}

/// What a host that asks for a buffer at each step of the payment of another, on the one slot of
/// the window, is answered: the steps it asked at with the answer, and the buffer it was lent.
type Asked = Arc<Mutex<Vec<(RepayStep, ak_status)>>>;
type Lent = Arc<Mutex<Option<(usize, usize, usize)>>>;

fn ask_at_each_repay_step(call: ak_handle) -> (Asked, Lent) {
    let asked = Asked::default();
    let lent = Lent::default();
    let inside = Arc::new(AtomicBool::new(false));
    let (hook_asked, hook_lent) = (Arc::clone(&asked), Arc::clone(&lent));
    hooks::at_each_repay_step(Some(Arc::new(move |step| {
        // The payment of the buffer this lends is not asked at.
        if inside.swap(true, Ordering::SeqCst) {
            return;
        }
        let (status, buffer) = lend(call, 8);
        if status == ak_status::AK_STATUS_OK {
            *hook_lent.lock().unwrap() =
                Some((buffer.ptr as usize, buffer.len, buffer.owner as usize));
        }
        hook_asked.lock().unwrap().push((step, status));
        inside.store(false, Ordering::SeqCst);
    })));
    (asked, lent)
}

/// A lend asked for during a payment is refused with INVALID_STATE at the first three steps and
/// admitted at the last, and never with SLOT_BUSY, whose wake-up is a WRITE_DONE nothing sent.
fn assert_asked_only_once_it_is_paid(
    asked: &Asked,
    lent: &Lent,
    fixture: &OneSlot,
    call: ak_handle,
) {
    hooks::at_each_repay_step(None);
    let asked = asked.lock().unwrap().clone();
    assert_eq!(
        asked,
        vec![
            (RepayStep::Begun, ak_status::AK_STATUS_INVALID_STATE),
            (RepayStep::Released, ak_status::AK_STATUS_INVALID_STATE),
            (RepayStep::Permitted, ak_status::AK_STATUS_INVALID_STATE),
            (RepayStep::Counted, ak_status::AK_STATUS_OK),
        ]
    );
    let (ptr, len, owner) = lent.lock().unwrap().take().expect("the last ask was lent");
    unsafe {
        ak_return_call_buffer(ak_buffer {
            ptr: ptr as *mut u8,
            len,
            owner: owner as *mut std::ffi::c_void,
        })
    };
    assert_lendable_again(fixture, call, "after the asks");
}

/// The payment of a refused lend, asked at from the same host.
#[test]
fn a_lend_asked_during_the_payment_of_a_refused_lend_is_not_told_to_wait_for_a_write() {
    let _turn = take_turn();
    let fixture = OneSlot::start();
    let call = start_call(fixture.channel, COLLECT, &[]);

    // Refused once, after the slot is spent and the bytes are charged: all three parts are owed.
    let refused = Arc::new(AtomicBool::new(false));
    hooks::at_each_lend_step(Some(Arc::new(move |reached| {
        if reached == LendStep::Charged && !refused.swap(true, Ordering::SeqCst) {
            panic!("injected at {reached:?}");
        }
    })));
    let _clear = Panicking;
    let (asked, lent) = ask_at_each_repay_step(call);
    let (status, _) = lend(call, 16);

    assert_eq!(status, ak_status::AK_STATUS_INTERNAL);
    assert_asked_only_once_it_is_paid(&asked, &lent, &fixture, call);
    cancel(&fixture, call);
    assert_shuts_down_clean(&fixture, call, "refused");
}

/// The payment of a buffer given back, asked at from the same host.
#[test]
fn a_lend_asked_during_the_return_of_a_buffer_is_not_told_to_wait_for_a_write() {
    let _turn = take_turn();
    let fixture = OneSlot::start();
    let call = start_call(fixture.channel, COLLECT, &[]);
    let (status, buffer) = lend(call, 16);
    assert_eq!(status, ak_status::AK_STATUS_OK);

    let _clear = Panicking;
    let (asked, lent) = ask_at_each_repay_step(call);
    unsafe { ak_return_call_buffer(buffer) };

    assert_asked_only_once_it_is_paid(&asked, &lent, &fixture, call);
    cancel(&fixture, call);
    assert_shuts_down_clean(&fixture, call, "returned");
}

/// A ceiling that another call's lend of `HELD` leaves too little of for a lend of `ASKED`.
const SMALL_CEILING: u64 = 64;
const HELD: usize = 60;
const ASKED: usize = 16;

/// A runtime whose ceiling another call's buffer nearly fills, and a call whose lend of `ASKED` is
/// refused for the budget.
fn with_the_budget_taken() -> (OneSlot, ak_handle, ak_handle, ak_buffer) {
    let fixture = OneSlot::with_ceiling(SMALL_CEILING);
    let other = start_call(fixture.channel, COLLECT, &[]);
    let call = start_call(fixture.channel, COLLECT, &[]);
    let (status, held) = lend(other, HELD);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    (fixture, call, other, held)
}

/// A lend refused for the budget leaves its send waiting for room, which is what its wake-up and
/// the lowered read ceiling are for.
#[test]
fn a_lend_refused_for_the_budget_leaves_its_send_waiting() {
    let _turn = take_turn();
    let (fixture, call, other, held) = with_the_budget_taken();

    let (status, _) = lend(call, ASKED);

    assert_eq!(status, ak_status::AK_STATUS_BUDGET_BUSY);
    assert_eq!(hooks::is_waiting_for_room(call), Some(true));
    unsafe { ak_return_call_buffer(held) };
    fixture.host.recorder.await_budget_wake();
    assert_lendable_again(&fixture, call, "after the wake-up");
    for call in [call, other] {
        cancel(&fixture, call);
    }
    fixture.close();
}

/// A panic once the send is recorded as waiting, in the second try of the charge, refuses the lend
/// as the others are, and the record goes with it: a refused lend holds nothing, and a send that
/// nothing is waiting for would lower the ceiling every call reads until its call ends.
#[test]
fn a_panic_after_the_send_is_recorded_as_waiting_takes_the_record_back() {
    let _turn = take_turn();
    let (fixture, call, other, held) = with_the_budget_taken();

    // The first charge of the lend is refused for the budget, and the second panics.
    let charges = Arc::new(AtomicUsize::new(0));
    hooks::at_each_charge_step(Some(Arc::new(move |reached| {
        if reached == ChargeStep::Begun && charges.fetch_add(1, Ordering::SeqCst) == 1 {
            panic!("injected in the second charge");
        }
    })));
    let (status, _) = lend(call, ASKED);
    hooks::at_each_charge_step(None);

    assert_eq!(status, ak_status::AK_STATUS_INTERNAL);
    assert_eq!(debt_of(call).buffers_lent, 0);
    assert_eq!(
        memory_usage(fixture.host.runtime).bytes_used,
        HELD as u64,
        "the other call's charge stands alone"
    );
    assert_eq!(hooks::is_waiting_for_room(call), Some(false));

    unsafe { ak_return_call_buffer(held) };
    assert_lendable_again(&fixture, call, "after the panic");
    for call in [call, other] {
        cancel(&fixture, call);
    }
    assert_shuts_down_clean(&fixture, call, "the second charge");
}

/// Past the size a channel keeps its arenas from.
const LARGE: usize = 128 * 1024;

/// One request of `len` bytes, answered and reclaimed: its arena is kept as a spare.
fn request(host: &Host, channel: ak_handle, len: usize) {
    let kept = hooks::spares_kept();
    let call = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
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

/// A lend that took a spare of the channel's is charged its slack as well, and a panic after the
/// slack is charged gives the whole of it back: the spare is lost with the panic, and nothing of it
/// stays charged.
#[test]
fn a_panic_after_taking_a_spare_gives_back_the_slack_it_charged() {
    let _turn = take_turn();
    for step in [LendStep::Allocated, LendStep::Backed, LendStep::Built] {
        let context = format!("{step:?}");
        let fixture = OneSlot::start();
        request(&fixture.host, fixture.channel, LARGE);
        let call = start_call(fixture.channel, COLLECT, &[]);

        let before = hooks::new_arenas();
        let panicking = Panicking::in_lend_at(step);
        let (status, _) = lend(call, LARGE - 1024);
        drop(panicking);

        assert_eq!(hooks::new_arenas() - before, 0, "{context}: a spare");
        assert_refused(&fixture, call, status, &context);
        assert_shuts_down_clean(&fixture, call, &context);
    }
}

/// A panic in the charge of the slack leaves the bytes asked for charged until the lend is
/// refused, and then not charged: what the first charge made is given back.
#[test]
fn a_panic_in_the_charge_of_a_spares_slack_gives_back_the_bytes_asked_for() {
    let _turn = take_turn();
    let fixture = OneSlot::start();
    request(&fixture.host, fixture.channel, LARGE);
    let call = start_call(fixture.channel, COLLECT, &[]);

    // The first charge of a lend is its bytes and the second is the slack of the spare.
    let charges = Arc::new(AtomicUsize::new(0));
    hooks::at_each_charge_step(Some(Arc::new(move |reached| {
        if reached == ChargeStep::Begun && charges.fetch_add(1, Ordering::SeqCst) == 1 {
            panic!("injected in the second charge");
        }
    })));
    let (status, _) = lend(call, LARGE - 1024);
    hooks::at_each_charge_step(None);

    assert_refused(&fixture, call, status, "the slack");
    assert_lendable_again(&fixture, call, "the slack");
    cancel(&fixture, call);
    assert_shuts_down_clean(&fixture, call, "the slack");
}

/// A spare whose slack the ceiling has no room for gives way to an arena of its own, and a panic at
/// that arena refuses the lend as the others: the bytes asked for are given back, and none of the
/// slack was charged.
#[test]
fn a_panic_in_the_arena_taken_for_want_of_room_gives_back_the_bytes_asked_for() {
    let _turn = take_turn();
    let ceiling = 3 * LARGE;
    let fixture = OneSlot::with_ceiling(ceiling as u64);
    request(&fixture.host, fixture.channel, LARGE);

    let other = start_call(fixture.channel, COLLECT, &[]);
    let call = start_call(fixture.channel, COLLECT, &[]);
    let len = LARGE - 1024;

    // Between the bytes charged and the slack, another call charges what leaves the request room
    // and the slack none.
    let held = ceiling - len - 512;
    let taken = Arc::new(Mutex::new(None));
    let inside = Arc::new(AtomicBool::new(false));
    let arenas = Arc::new(AtomicUsize::new(0));
    let (hook_taken, hook_inside) = (Arc::clone(&taken), Arc::clone(&inside));
    hooks::at_each_lend_step(Some(Arc::new(move |reached| {
        if reached != LendStep::Allocated || hook_inside.load(Ordering::SeqCst) {
            return;
        }
        match arenas.fetch_add(1, Ordering::SeqCst) {
            0 => {
                hook_inside.store(true, Ordering::SeqCst);
                let (status, buffer) = lend(other, held);
                hook_inside.store(false, Ordering::SeqCst);
                assert_eq!(status, ak_status::AK_STATUS_OK);
                *hook_taken.lock().unwrap() =
                    Some((buffer.ptr as usize, buffer.len, buffer.owner as usize));
            }
            _ => panic!("injected at the second arena"),
        }
    })));
    let (status, _) = lend(call, len);
    hooks::at_each_lend_step(None);

    assert_eq!(status, ak_status::AK_STATUS_INTERNAL);
    assert_eq!(debt_of(call).buffers_lent, 0);
    assert_eq!(
        memory_usage(fixture.host.runtime).bytes_used,
        held as u64,
        "the other call's charge stands alone"
    );

    let (ptr, len, owner) = taken.lock().unwrap().take().expect("the hook ran");
    unsafe {
        ak_return_call_buffer(ak_buffer {
            ptr: ptr as *mut u8,
            len,
            owner: owner as *mut std::ffi::c_void,
        })
    };
    assert_eq!(memory_usage(fixture.host.runtime).bytes_used, 0);
    assert_lendable_again(&fixture, call, "after");
    for call in [call, other] {
        assert_eq!(
            unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
    }
    fixture.close();
}
