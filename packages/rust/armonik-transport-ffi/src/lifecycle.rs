use std::sync::{Arc, Weak};

use crate::abi::{ak_event_kind, ak_handle, ak_host_debt, ak_runtime_state, ak_status};
use crate::channel::AkChannel;
use crate::host::Host;
use crate::ledger::Ledger;
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
        let failed = Weak::clone(&weak);
        if crate::guarded(shutting_down(weak, host, ledger))
            .await
            .is_none()
        {
            // Nothing else can finish this shutdown: no SHUTDOWN_COMPLETE went out and no
            // teardown thread was started, so the runtime would answer STOPPED for the life of
            // the process and refuse every destroy. That is what AK_RUNTIME_FAILED_UNQUIESCED
            // names - "quiescence impossible, destroy refused".
            if let Some(runtime) = failed.upgrade() {
                runtime.set_state(ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED);
            }
        }
    });
}

async fn shutting_down(weak: Weak<AkRuntime>, host: Arc<Host>, ledger: Arc<Ledger>) {
    let reached = |act: fn(&AkRuntime)| {
        if let Some(runtime) = weak.upgrade() {
            act(&runtime);
        }
    };

    // On the blocking pool rather than on this worker. What it waits for is every host thread to
    // leave `ak_channel_create` and `ak_call_start`, which is short and is not this thread's to
    // predict - and a worker parked on a lock is one not driving the calls that have to reach
    // their terminals before this shutdown can go on.
    if let Some(runtime) = weak.upgrade() {
        tokio::task::spawn_blocking(move || runtime.close_the_gate())
            .await
            .expect("the runtime is running, so its blocking pool takes this");
    }

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

    // Two signals, because the host may still hold payloads and buffers when the gRPC side
    // stops: this one says whether it does, and RESOURCES_RELEASED below says it has given
    // them all back. Only then is the runtime quiescent and `ak_runtime_destroy` accepted.
    //
    // STOPPED once the callback has returned, as level 1's RuntimeRelease has it, so a host
    // that reads the status from inside the callback reads STOPPING. QUIESCENT is derived from
    // STOPPED and the teardown thread's end, so the store comes before `tear_down` below.
    host.signal_runtime(ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE, debt);
    reached(|runtime| runtime.set_state(ak_runtime_state::AK_RUNTIME_GRPC_STOPPED));

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
}

pub(crate) fn release_channel(handle: ak_handle) {
    if let Some(found) = tables::channels().get(handle) {
        let enlisted = found.release();
        close(&found, enlisted);
    }
}

fn close_channel(channel: &Arc<AkChannel>) {
    close(channel, channel.start_closing());
}

/// Cancels the calls a close has to, when this is the close that started it.
fn close(channel: &Arc<AkChannel>, enlisted: Option<Vec<ak_handle>>) {
    if let Some(enlisted) = enlisted {
        for call in enlisted {
            if let Some(call) = tables::calls().get(call) {
                call.cancel();
            }
        }
        channel.grpc.close();
    }
    // Whoever brings the count to zero finishes the close, and with no call to wait for that is
    // this thread. It is also this thread when the last call left while the loop above ran - its
    // own attempt found the count still standing - and when a release finds a channel the
    // shutdown already closed, which only the release can reclaim. So the call is made
    // unconditionally rather than reasoned about.
    channel.finish_closing();
}

pub(crate) fn call_settled(call: ak_handle) {
    tables::calls().remove(call);
}
