//! Points where a test holds a host thread inside a downcall, to reach an interleaving the
//! scheduler reaches only by chance, and what a test reads of the library's own threads.
//! Compiled with the `test-hooks` feature only.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// What a hook runs on the thread that reaches it.
pub type Hook = Arc<dyn Fn() + Send + Sync>;

/// What a step hook runs, told the step an entry point has reached.
pub type StepHook<S = ResizeStep> = Arc<dyn Fn(S) + Send + Sync>;

/// The points of an `ak_call_send_message` after it has taken the buffer from the host, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendStep {
    /// The buffer is taken and nothing is checked.
    Taken,
    /// The bytes the host says it wrote are within what was lent, and the sentinel is intact.
    Sealed,
    /// The call the handle names is the buffer's own.
    Resolved,
    /// The call admits the message and, on a stream, has room in its queue, and the buffer is
    /// still the host's. An empty message on a stream takes no buffer and does not reach this.
    Admitted,
    /// On a call that sends one request: the sending is ended, nothing is given yet, and the
    /// buffer is still the host's.
    Ending,
    /// The buffer's arena is taken to be the message: nothing of the host's buffer is left, and
    /// nothing is queued.
    Framing,
    /// The message is queued, and what is left is its accounting and telling whoever waits.
    Queued,
    /// On a call that sends one request: the request is given, and only the task is left to spawn.
    Spawning,
}

/// The points of an `ak_return_call_buffer` after it has taken the buffer from the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnStep {
    /// The buffer is taken and its sentinel not yet read.
    Taken,
}

/// The parts of the debt of a buffer that is over for the host, paid in this order, whether it is
/// given back, taken back as an overrun, lost to a panic, or sent as a one-request call's message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepayStep {
    /// Nothing is paid.
    Begun,
    /// The call's one buffer is no longer counted.
    Counted,
    /// The bytes the buffer was charged are given back.
    Released,
    /// The send window has its slot back.
    Permitted,
}

/// The points of an `ak_resize_call_buffer` after it has taken the buffer from the host, in order.
/// An overrun is out of this sequence: see `RepayStep`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeStep {
    /// The buffer is taken and nothing is checked.
    Taken,
    /// The buffer is intact and the call admits the new length.
    Admitted,
    /// The new arena is allocated.
    Allocated,
    /// The kept bytes are in the new arena and the charge is not yet made. Reached again when the
    /// first arena had no room for its slack and another is made.
    Copied,
    /// The charge is moved and the new arena is the buffer's: only the old arena is left to park.
    Exchanged,
}

static BEFORE_COPY_CHARGE: Mutex<Option<Hook>> = Mutex::new(None);
static RESIZE_STEP: Mutex<Option<StepHook>> = Mutex::new(None);
static SEND_STEP: Mutex<Option<StepHook<SendStep>>> = Mutex::new(None);
static RETURN_STEP: Mutex<Option<StepHook<ReturnStep>>> = Mutex::new(None);
static REPAY_STEP: Mutex<Option<StepHook<RepayStep>>> = Mutex::new(None);
static BEFORE_CHARGE: Mutex<Option<Hook>> = Mutex::new(None);
static BEFORE_QUEUEING: Mutex<Option<Hook>> = Mutex::new(None);
static CHANNEL_THREAD_ENDING: Mutex<Option<Hook>> = Mutex::new(None);
static CHANNEL_THREADS: AtomicUsize = AtomicUsize::new(0);
static NEW_ARENAS: AtomicUsize = AtomicUsize::new(0);

/// The options a runtime was created with, as its configuration loaded them.
pub fn runtime_options(
    runtime: crate::abi::ak_handle,
) -> Option<armonik_transport::options::RuntimeOptions> {
    crate::tables::runtimes()
        .get(runtime)
        .map(|found| found.options().clone())
}

/// How many arenas lends have allocated rather than taken from a channel's spares.
pub fn new_arenas() -> usize {
    NEW_ARENAS.load(Ordering::SeqCst)
}

pub(crate) fn count_new_arena() {
    NEW_ARENAS.fetch_add(1, Ordering::SeqCst);
}

static SPARES_KEPT: AtomicUsize = AtomicUsize::new(0);

