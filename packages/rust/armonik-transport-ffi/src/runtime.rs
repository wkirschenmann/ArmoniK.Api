use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard};

use crate::abi::{ak_runtime_state, ak_status};
use crate::call::CallServices;
use crate::host::Host;
use crate::ledger::Ledger;

static LIVE: AtomicBool = AtomicBool::new(false);

pub(crate) struct AkRuntime {
    tokio: Mutex<Option<tokio::runtime::Runtime>>,
    spawner: tokio::runtime::Handle,
    host: Arc<Host>,
    ledger: Arc<Ledger>,
    state: AtomicI32,
    gate: RwLock<()>,
}

impl AkRuntime {
    pub(crate) fn claim() -> bool {
        LIVE.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

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

    /// A pass that lasts as long as the caller holds it, which is what makes the shutdown below
    /// wait rather than race: the state is read under the read lock, so a shutdown that has not
    /// taken the write lock yet cannot have changed it.
    pub(crate) fn pass_the_gate(&self) -> Option<RwLockReadGuard<'_, ()>> {
        let pass = self.gate.read().unwrap_or_else(PoisonError::into_inner);
        (self.state() == ak_runtime_state::AK_RUNTIME_RUNNING).then_some(pass)
    }

    /// Taken and dropped for the wait alone: once this returns, every pass handed out before the
    /// state changed has been given back.
    pub(crate) fn close_the_gate(&self) {
        drop(self.gate.write().unwrap_or_else(PoisonError::into_inner));
    }

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
