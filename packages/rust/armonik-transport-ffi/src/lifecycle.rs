use std::sync::Arc;

use crate::abi::{ak_event_kind, ak_handle, ak_host_debt, ak_runtime_state, ak_status};
use crate::channel::AkChannel;
use crate::host::Host;
use crate::runtime::{AkRuntime, Claim};
use crate::tables;

pub(crate) fn create_runtime(
    worker_threads: u32,
    memory_ceiling: u64,
    host: Host,
) -> Result<ak_handle, ak_status> {
    let claim = Claim::take().ok_or(ak_status::AK_STATUS_INVALID_STATE)?;
    let runtime = AkRuntime::new(worker_threads, memory_ceiling, host)?;
    let handle = tables::runtimes()
        .insert(runtime)
        .ok_or(ak_status::AK_STATUS_INTERNAL)?;

    // Every exit above drops the claim; from here the runtime in the table owns it.
    claim.keep();
    Ok(handle)
}

pub(crate) fn destroy_runtime(handle: ak_handle) -> ak_status {
    let Some(found) = tables::runtimes().get(handle) else {
        return ak_status::AK_STATUS_HANDLE_STALE;
    };
    if found.state() != ak_runtime_state::AK_RUNTIME_QUIESCENT {
        return ak_status::AK_STATUS_INVALID_STATE;
    }

    // The remove is what picks one caller of two, and it has to come first. QUIESCENT is final, so
    // the check above stays true for both; a second destroy that reached the drains would take the
    // calls and channels of whatever runtime was created after the first one finished, and would
    // relinquish a claim it does not hold.
    if tables::runtimes().remove(handle).is_none() {
        return ak_status::AK_STATUS_HANDLE_STALE;
    }

    // From the remove on, this caller owns the claim, and the guard gives it back however it
    // leaves.
    //
    // Nothing here shuts tokio down: the teardown thread did, and QUIESCENT above is that thread
    // having finished. So this is a reap and three removals - it cannot panic, and it does not
    // hold the host for the length of a shutdown.
    let claim = Claim::held();
    tables::calls().drain();
    tables::channels().drain();
    found.join_teardown();
    drop(claim);
    ak_status::AK_STATUS_OK
}

pub(crate) fn begin_shutdown(runtime: &Arc<AkRuntime>) {
    if !runtime.start_stopping() {
        return;
    }

    let weak = Arc::downgrade(runtime);
    let host = Arc::clone(runtime.host());
    let ledger = Arc::clone(runtime.ledger());

    runtime.spawner().spawn(async move {
        let reached = |act: fn(&AkRuntime)| {
            if let Some(runtime) = weak.upgrade() {
                act(&runtime);
            }
        };

        reached(AkRuntime::close_the_gate);

        for channel in tables::channels().values() {
            close_channel(&channel);
        }
        let calls = tables::calls().values();
        for call in &calls {
            call.cancel();
        }
        for call in &calls {
            call.finished().await;
        }

        let debt = if ledger.empty() {
            ak_host_debt::AK_HOST_NOTHING_TO_RETURN
        } else {
            ak_host_debt::AK_HOST_MUST_RETURN
        };

        // The state before the event that announces it, so a host that reads the status from
        // inside the callback reads STOPPED. Nothing gates on STOPPED - destroy wants QUIESCENT,
        // the gate and `start_stopping` want RUNNING - so publishing it early costs nothing.
        //
        // Two signals, because the host may still hold payloads and buffers when the gRPC side
        // stops: this one says whether it does, and RESOURCES_RELEASED below says it has given
        // them all back. Only then is the runtime quiescent and `ak_runtime_destroy` accepted.
        reached(|runtime| runtime.set_state(ak_runtime_state::AK_RUNTIME_GRPC_STOPPED));
        host.signal_runtime(ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE, debt);

        let owed = debt == ak_host_debt::AK_HOST_MUST_RETURN;
        if owed {
            ledger.drained().await;
        }

        // The rest is a thread's, not a task's. It emits RESOURCES_RELEASED and then shuts tokio
        // down, and QUIESCENT is that thread having finished - so the host reads it when there is
        // nothing left rather than when this task got here.
        if let Some(runtime) = weak.upgrade() {
            runtime.tear_down(owed);
        }
    });
}

pub(crate) fn release_channel(handle: ak_handle) {
    if let Some(found) = tables::channels().get(handle) {
        close_channel(&found);
    }
}

fn close_channel(channel: &Arc<AkChannel>) {
    if !channel.start_closing() {
        return;
    }

    for call in tables::calls().values() {
        if call.belongs_to_channel(channel) {
            call.cancel();
        }
    }
    channel.grpc.close();
    // Whoever brings the count to zero finishes the close, and with no call to wait for that is
    // this thread. It is also this thread when the last call left while the loop above ran - its
    // own attempt found the count still standing - so the call is made unconditionally rather
    // than reasoned about.
    channel.finish_closing();
}

pub(crate) fn call_settled(call: ak_handle) {
    tables::calls().remove(call);
}
