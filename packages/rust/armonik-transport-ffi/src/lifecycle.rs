use std::sync::Arc;

use crate::abi::{ak_event_kind, ak_handle, ak_host_debt, ak_runtime_state, ak_status};
use crate::host::Host;
use crate::runtime::AkRuntime;
use crate::tables;

pub(crate) fn create_runtime(
    worker_threads: u32,
    memory_ceiling: u64,
    host: Host,
) -> Result<ak_handle, ak_status> {
    if !AkRuntime::claim() {
        return Err(ak_status::AK_STATUS_INVALID_STATE);
    }
    match AkRuntime::new(worker_threads, memory_ceiling, host) {
        Err(status) => {
            AkRuntime::relinquish();
            Err(status)
        }
        Ok(runtime) => match tables::runtimes().insert(runtime) {
            Some(handle) => Ok(handle),
            None => {
                AkRuntime::relinquish();
                Err(ak_status::AK_STATUS_INTERNAL)
            }
        },
    }
}

pub(crate) fn destroy_runtime(handle: ak_handle) -> ak_status {
    let Some(found) = tables::runtimes().get(handle) else {
        return ak_status::AK_STATUS_HANDLE_STALE;
    };
    if found.state() != ak_runtime_state::AK_RUNTIME_QUIESCENT {
        return ak_status::AK_STATUS_INVALID_STATE;
    }

    tables::calls().drain();
    tables::channels().drain();
    found.release_threads();
    tables::runtimes().remove(handle);
    AkRuntime::relinquish();
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
            release_channel(channel.handle);
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

pub(crate) fn release_channel(handle: ak_handle) {
    let Some(found) = tables::channels().get(handle) else {
        return;
    };
    if !found.start_closing() {
        return;
    }

    for call in tables::calls().values() {
        if call.belongs_to_channel(handle) {
            call.cancel();
        }
    }
    found.grpc.close();
}

pub(crate) fn call_reached_terminal(channel: ak_handle) {
    if let Some(found) = tables::channels().get(channel) {
        found.leave();
    }
}

pub(crate) fn call_settled(call: ak_handle) {
    tables::calls().remove(call);
}
