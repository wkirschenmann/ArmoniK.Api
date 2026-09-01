//! The runtime: what owns the Tokio threads, the registries, and the shutdown chain.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use armonik_transport::grpc::GrpcChannel;
use tokio::sync::Notify;

use crate::abi::{ak_event_kind, ak_handle, ak_host_debt, ak_runtime_state, ak_status};
use crate::call::CallState;
use crate::host::{Host, HostPtr};
use crate::tables;

/// What the host holds of a runtime: payloads not consumed, buffers not given back.
///
/// Quiescence is this reaching zero after the runtime has stopped, which is why the host gets
/// there by acting rather than by waiting.
#[derive(Default)]
pub(crate) struct Ledger {
    outstanding: AtomicU64,
    changed: Notify,
}

impl Ledger {
    pub(crate) fn hold(&self) {
        self.outstanding.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn release(&self) {
        self.outstanding.fetch_sub(1, Ordering::AcqRel);
        self.changed.notify_waiters();
    }

    fn empty(&self) -> bool {
        self.outstanding.load(Ordering::Acquire) == 0
    }

    async fn drained(&self) {
        while !self.empty() {
            self.changed.notified().await;
        }
    }
}

/// A channel and the runtime it belongs to.
pub(crate) struct AkChannel {
    pub(crate) grpc: GrpcChannel,
    pub(crate) runtime: Weak<AkRuntime>,
    handle: OnceLock<ak_handle>,
}

impl AkChannel {
    pub(crate) fn new(grpc: GrpcChannel, runtime: &Arc<AkRuntime>) -> Self {
        Self {
            grpc,
            runtime: Arc::downgrade(runtime),
            handle: OnceLock::new(),
        }
    }

    pub(crate) fn name_it(&self, handle: ak_handle) {
        let _ = self.handle.set(handle);
    }

    fn handle(&self) -> ak_handle {
        self.handle.get().copied().unwrap_or_default()
    }
}

/// One runtime: its threads, its objects, and the state the host polls.
pub(crate) struct AkRuntime {
    /// Taken by `ak_runtime_destroy`, which is what gives the threads up. Behind a lock rather
    /// than owned outright so that dropping the last handle never has to drop a Tokio runtime
    /// from one of its own threads, which panics.
    tokio: Mutex<Option<tokio::runtime::Runtime>>,
    spawner: tokio::runtime::Handle,
    pub(crate) host: Arc<Host>,
    pub(crate) ledger: Arc<Ledger>,
    state: AtomicI32,
    /// Raised by the first `begin_shutdown`, so a second is a no-op.
    stopping: AtomicBool,
}

impl AkRuntime {
    pub(crate) fn new(worker_threads: u32, host: Host) -> Result<Arc<Self>, ak_status> {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.enable_all();
        if worker_threads > 0 {
            builder.worker_threads(worker_threads as usize);
        }
        let tokio = builder.build().map_err(|_| ak_status::AK_STATUS_INTERNAL)?;
        let spawner = tokio.handle().clone();

        Ok(Arc::new(Self {
            tokio: Mutex::new(Some(tokio)),
            spawner,
            host: Arc::new(host),
            ledger: Arc::default(),
            state: AtomicI32::new(ak_runtime_state::AK_RUNTIME_RUNNING as i32),
            stopping: AtomicBool::new(false),
        }))
    }

    pub(crate) fn spawner(&self) -> &tokio::runtime::Handle {
        &self.spawner
    }

    pub(crate) fn state(&self) -> ak_runtime_state {
        match self.state.load(Ordering::Acquire) {
            1 => ak_runtime_state::AK_RUNTIME_RUNNING,
            2 => ak_runtime_state::AK_RUNTIME_GRPC_STOPPING,
            3 => ak_runtime_state::AK_RUNTIME_GRPC_STOPPED,
            4 => ak_runtime_state::AK_RUNTIME_QUIESCENT,
            _ => ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED,
        }
    }

    fn set_state(&self, state: ak_runtime_state) {
        self.state.store(state as i32, Ordering::Release);
    }

    /// Whether the start gate is open: channels and calls begin only on a running runtime.
    pub(crate) fn accepts_new_work(&self) -> bool {
        self.state() == ak_runtime_state::AK_RUNTIME_RUNNING
    }

    /// Takes a call's handle back, now that nothing of it is outstanding.
    pub(crate) fn reclaim_call(&self, handle: ak_handle) {
        tables::calls().remove(handle);
    }

    /// The calls of this runtime, as a snapshot.
    fn own_calls(this: &Weak<Self>) -> Vec<Arc<CallState>> {
        tables::calls()
            .values()
            .into_iter()
            .filter(|call| call.belongs_to(this))
            .collect()
    }

    /// Stales every handle this runtime owns, and hands back what they named.
    pub(crate) fn stale_own_handles(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        for call in Self::own_calls(&weak) {
            tables::calls().remove(call.handle());
        }
        for channel in tables::channels().values() {
            if Weak::ptr_eq(&channel.runtime, &weak) {
                tables::channels().remove(channel.handle());
            }
        }
    }

    /// Closes the start gate and drains. Idempotent: a second call is a no-op.
    pub(crate) fn begin_shutdown(self: &Arc<Self>) {
        if self.stopping.swap(true, Ordering::AcqRel) {
            return;
        }
        self.set_state(ak_runtime_state::AK_RUNTIME_GRPC_STOPPING);

        // The task holds no strong reference: the last one must be free to go on a host thread,
        // where giving the Tokio runtime up is legal.
        let weak = Arc::downgrade(self);
        let host = Arc::clone(&self.host);
        let ledger = Arc::clone(&self.ledger);

        self.spawner.spawn(async move {
            // Closing a channel cancels its calls, which is what makes them reach a terminal.
            // The call registry is not drained: a handle stays valid until its call is reclaimed,
            // and reclamation is what empties it.
            for channel in tables::channels().values() {
                if Weak::ptr_eq(&channel.runtime, &weak) {
                    tables::channels().remove(channel.handle());
                    channel.grpc.close();
                }
            }
            for call in Self::own_calls(&weak) {
                call.cancel();
            }
            for call in Self::own_calls(&weak) {
                call.finished().await;
            }

            // Every call has delivered its terminal and every callback has returned; what may be
            // left is what the host holds.
            let debt = if ledger.empty() {
                ak_host_debt::AK_HOST_NOTHING_TO_RETURN
            } else {
                ak_host_debt::AK_HOST_MUST_RETURN
            };

            host.signal_shutdown(debt);
            if let Some(runtime) = weak.upgrade() {
                runtime.set_state(ak_runtime_state::AK_RUNTIME_GRPC_STOPPED);
            }

            if debt == ak_host_debt::AK_HOST_MUST_RETURN {
                ledger.drained().await;
                host.signal(HostPtr::null(), ak_event_kind::AK_EVENT_RESOURCES_RELEASED);
            }
            if let Some(runtime) = weak.upgrade() {
                runtime.set_state(ak_runtime_state::AK_RUNTIME_QUIESCENT);
            }
        });
    }

    /// Gives the threads up. Only reached once nothing of the runtime is outstanding, so no task
    /// is left to be cut short.
    pub(crate) fn release_threads(&self) {
        let taken = self
            .tokio
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(tokio) = taken {
            tokio.shutdown_background();
        }
    }
}
