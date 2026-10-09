//! A panic while an overrun is taken back is still the overrun's answer, and the shutdown it
//! begins completes.

// The fixture serves every cardinality; which parts this binary reaches is not a fact about it.
#[allow(dead_code)]
mod support;

use std::sync::Arc;

use armonik_transport_ffi::hooks::{self, RepayStep};
use armonik_transport_ffi::*;
use support::host::*;
use support::COLLECT;

/// Takes the hook away however the test ends.
struct Panicking;

impl Panicking {
    /// Makes every payment of an overrun panic on reaching `step`.
    fn at(step: RepayStep) -> Self {
        hooks::at_each_repay_step(Some(Arc::new(move |reached| {
            if reached == step {
                panic!("injected at {reached:?}");
            }
        })));
        Self
    }
}

impl Drop for Panicking {
    fn drop(&mut self) {
        hooks::at_each_repay_step(None);
    }
}

/// The buffer is gone whatever the panic, and INTERNAL would have the host give back an owner that
/// no longer exists. The debt a panic left unpaid would hold the shutdown for good: the lend is
/// counted against the ledger, and the shutdown waits for the host to give back what it holds.
#[test]
fn a_panic_while_taking_back_an_overrun_answers_corrupted_and_the_shutdown_completes() {
    for step in [
        RepayStep::Begun,
        RepayStep::Released,
        RepayStep::Permitted,
        RepayStep::Counted,
    ] {
        let fixture = Host::connected();
        let (host, channel) = (&fixture.host, fixture.channel);
        let call = start_call(channel, COLLECT, &[]);
        let (_, buffer) = lend(call, 8);
        assert_eq!(memory_usage(host.runtime).bytes_used, 8, "{step:?}");

        let panicking = Panicking::at(step);
        let mut out = support::empty_buffer();
        let status =
            unsafe { ak_resize_call_buffer(buffer, 64, 9, &mut out, std::ptr::null_mut()) };
        drop(panicking);

        assert_eq!(status, ak_status::AK_STATUS_CORRUPTED, "{step:?}");
        assert!(out.owner.is_null(), "{step:?}");
        host.await_state(ak_runtime_state::AK_RUNTIME_QUIESCENT);
        assert_eq!(
            host.recorder.shutdown_debt(),
            Some(ak_host_debt::AK_HOST_NOTHING_TO_RETURN),
            "{step:?}: the shutdown found nothing the host still owes"
        );
        assert_eq!(memory_usage(host.runtime).bytes_used, 0, "{step:?}");
        support::await_call_reclaimed(call);
        fixture.close();
    }
}
