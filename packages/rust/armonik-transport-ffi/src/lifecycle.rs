use std::sync::{Arc, Weak};

use armonik_transport::configuration::Configuration;
use armonik_transport::options::RuntimeOptions;

use crate::abi::{ak_event_kind, ak_handle, ak_host_debt, ak_runtime_state, ak_status};
use crate::channel::AkChannel;
use crate::host::Host;
use crate::log::{self, Sink};
use crate::refusal::Refusal;
use crate::runtime::{AkRuntime, Claim};
use crate::tables;

pub(crate) fn create_runtime(
    memory_ceiling: u64,
    memory_hard_ceiling: u64,
    channel_defaults: &[u8],
    sink: Option<Sink>,
    host: Host,
) -> Result<ak_handle, Refusal> {
    let claim = Claim::take().ok_or(ak_status::AK_STATUS_INVALID_STATE)?;
    // After the claim, so a second runtime is refused as one whatever its defaults say, and
    // logs nothing to the callback of the one that lives.
    let loading = log::begin(sink);
    let mut options = RuntimeOptions::default();
    let defaults = crate::config::defaults(channel_defaults);
    loading.settle(None);
    options.channel_defaults = defaults.map_err(Refusal::config)?;
    start(claim, options, (memory_ceiling, memory_hard_ceiling), host)
}

pub(crate) fn create_runtime_from(
    configuration: &Configuration,
    sink: Option<Sink>,
    host: Host,
) -> Result<ak_handle, Refusal> {
    let claim = Claim::take().ok_or(ak_status::AK_STATUS_INVALID_STATE)?;
    // After the claim, as ak_runtime_create reads its defaults after it: a second runtime is
    // refused as one, and its sources are not read.
    let loading = log::begin(sink);
    let loaded = crate::config::runtime(configuration);
    // What a refused load logged is delivered too, by the default filter.
    loading.settle(
        loaded
            .as_ref()
            .ok()
            .and_then(|options| options.logging.filter.as_deref()),
    );
    let options = loaded.map_err(Refusal::config)?;
    let ceilings = options.memory_ceiling.bytes();
    start(claim, options, ceilings, host)
}

/// Starts a runtime whose ceilings, soft then hard, are `ceilings` in bytes, 0 for the library's
/// own.
fn start(
    claim: Claim,
    options: RuntimeOptions,
    ceilings: (u64, u64),
    host: Host,
) -> Result<ak_handle, Refusal> {
    let runtime = AkRuntime::new(options, ceilings, host)?;
    crate::config::log_runtime(runtime.options(), ceilings);
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

    runtime.spawner().spawn(async move {
        let failed = Weak::clone(&weak);
        if crate::guarded(shutting_down(weak, host)).await.is_none() {
            // Before GRPC_STOPPED nothing else can finish this shutdown, so the runtime would
            // answer STOPPING for the life of the process and refuse every destroy. That is what
            // AK_RUNTIME_FAILED_UNQUIESCED names - "quiescence impossible, destroy refused". From
            // GRPC_STOPPED on, the teardown thread finishes it, and the state stays.
            if let Some(runtime) = failed.upgrade() {
                runtime.fail_while_stopping();
            }
        }
    });
}

async fn shutting_down(weak: Weak<AkRuntime>, host: Arc<Host>) {
    let reached = |act: fn(&AkRuntime)| {
        if let Some(runtime) = weak.upgrade() {
            act(&runtime);
        }
    };

    // On the blocking pool rather than on this worker. What it waits for is every host thread to
    // leave `ak_channel_create` and `ak_call_start`, which is short but not this thread's to
    // predict, and a tokio worker does not block.
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

    let Some(runtime) = weak.upgrade() else {
        return;
    };
    let debt = if runtime.ledger().empty() {
        ak_host_debt::AK_HOST_NOTHING_TO_RETURN
    } else {
        ak_host_debt::AK_HOST_MUST_RETURN
    };

    // The rest is a thread's, not a task's: it waits for the host to give back what it owes,
    // emits RESOURCES_RELEASED, and shuts tokio down, and QUIESCENT is that thread having
    // finished. Obtained while the runtime is still stopping, where a runtime that cannot get one
    // fails: GRPC_STOPPED is level 0's RELEASED, which no failure leaves.
    let owed = debt == ak_host_debt::AK_HOST_MUST_RETURN;
    let Some(teardown) = runtime.prepare_teardown(owed) else {
        runtime.fail_while_stopping();
        return;
    };
    drop(runtime);

    // Two signals, because the host may still hold payloads and buffers when the gRPC side
    // stops: this one says whether it does, and RESOURCES_RELEASED says it has given them all
    // back. Only then is the runtime quiescent and `ak_runtime_destroy` accepted.
    //
    // STOPPED once the callback has returned, as level 1's RuntimeRelease has it, so a host
    // that reads the status from inside the callback reads STOPPING. QUIESCENT is derived from
    // STOPPED and the teardown thread's end, so the store comes before the thread goes on; a
    // panic past the store lets it go all the same.
    host.signal_runtime(ak_event_kind::AK_EVENT_SHUTDOWN_COMPLETE, debt);
    reached(|runtime| runtime.set_state(ak_runtime_state::AK_RUNTIME_GRPC_STOPPED));
    #[cfg(feature = "test-hooks")]
    crate::hooks::run_after_stopped();
    teardown.start();
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
        // First, so the watcher ends whatever the rest does: the calls it watches are cancelled
        // below, and a call that starts now is refused its listing and cancels itself.
        channel.deadlines.close();
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
