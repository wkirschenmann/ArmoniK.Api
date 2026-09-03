//! The runtime: the threads, the start gate, and the shutdown chain.
//!
//! One generation at a time, which is the model's own assumption and what every promise about
//! "the runtime" is stated over. It owns a ledger and it closes its channels, but it defines
//! neither: what belongs here is the lifecycle nothing else can see.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::abi::{ak_event_kind, ak_host_debt, ak_runtime_state, ak_status};
use crate::call::CallState;
use crate::channel::AkChannel;
use crate::host::{Host, HostPtr};
use crate::ledger::Ledger;
use crate::tables;

/// One runtime: its threads, its objects, and the state the host polls.
pub(crate) struct AkRuntime {
    /// Taken by `ak_runtime_destroy`. Behind a lock rather than owned outright so dropping
    /// the last handle never has to drop a Tokio runtime from one of its own threads.
    tokio: Mutex<Option<tokio::runtime::Runtime>>,
    spawner: tokio::runtime::Handle,
    pub(crate) host: Arc<Host>,
    pub(crate) ledger: Arc<Ledger>,
    state: AtomicI32,
    /// Raised by the first `begin_shutdown`, so a second is a no-op.
    stopping: AtomicBool,
    /// Held for reading while a downcall passes the start gate and publishes what it
    /// started, and for writing by the shutdown before it takes its snapshot. Without it a
    /// call can be published after the runtime has declared its last event.
    gate: std::sync::RwLock<()>,
}

impl AkRuntime {
    pub(crate) fn new(
        worker_threads: u32,
        memory_ceiling: u64,
        host: Host,
    ) -> Result<Arc<Self>, ak_status> {
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
            ledger: Arc::new(Ledger::new(memory_ceiling)),
            state: AtomicI32::new(ak_runtime_state::AK_RUNTIME_RUNNING as i32),
            stopping: AtomicBool::new(false),
            gate: std::sync::RwLock::new(()),
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

    /// The start gate, open only while the runtime is running.
    ///
    /// The guard is what makes the check and the publishing that follows it one step.
    pub(crate) fn pass_the_gate(&self) -> Option<std::sync::RwLockReadGuard<'_, ()>> {
        let pass = self
            .gate
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (self.state() == ak_runtime_state::AK_RUNTIME_RUNNING).then_some(pass)
    }

    /// Every call there is, which is every call of this runtime: one generation exists at a
    /// time, so a filter by owner would be filtering a set it already holds whole.
    fn own_calls() -> Vec<Arc<CallState>> {
        tables::calls().values()
    }

    /// The same, taken out of the table rather than read: a channel closed by a shutdown is not
    /// one a later downcall may name.
    fn take_own_channels() -> Vec<Arc<AkChannel>> {
        tables::channels()
            .values()
            .into_iter()
            .filter_map(|channel| tables::channels().remove(channel.handle))
            .collect()
    }

    /// Stales every handle this runtime owns.
    pub(crate) fn stale_own_handles(&self) {
        for call in Self::own_calls() {
            tables::calls().remove(call.handle());
        }
        Self::take_own_channels();
    }

    /// Closes the start gate and drains. Idempotent: a second call is a no-op.
    pub(crate) fn begin_shutdown(self: &Arc<Self>) {
        if self.stopping.swap(true, Ordering::AcqRel) {
            return;
        }
        self.set_state(ak_runtime_state::AK_RUNTIME_GRPC_STOPPING);

        // The task holds no strong reference: the last one must be free to go on a host
        // thread, where giving the Tokio runtime up is legal.
        let weak = Arc::downgrade(self);
        let host = Arc::clone(&self.host);
        let ledger = Arc::clone(&self.ledger);

        self.spawner.spawn(async move {
            // Waits out any downcall that passed the gate and has not published yet, so the
            // snapshots below cannot miss a call that is about to exist. Nothing is awaited
            // while it is held.
            if let Some(runtime) = weak.upgrade() {
                drop(
                    runtime
                        .gate
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
            }

            for channel in Self::take_own_channels() {
                channel.grpc.close();
            }
            let calls = Self::own_calls();
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

    /// Gives the threads up, waiting for them.
    ///
    /// Only reached from quiescence, so nothing should still be running - but returning while a
    /// detached thread is inside a host callback is what would make unloading the library unsafe,
    /// and that is the one thing this call is supposed to permit.
    pub(crate) fn release_threads(&self) {
        let taken = self
            .tokio
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(tokio) = taken {
            tokio.shutdown_timeout(std::time::Duration::from_secs(5));
        }
    }
}
