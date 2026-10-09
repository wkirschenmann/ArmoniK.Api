//! A lend refused with SLOT_BUSY or BUDGET_BUSY has given back everything it took, the call's one
//! buffer included, before the wake-up it waits for reaches the host. A host woken may ask again at
//! once, from inside the callback, and is served: AK_STATUS_INVALID_STATE on a live call whose host
//! holds no buffer is an answer an atomic lend never gives.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use armonik_transport_ffi::hooks::{self, RepayStep};
use armonik_transport_ffi::*;
use support::host::*;
use support::{poll_until, Recorder, TestServer, COLLECT, ECHO};

/// The hooks are process-wide, so the tests of this binary take turns.
static TURN: Mutex<()> = Mutex::new(());

fn take_turn() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A window of one slot: a message committed and not yet acquitted fills it.
const ONE_SLOT: &str = r#"{"Grpc":{"Host":{"Send":{"Window":1}}}}"#;

/// The steps of a refused lend's payment, all after its refusal is decided.
const STEPS: [RepayStep; 4] = [
    RepayStep::Begun,
    RepayStep::Released,
    RepayStep::Permitted,
    RepayStep::Counted,
];

/// How long a refused lend is held at a step for a wake-up that can arrive there to arrive.
const HOLD: Duration = Duration::from_millis(300);

const ASKED: usize = 16;

/// A buffer as plain numbers, which cross threads where its pointers do not.
type Parts = (usize, usize, usize);

fn parts(buffer: ak_buffer) -> Parts {
    (buffer.ptr as usize, buffer.len, buffer.owner as usize)
}

fn give_back((ptr, len, owner): Parts) {
    unsafe {
        ak_return_call_buffer(ak_buffer {
            ptr: ptr as *mut u8,
            len,
            owner: owner as *mut std::ffi::c_void,
        })
    };
}

/// The lend a host asks for once from inside a wake-up's callback, and its answer.
#[derive(Default)]
struct Retry {
    answer: Mutex<Option<(ak_status, Parts)>>,
    arrived: Condvar,
    asked: AtomicBool,
}

impl Retry {
    fn ask_once(&self, call: ak_handle) {
        if self.asked.swap(true, Ordering::SeqCst) {
            return;
        }
        let (status, buffer) = lend(call, ASKED);
        *self.answer.lock().unwrap_or_else(PoisonError::into_inner) = Some((status, parts(buffer)));
        self.arrived.notify_all();
    }

    fn within(&self, wait: Duration) -> Option<(ak_status, Parts)> {
        let answer = self.answer.lock().unwrap_or_else(PoisonError::into_inner);
        *self
            .arrived
            .wait_timeout_while(answer, wait, |answer| answer.is_none())
            .unwrap_or_else(PoisonError::into_inner)
            .0
    }
}

/// Keeps the channel's thread in a callback until it is opened, and with it every call's writer.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
    holding: AtomicBool,
}

impl Gate {
    fn hold(&self) {
        self.holding.store(true, Ordering::SeqCst);
        let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let _open = self
            .changed
            .wait_while(open, |open| !*open)
            .unwrap_or_else(PoisonError::into_inner);
    }

    fn open(&self) {
        *self.open.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.changed.notify_all();
    }
}

/// Takes the hook and the reaction away, and opens the gate, however the test ends.
struct Unhook<'a> {
    recorder: &'a Recorder,
    gate: Arc<Gate>,
}

impl Drop for Unhook<'_> {
    fn drop(&mut self) {
        hooks::at_each_repay_step(None);
        self.recorder.react(None);
        self.gate.open();
    }
}

/// Holds the lend this thread is refused at `step` of its payment: `wake` brings its wake-up about,
/// and the hold lasts until the host has asked again from inside that wake-up's callback, or for
/// `HOLD` if it cannot get there.
fn hold_the_refused_lend_at(
    step: RepayStep,
    retry: &Arc<Retry>,
    wake: impl Fn() + Send + Sync + 'static,
) {
    let lender = std::thread::current().id();
    let held = AtomicBool::new(false);
    let retry = Arc::clone(retry);
    hooks::at_each_repay_step(Some(Arc::new(move |reached| {
        if reached != step
            || std::thread::current().id() != lender
            || held.swap(true, Ordering::SeqCst)
        {
            return;
        }
        wake();
        retry.within(HOLD);
    })));
}

/// The answer the host had from inside the wake-up, which must be the lend: the window and the
/// ceiling both have room by then, and the host holds nothing.
fn assert_served(retry: &Retry, context: &str) -> Parts {
    let (status, buffer) = retry
        .within(Duration::from_secs(10))
        .unwrap_or_else(|| panic!("{context}: no wake-up came"));
    assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");
    buffer
}

