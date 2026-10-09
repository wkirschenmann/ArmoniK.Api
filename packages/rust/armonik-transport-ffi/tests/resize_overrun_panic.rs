//! A panic while an overrun is taken back is still the overrun's answer.
//!
//! Alone in its binary: the panic leaves the call's debt unpaid, so the runtime never quiesces, and
//! the one runtime a process holds is not one another test could create again.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::Arc;

use armonik_transport_ffi::hooks;
use armonik_transport_ffi::*;
use support::host::*;
use support::COLLECT;

/// The buffer is gone whatever the panic, and INTERNAL would have the host give back an owner that
/// no longer exists.
#[test]
fn a_panic_while_taking_back_an_overrun_answers_corrupted() {
    let fixture = Host::connected();
    let call = start_call(fixture.channel, COLLECT, &[]);
    let (_, buffer) = lend(call, 8);

    hooks::after_overrun_abandoned(Some(Arc::new(|| {
        panic!("injected after the buffer is forgotten");
    })));
    let mut out = support::empty_buffer();
    let status = unsafe { ak_resize_call_buffer(buffer, 64, 9, &mut out, std::ptr::null_mut()) };
    hooks::after_overrun_abandoned(None);

    assert_eq!(status, ak_status::AK_STATUS_CORRUPTED);
    assert!(out.owner.is_null());
    assert_ne!(
        ak_runtime_status(fixture.host.runtime),
        ak_runtime_state::AK_RUNTIME_RUNNING,
        "the runtime is shutting down, as CORRUPTED says"
    );
    // Not closed: the debt the panic left unpaid would hold the close.
    std::mem::forget(fixture);
}
