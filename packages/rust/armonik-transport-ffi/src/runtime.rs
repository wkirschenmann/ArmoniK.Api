//! The runtime: the threads, the start gate, and the state the host polls.
//!
//! One generation at a time, which is the model's own assumption and what every promise about
//! "the runtime" is stated over. What belongs here is what nothing else can see: the Tokio
//! runtime, the gate a downcall passes, and the word that says how far along the stop is. The
//! ordered chains that reach channels and calls are in `lifecycle`.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard};

use crate::abi::{ak_runtime_state, ak_status};
use crate::call::CallServices;
use crate::host::Host;
use crate::ledger::Ledger;

/// Whether a generation exists. Claimed by a create and given back by a destroy, so a fresh one
/// may follow the one before it.
static LIVE: AtomicBool = AtomicBool::new(false);

/// One runtime: its threads, its ledger, and the state the host polls.
pub(crate) struct AkRuntime {
    /// Taken by the destroy. Behind a lock rather than owned outright so dropping the last
    /// handle never has to drop a Tokio runtime from one of its own threads.
    tokio: Mutex<Option<tokio::runtime::Runtime>>,
    spawner: tokio::runtime::Handle,
    host: Arc<Host>,
    ledger: Arc<Ledger>,
    state: AtomicI32,
    /// Held for reading while a downcall passes the start gate and publishes what it
    /// started, and for writing by the shutdown before it takes its snapshot. Without it a
    /// call can be published after the runtime has declared its last event.
    gate: RwLock<()>,
}

impl AkRuntime {
    /// Claims the one generation the model admits (L0!SingleRuntime), or reports it taken.
    ///
    /// A second runtime is refused rather than admitted into a state space nothing was proved
    /// over. Claiming is separate from building because a build that fails must give the claim
    /// back, and only the sequence that took it knows that.
    pub(crate) fn claim() -> bool {
        LIVE.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Gives the generation back, so a fresh one may be created.
    pub(crate) fn relinquish() {
        LIVE.store(false, Ordering::Release);
    }

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
            gate: RwLock::new(()),
        }))
    }

    pub(crate) fn spawner(&self) -> &tokio::runtime::Handle {
        &self.spawner
    }

    pub(crate) fn host(&self) -> &Arc<Host> {
        &self.host
    }

    pub(crate) fn ledger(&self) -> &Arc<Ledger> {
        &self.ledger
    }

    /// What a call needs of this runtime, in the shape the call's own module asks for.
    pub(crate) fn services(&self) -> CallServices<'_> {
        CallServices {
            host: &self.host,
            ledger: &self.ledger,
            spawner: &self.spawner,
        }
    }

    pub(crate) fn state(&self) -> ak_runtime_state {
        ak_runtime_state::from_repr(self.state.load(Ordering::Acquire))
            .unwrap_or(ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED)
    }

    pub(crate) fn set_state(&self, state: ak_runtime_state) {
        self.state.store(state as i32, Ordering::Release);
    }

    /// Latches the runtime to stopping. Answers false if it was already, so a second shutdown
    /// drains nothing twice.
    ///
    /// The latch and the state are one word, so there is no moment in which a shutdown has been
    /// decided and the gate still reads as open.
    pub(crate) fn start_stopping(&self) -> bool {
        self.state
            .compare_exchange(
                ak_runtime_state::AK_RUNTIME_RUNNING as i32,
                ak_runtime_state::AK_RUNTIME_GRPC_STOPPING as i32,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// The start gate, open only while the runtime is running.
    ///
    /// The guard is what makes the check and the publishing that follows it one step.
    pub(crate) fn pass_the_gate(&self) -> Option<RwLockReadGuard<'_, ()>> {
        let pass = self.gate.read().unwrap_or_else(PoisonError::into_inner);
        (self.state() == ak_runtime_state::AK_RUNTIME_RUNNING).then_some(pass)
    }

    /// Waits out any downcall that passed the gate and has not published yet, so a snapshot
    /// taken after this cannot miss a call that is about to exist.
    pub(crate) fn close_the_gate(&self) {
        drop(self.gate.write().unwrap_or_else(PoisonError::into_inner));
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
