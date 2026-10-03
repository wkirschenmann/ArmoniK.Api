//! Points where a test holds a host thread inside a downcall, to reach an interleaving the
//! scheduler reaches only by chance, and what a test reads of the library's own threads.
//! Compiled with the `test-hooks` feature only.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// What a hook runs on the thread that reaches it.
pub type Hook = Arc<dyn Fn() + Send + Sync>;

static BEFORE_CHARGE: Mutex<Option<Hook>> = Mutex::new(None);
static BEFORE_QUEUEING: Mutex<Option<Hook>> = Mutex::new(None);
static CHANNEL_THREAD_ENDING: Mutex<Option<Hook>> = Mutex::new(None);
static CHANNEL_THREADS: AtomicUsize = AtomicUsize::new(0);

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

/// Runs `hook` in every lend that has passed its checks, just before the ledger is charged.
/// `None` removes it.
pub fn before_charge(hook: Option<Hook>) {
    *BEFORE_CHARGE.lock().unwrap_or_else(PoisonError::into_inner) = hook;
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
