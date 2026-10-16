//! A shutdown that cannot get its teardown thread fails the runtime, and fails it from
//! GRPC_STOPPING: GRPC_STOPPED is the level-0 RELEASED, which no failure leaves. One test in a
//! binary of its own, because a failed runtime is never destroyed and keeps the process's claim.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::{Arc, Mutex, PoisonError};

use armonik_transport_ffi::hooks;
use armonik_transport_ffi::*;
use support::host::*;

#[test]
fn a_teardown_thread_refused_fails_the_runtime_before_it_reads_stopped() {
    let host = Host::start();
    let runtime = host.runtime;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let reads = Arc::clone(&seen);
    hooks::at_teardown_spawn(Some(Arc::new(move || {
        reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ak_runtime_status(runtime));
        true
    })));

    assert_eq!(
        unsafe { ak_runtime_begin_shutdown(runtime, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.await_state(ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED);
    hooks::at_teardown_spawn(None);

    assert_eq!(
        *seen.lock().unwrap_or_else(PoisonError::into_inner),
        vec![ak_runtime_state::AK_RUNTIME_GRPC_STOPPING],
        "the thread is asked for before the runtime reads stopped"
    );
    assert_eq!(
        unsafe { ak_runtime_destroy(runtime, std::ptr::null_mut()) },
        ak_status::AK_STATUS_INVALID_STATE
    );
    // Not dropped: its teardown would wait for a quiescence a failed runtime never reaches.
    std::mem::forget(host);
}
