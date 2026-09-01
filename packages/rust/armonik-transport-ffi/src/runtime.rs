//! The runtime: what owns the Tokio threads, the registries, and the shutdown chain.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use armonik_transport::grpc::GrpcChannel;
use tokio::sync::watch;

use crate::abi::{
    ak_event_kind, ak_handle, ak_host_debt, ak_memory_usage, ak_runtime_state, ak_status,
};
use crate::call::CallState;
use crate::host::{Host, HostPtr};
use crate::tables;

/// What the host holds of a runtime: payloads not consumed, buffers not given back.
///
/// Quiescence is this reaching zero after the runtime has stopped, which is why the host gets
/// there by acting rather than by waiting.
pub(crate) struct Ledger {
    /// Payloads and buffers together: what decides quiescence. A zero-length payload is still
    /// something the host holds, which is why this is a count and not the byte total.
    outstanding: AtomicU64,
    /// Lent buffers only: what the ceiling bounds.
    bytes: AtomicU64,
    ceiling: u64,
    /// Bumped on every release. A version and not a `Notify`: a waiter that checks its condition
    /// before creating the future misses a `notify_waiters` landing in between, and here that
    /// costs the runtime its quiescence forever.
    changed: watch::Sender<u64>,
}

impl Ledger {
    fn new(ceiling: u64) -> Self {
        Self {
            outstanding: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            ceiling,
            changed: watch::channel(0).0,
        }
    }

    pub(crate) fn usage(&self) -> ak_memory_usage {
        ak_memory_usage {
            bytes_used: self.bytes.load(Ordering::Acquire),
            ceiling: self.ceiling,
        }
    }

    pub(crate) fn hold(&self) {
        self.outstanding.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn release(&self) {
        self.outstanding.fetch_sub(1, Ordering::AcqRel);
        self.changed.send_modify(|version| *version += 1);
    }

    /// Whether a request of `len` could ever fit. A refusal here is permanent: no return by
    /// anyone will make room, so retrying is pointless.
    pub(crate) fn could_ever_fit(&self, len: usize) -> Result<(), ak_status> {
        match self.ceiling {
            0 => Ok(()),
            ceiling if len as u64 <= ceiling => Ok(()),
            _ => Err(ak_status::AK_STATUS_MESSAGE_TOO_LARGE),
        }
    }

    /// Takes `len` bytes against the ceiling, or reports that they are not there yet.
    pub(crate) fn reserve(&self, len: usize) -> Result<(), ak_status> {
        if self.ceiling == 0 {
            self.bytes.fetch_add(len as u64, Ordering::AcqRel);
            self.hold();
            return Ok(());
        }

        let mut seen = self.bytes.load(Ordering::Acquire);
        loop {
            let wanted = seen + len as u64;
            if wanted > self.ceiling {
                return Err(ak_status::AK_STATUS_BUDGET_BUSY);
            }
            match self.bytes.compare_exchange_weak(
                seen,
                wanted,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.hold();
                    return Ok(());
                }
                Err(current) => seen = current,
            }
        }
    }

    /// Gives `len` bytes back to the ceiling, and the buffer back to the count.
    pub(crate) fn release_bytes(&self, len: usize) {
        self.bytes.fetch_sub(len as u64, Ordering::AcqRel);
        self.release();
    }

    fn empty(&self) -> bool {
        self.outstanding.load(Ordering::Acquire) == 0
    }

    async fn drained(&self) {
        let mut changed = self.changed.subscribe();
        while !self.empty() {
            if changed.changed().await.is_err() {
                return;
            }
        }
    }
}

/// A channel and the runtime it belongs to.
pub(crate) struct AkChannel {
    pub(crate) grpc: GrpcChannel,
    pub(crate) runtime: Weak<AkRuntime>,
    handle: ak_handle,
}

impl AkChannel {
    pub(crate) fn new(grpc: GrpcChannel, runtime: &Arc<AkRuntime>, handle: ak_handle) -> Self {
        Self {
            grpc,
            runtime: Arc::downgrade(runtime),
            handle,
        }
    }

    fn handle(&self) -> ak_handle {
        self.handle
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
    /// Held for reading while a downcall passes the start gate and publishes what it started, and
    /// for writing by the shutdown before it takes its snapshot. Without it a call can be
    /// published after the runtime has declared its last event.
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
    /// The guard is what makes the check and the publishing that follows it one step: the
    /// shutdown waits for it before deciding what there is to drain.
    pub(crate) fn pass_the_gate(&self) -> Option<std::sync::RwLockReadGuard<'_, ()>> {
        let pass = self
            .gate
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (self.state() == ak_runtime_state::AK_RUNTIME_RUNNING).then_some(pass)
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

    /// Takes this runtime's channels out of the registry and hands them over.
    fn take_own_channels(this: &Weak<Self>) -> Vec<Arc<AkChannel>> {
        tables::channels()
            .values()
            .into_iter()
            .filter(|channel| Weak::ptr_eq(&channel.runtime, this))
            .inspect(|channel| {
                tables::channels().remove(channel.handle());
            })
            .collect()
    }

    /// Stales every handle this runtime owns.
    pub(crate) fn stale_own_handles(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        for call in Self::own_calls(&weak) {
            tables::calls().remove(call.handle());
        }
        Self::take_own_channels(&weak);
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
            // Waits out any downcall that passed the gate and has not published yet, so the
            // snapshots below cannot miss a call that is about to exist. Nothing is awaited while
            // it is held.
            if let Some(runtime) = weak.upgrade() {
                drop(
                    runtime
                        .gate
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
            }

            // Closing a channel cancels its calls, which is what makes them reach a terminal.
            // The call registry is not drained: a handle stays valid until its call is reclaimed,
            // and reclamation is what empties it.
            for channel in Self::take_own_channels(&weak) {
                channel.grpc.close();
            }
            let calls = Self::own_calls(&weak);
            for call in &calls {
                call.cancel();
            }
            for call in &calls {
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

    /// Gives the threads up, waiting for them.
    ///
    /// Only reached from quiescence, so nothing should still be running - but returning while a
    /// detached thread is inside a host callback is what would make unloading the library unsafe,
    /// and that is the one thing this call is supposed to permit. The bound keeps a task that
    /// refuses to end from hanging the downcall for good.
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
