//! Points where a test runs its own code inside the engine, to reach a path nothing else reaches,
//! and counts of what the engine writes and of the rounds its deliveries wait. Compiled with the
//! `test-hooks` feature only.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// What a hook runs on the thread that reaches it.
pub type Hook = Arc<dyn Fn() + Send + Sync>;

static IN_DRIVER: Mutex<Option<Hook>> = Mutex::new(None);
static IN_DIAL: Mutex<Option<Hook>> = Mutex::new(None);
static WRITES: AtomicUsize = AtomicUsize::new(0);
static DELIVERY_ROUNDS: AtomicUsize = AtomicUsize::new(0);

/// How many writes the HTTP/2 connections of this process have made to the stream under them,
/// TLS's when there is one: a write taken in parts counts each part.
pub fn writes() -> usize {
    WRITES.load(Ordering::SeqCst)
}

pub(crate) fn count_write() {
    WRITES.fetch_add(1, Ordering::SeqCst);
}

/// How many rounds of the runtime the deliveries of this process's responses have waited for
/// their next read.
pub fn delivery_rounds() -> usize {
    DELIVERY_ROUNDS.load(Ordering::SeqCst)
}

pub(crate) fn count_delivery_round() {
    DELIVERY_ROUNDS.fetch_add(1, Ordering::SeqCst);
}

/// Runs `hook` at the start of every call's driver. `None` removes it.
pub fn in_driver(hook: Option<Hook>) {
    set(&IN_DRIVER, hook);
}

/// Runs `hook` at the start of every dial, before the connection is opened. `None` removes it.
pub fn in_dial(hook: Option<Hook>) {
    set(&IN_DIAL, hook);
}

pub(crate) fn run_in_driver() {
    run(&IN_DRIVER);
}

pub(crate) fn run_in_dial() {
    run(&IN_DIAL);
}

fn set(point: &Mutex<Option<Hook>>, hook: Option<Hook>) {
    *point.lock().unwrap_or_else(PoisonError::into_inner) = hook;
}

// Cloned out of the lock before it runs, so a hook that panics leaves the lock unpoisoned.
fn run(point: &Mutex<Option<Hook>>) {
    let hook = point.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(hook) = hook {
        hook();
    }
}