/// How many arenas channels have kept as spares.
pub fn spares_kept() -> usize {
    SPARES_KEPT.load(Ordering::SeqCst)
}

pub(crate) fn count_spare_kept() {
    SPARES_KEPT.fetch_add(1, Ordering::SeqCst);
}

/// How many channels' threads are running, counted until each has dropped its tokio runtime.
pub fn channel_threads() -> usize {
    CHANNEL_THREADS.load(Ordering::SeqCst)
}

/// Counts a channel's thread for as long as it lives; the first local of the thread, so that it is
/// the last one dropped.
pub(crate) struct ChannelThreadAlive;

impl ChannelThreadAlive {
    pub(crate) fn new() -> Self {
        CHANNEL_THREADS.fetch_add(1, Ordering::SeqCst);
        ChannelThreadAlive
    }
}

impl Drop for ChannelThreadAlive {
    fn drop(&mut self) {
        run(&CHANNEL_THREAD_ENDING);
        CHANNEL_THREADS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Runs `hook` in every lend that has passed its checks, just before the ledger is charged, and in
/// every resize that has its new arena, just before its charge is changed. `None` removes it.
pub fn before_charge(hook: Option<Hook>) {
    *BEFORE_CHARGE.lock().unwrap_or_else(PoisonError::into_inner) = hook;
}

/// Runs `hook` at each step of every resize, which a test makes panic to see what a panic leaves.
/// `None` removes it.
pub fn at_each_resize_step(hook: Option<StepHook>) {
    *RESIZE_STEP.lock().unwrap_or_else(PoisonError::into_inner) = hook;
}

/// Runs `hook` in every send that has counted itself in, just before it looks at the call and
/// queues its command. `None` removes it.
pub fn before_queueing(hook: Option<Hook>) {
    *BEFORE_QUEUEING
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = hook;
}

/// Runs `hook` on every channel's thread as it ends, before it stops being counted. `None`
/// removes it.
pub fn channel_thread_ending(hook: Option<Hook>) {
    *CHANNEL_THREAD_ENDING
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = hook;
}

/// Runs `hook` at each step of every send of a buffer, which a test makes panic to see what a
/// panic leaves. `None` removes it.
pub fn at_each_send_step(hook: Option<StepHook<SendStep>>) {
    *SEND_STEP.lock().unwrap_or_else(PoisonError::into_inner) = hook;
}

/// Runs `hook` at each step of every return of a buffer. `None` removes it.
pub fn at_each_return_step(hook: Option<StepHook<ReturnStep>>) {
    *RETURN_STEP.lock().unwrap_or_else(PoisonError::into_inner) = hook;
}

/// Runs `hook` at each step of the payment of every buffer that is over for the host, which a test
/// makes panic to see that the debt is paid whatever a step does. `None` removes it.
pub fn at_each_repay_step(hook: Option<StepHook<RepayStep>>) {
    *REPAY_STEP.lock().unwrap_or_else(PoisonError::into_inner) = hook;
}

/// Runs `hook` in every compressed copy that asks the ceiling for room, just before it is
/// charged. `None` removes it.
pub fn before_copy_charge(hook: Option<Hook>) {
    *BEFORE_COPY_CHARGE
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = hook;
}

pub(crate) fn run_before_copy_charge() {
    run(&BEFORE_COPY_CHARGE);
}

pub(crate) fn at_resize_step(step: ResizeStep) {
    reach(&RESIZE_STEP, step);
}

pub(crate) fn at_send_step(step: SendStep) {
    reach(&SEND_STEP, step);
}

pub(crate) fn at_return_step(step: ReturnStep) {
    reach(&RETURN_STEP, step);
}

pub(crate) fn at_repay_step(step: RepayStep) {
    reach(&REPAY_STEP, step);
}

fn reach<S>(slot: &Mutex<Option<StepHook<S>>>, step: S) {
    let hook = slot.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(hook) = hook {
        hook(step);
    }
}

pub(crate) fn run_before_charge() {
    run(&BEFORE_CHARGE);
}

pub(crate) fn run_before_queueing() {
    run(&BEFORE_QUEUEING);
}

// Cloned out of the lock before it runs, so a hook that blocks holds no lock while it does.
fn run(slot: &Mutex<Option<Hook>>) {
    let hook = slot.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(hook) = hook {
        hook();
    }
}
