//! Points where a test runs its own code inside the engine, to reach a path nothing else reaches.
//! Compiled with the `test-hooks` feature only.

use std::sync::{Arc, Mutex, PoisonError};

/// What a hook runs on the thread that reaches it.
pub type Hook = Arc<dyn Fn() + Send + Sync>;

static IN_DRIVER: Mutex<Option<Hook>> = Mutex::new(None);
static IN_DIAL: Mutex<Option<Hook>> = Mutex::new(None);

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
