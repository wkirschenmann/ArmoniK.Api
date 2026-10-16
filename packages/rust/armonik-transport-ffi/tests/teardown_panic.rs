//! A panic in a shutdown once it has stored GRPC_STOPPED leaves the runtime there, and the runtime
//! still reaches QUIESCENT: GRPC_STOPPED is the level-0 RELEASED, which no failure leaves. A
//! binary of its own, because the hook is process-wide and a runtime the defect fails is never
//! destroyed.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::Arc;

use armonik_transport_ffi::hooks;
use armonik_transport_ffi::*;
use support::host::*;

#[test]
fn a_panic_after_the_runtime_reads_stopped_leaves_it_to_reach_quiescence() {
    let host = Host::start();
    hooks::after_stopped(Some(Arc::new(|| {
        panic!("injected once GRPC_STOPPED is stored")
    })));

    assert_eq!(
        unsafe { ak_runtime_begin_shutdown(host.runtime, std::ptr::null_mut()) },
        ak_status::AK_STATUS_OK
    );
    host.await_state(ak_runtime_state::AK_RUNTIME_QUIESCENT);
    hooks::after_stopped(None);
}