/// SLOT_BUSY: the WRITE_DONE that frees the slot comes from another buffer's send, which the
/// channel's thread holds back until the refused lend is held.
#[test]
fn a_lend_asked_at_the_write_done_a_slot_busy_waits_for_is_served() {
    let _turn = take_turn();
    for step in STEPS {
        asked_at_the_write_done(step);
    }
}

fn asked_at_the_write_done(step: RepayStep) {
    let context = format!("{step:?}");
    let server = TestServer::start();
    let host = Host::start();
    let channel = host.channel_with(&server.endpoint, ONE_SLOT);
    let call = start_call(channel, COLLECT, &[]);

    let gate = Arc::new(Gate::default());
    let retry = Arc::new(Retry::default());
    let _unhook = Unhook {
        recorder: &host.recorder,
        gate: Arc::clone(&gate),
    };
    let (reaction_gate, reaction_retry) = (Arc::clone(&gate), Arc::clone(&retry));
    let asking = Arc::new(AtomicBool::new(false));
    let reaction_asking = Arc::clone(&asking);
    host.recorder.react(Some(Arc::new(move |kind| match kind {
        ak_event_kind::AK_EVENT_INITIAL_METADATA => reaction_gate.hold(),
        ak_event_kind::AK_EVENT_WRITE_DONE if reaction_asking.load(Ordering::SeqCst) => {
            reaction_retry.ask_once(call)
        }
        _ => {}
    })));

    // A call of one request has no WRITE_DONE to be mistaken for the other's, and its head is
    // where the gate holds the channel's thread.
    let holder = start_call_flagged(channel, ECHO, &[], AK_CALL_ONE_REQUEST);
    let (status, request) = lend(holder, 4);
    assert_eq!(status, ak_status::AK_STATUS_OK);
    assert_eq!(
        unsafe { ak_call_send_message(holder, request, 4, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    poll_until(
        || gate.holding.load(Ordering::SeqCst),
        || format!("{context}: the gate never held the channel's thread"),
    );

    // The one slot is spent by a message the writer cannot send yet.
    let (status, first) = lend(call, ASKED);
    assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");
    assert_eq!(
        unsafe { ak_call_send_message(call, first, ASKED, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK,
        "{context}"
    );

    asking.store(true, Ordering::SeqCst);
    let opener = Arc::clone(&gate);
    hold_the_refused_lend_at(step, &retry, move || opener.open());
    let (status, _) = lend(call, ASKED);
    hooks::at_each_repay_step(None);
    assert_eq!(status, ak_status::AK_STATUS_SLOT_BUSY, "{context}");

    give_back(assert_served(&retry, &context));
    assert_eq!(
        unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.recorder.await_terminals(2);
    ak_channel_release(channel);
    host.stop();
}

/// A ceiling that another call's lend of `HELD` leaves too little of for a lend of `ASKED`.
const SMALL_CEILING: u64 = 64;
const HELD: usize = 60;

/// BUDGET_BUSY: the release that wakes the refused lend is another call's buffer given back. A
/// call that sends one request has no task until the refusal spawns the one that raises it.
#[test]
fn a_lend_asked_at_the_budget_wake_a_budget_busy_waits_for_is_served() {
    let _turn = take_turn();
    for (method, flags) in [(COLLECT, 0), (ECHO, AK_CALL_ONE_REQUEST)] {
        for step in STEPS {
            asked_at_the_budget_wake(method, flags, step);
        }
    }
}

fn asked_at_the_budget_wake(method: &str, flags: u32, step: RepayStep) {
    let context = format!("flags {flags}, {step:?}");
    let server = TestServer::start();
    let host = Host::with_ceiling(SMALL_CEILING);
    let channel = host.channel_with(&server.endpoint, ONE_SLOT);
    let other = start_call(channel, COLLECT, &[]);
    let call = start_call_flagged(channel, method, &[], flags);
    let (status, held) = lend(other, HELD);
    assert_eq!(status, ak_status::AK_STATUS_OK, "{context}");
    let held = parts(held);

    let retry = Arc::new(Retry::default());
    let _unhook = Unhook {
        recorder: &host.recorder,
        gate: Arc::default(),
    };
    let reaction_retry = Arc::clone(&retry);
    host.recorder.react(Some(Arc::new(move |kind| {
        if kind == ak_event_kind::AK_EVENT_BUDGET_WAKE {
            reaction_retry.ask_once(call);
        }
    })));

    hold_the_refused_lend_at(step, &retry, move || give_back(held));
    let (status, _) = lend(call, ASKED);
    hooks::at_each_repay_step(None);
    assert_eq!(status, ak_status::AK_STATUS_BUDGET_BUSY, "{context}");

    give_back(assert_served(&retry, &context));
    for call in [call, other] {
        assert_eq!(
            unsafe { ak_call_cancel(call, std::ptr::null_mut()) },
            ak_status::AK_STATUS_OK
        );
    }
    host.recorder.await_terminals(2);
    ak_channel_release(channel);
    host.stop();
}
