//! Points where a test holds a host thread inside a downcall, to reach an interleaving the
//! scheduler reaches only by chance. Compiled with the `test-hooks` feature only.

use std::sync::{Arc, Mutex, PoisonError};

/// What a hook runs on the thread that reaches it.
pub type Hook = Arc<dyn Fn() + Send + Sync>;

static BEFORE_CHARGE: Mutex<Option<Hook>> = Mutex::new(None);

/// Runs `hook` in every lend that has passed its checks, just before the ledger is charged.
/// `None` removes it.
pub fn before_charge(hook: Option<Hook>) {
    *BEFORE_CHARGE.lock().unwrap_or_else(PoisonError::into_inner) = hook;
}

// Cloned out of the lock before it runs, so a hook that blocks holds no lock while it does.
pub(crate) fn run_before_charge() {
    let hook = BEFORE_CHARGE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(hook) = hook {
        hook();
    }
}
