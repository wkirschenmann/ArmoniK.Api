//! The sequences that span more than one kind of object.
//!
//! What is stated here is an order, and an order is not a fact about any one of the objects it
//! reaches: a channel closing has to latch before it cancels its calls, and a call settling has
//! to be able to finish the channel it was the last of. Each object refers to its peers by handle
//! rather than by reference, so the modules could see one another - what they could not do is
//! hold half of a rule neither can state alone, and a reader of `call` would have no way to find
//! out that a settling call may close a channel.
//!
//! Every chain that reaches two kinds is here, so "who reaches whom, in what order" has one file
//! to read: creating and destroying a runtime, stopping one, closing a channel, settling a call.

use std::sync::Arc;

use crate::abi::{ak_event_kind, ak_handle, ak_host_debt, ak_runtime_state, ak_status};
use crate::host::Host;
use crate::runtime::AkRuntime;
use crate::tables;

/// Creates the one runtime the model admits, and publishes it.
pub(crate) fn create_runtime(
    worker_threads: u32,
    memory_ceiling: u64,
    host: Host,
) -> Result<ak_handle, ak_status> {
    if !AkRuntime::claim() {
        return Err(ak_status::AK_STATUS_INVALID_STATE);
    }
    match AkRuntime::new(worker_threads, memory_ceiling, host) {
        // The claim goes back on the way out: a build that failed left no generation behind, and
        // a host that retries has to be able to.
        Err(status) => {
            AkRuntime::relinquish();
            Err(status)
        }
        Ok(runtime) => Ok(tables::runtimes().insert(runtime)),
    }
}

/// Frees a quiescent runtime: stales every handle it owns, gives its threads up, and lets a fresh
/// generation follow. Refused before quiescence, and that is the only reason.
pub(crate) fn destroy_runtime(handle: ak_handle) -> ak_status {
    let Some(found) = tables::runtimes().get(handle) else {
        return ak_status::AK_STATUS_HANDLE_STALE;
    };
    if found.state() != ak_runtime_state::AK_RUNTIME_QUIESCENT {
        return ak_status::AK_STATUS_INVALID_STATE;
    }

    // One generation exists at a time, so every entry in the two tables is this runtime's.
    tables::calls().drain();
    tables::channels().drain();
    found.release_threads();
    tables::runtimes().remove(handle);
    AkRuntime::relinquish();
    ak_status::AK_STATUS_OK
}

/// Closes the start gate and drains, on the runtime's own threads. Idempotent: a second call is
/// a no-op.
pub(crate) fn begin_shutdown(runtime: &Arc<AkRuntime>) {
    if !runtime.start_stopping() {
        return;
    }

    // The task holds no strong reference: the last one must be free to go on a host thread,
    // where giving the Tokio runtime up is legal.
    let weak = Arc::downgrade(runtime);
    let host = Arc::clone(runtime.host());
    let ledger = Arc::clone(runtime.ledger());

    runtime.spawner().spawn(async move {
        // Every reach back into the runtime is conditional: the host may destroy it, and the
        // last strong reference must be free to go on a host thread.
        let reached = |act: fn(&AkRuntime)| {
            if let Some(runtime) = weak.upgrade() {
                act(&runtime);
            }
        };

        reached(AkRuntime::close_the_gate);

        // Through the same sequence a host release goes through, so a channel closed by a
        // shutdown reaches CLOSED the way one closed by hand does. The handle stays: the header
        // promises a close does not stale it, `settle_channel` needs to find the channel again
        // when its last call is reclaimed, and the destroy is what reclaims every handle.
        for channel in tables::channels().values() {
            release_channel(channel.handle);
        }
        // A call of a channel the host had already released is reached by nothing above.
        let calls = tables::calls().values();
        for call in &calls {
            call.cancel();
        }
        for call in &calls {
            call.finished().await;
        }

        // Every call has delivered its terminal and every callback has returned; what
        // may be left is what the host holds.
        let debt = if ledger.empty() {
            ak_host_debt::AK_HOST_NOTHING_TO_RETURN
        } else {
            ak_host_debt::AK_HOST_MUST_RETURN
        };

        host.signal_runtime(ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE, debt);
        reached(|runtime| runtime.set_state(ak_runtime_state::AK_RUNTIME_GRPC_STOPPED));

        if debt == ak_host_debt::AK_HOST_MUST_RETURN {
            ledger.drained().await;
            host.signal_runtime(
                ak_event_kind::AK_EVENT_RESOURCES_RELEASED,
                ak_host_debt::AK_HOST_NOTHING_TO_RETURN,
            );
        }
        reached(|runtime| runtime.set_state(ak_runtime_state::AK_RUNTIME_QUIESCENT));
    });
}

/// Closes a channel and cancels its calls, in that order and only once.
///
/// The cancellation is not a courtesy: the channel is closing from this moment and no closing
/// channel may have an active call, so the latch is what makes the drain this library's own
/// business rather than something the host must provoke. A call parked on a delivery credit is
/// the case that needs it - it is not watching the transport, so closing the session alone would
/// never reach it.
pub(crate) fn release_channel(handle: ak_handle) {
    let Some(found) = tables::channels().get(handle) else {
        return;
    };
    if !found.start_closing() {
        return;
    }

    // Every call that had joined by the time the latch took: one that had not cannot join now,
    // and one that is joining has already taken its place, so `start_closing` left the channel
    // CLOSING rather than CLOSED and this snapshot is taken after the publish that follows.
    for call in tables::calls().values() {
        if call.belongs_to_channel(handle) {
            call.cancel();
        }
    }
    found.grpc.close();
}

/// A call has reached its terminal, so its channel may have nothing active left.
///
/// Separate from the reclamation below, and this is the one the channel's state turns on: a call
/// past its terminal is no longer active even though it stays in the table until the host gives
/// back what it holds. Closing on reclamation instead would make CLOSED wait on the host, and a
/// runtime could then announce it had stopped with a channel still CLOSING - which is neither
/// what the header promises nor what the model admits of a drained runtime.
pub(crate) fn call_reached_terminal(channel: ak_handle) {
    if let Some(found) = tables::channels().get(channel) {
        found.leave();
    }
}

/// Takes a settled call's handle back.
///
/// It asks nothing of the channel: what a channel's closing turns on is the count of its calls
/// short of their terminal, which [`call_reached_terminal`] already decremented, and nothing
/// between there and here changes it. A channel that became closable did so then; one that
/// latched CLOSING in between was settled by the release that latched it.
pub(crate) fn call_settled(call: ak_handle) {
    tables::calls().remove(call);
}
