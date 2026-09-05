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

/// The process-wide claim on being the one runtime.
///
/// A guard rather than a pair of calls, because what happens between taking it and having a
/// runtime to attach it to is not all under this crate's control: `AkRuntime::new` builds a tokio
/// runtime, and tokio panics rather than answering when the OS refuses a worker thread. A claim
/// taken by hand would survive that unwind still taken, and no runtime could be created again for
/// the life of the process.
pub(crate) struct Claim;

impl Claim {
    pub(crate) fn take() -> Option<Self> {
        LIVE.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            .then_some(Claim)
    }

    /// Handed to the runtime that is now in the table; `ak_runtime_destroy` gives it back.
    pub(crate) fn keep(self) {
        std::mem::forget(self);
    }

    /// The claim a caller already holds, as a guard.
    ///
    /// Not `take`: `ak_runtime_destroy` is reached through a live runtime, so the claim is held
    /// and there is nothing to acquire - only something to be sure of giving back.
    pub(crate) fn held() -> Self {
        Claim
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        AkRuntime::relinquish();
    }
}

impl AkRuntime {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing else in this crate's unit tests touches `LIVE`, so these run without a lock of
    /// their own; an integration test would need `ak_runtime_destroy` to give the claim back.
    #[test]
    fn a_claim_not_kept_is_given_back_however_the_holder_leaves() {
        let claim = Claim::take().expect("nothing holds it");
        assert!(Claim::take().is_none(), "one runtime at a time");
        drop(claim);

        let again = Claim::take().expect("the drop gave it back");
        again.keep();
        assert!(Claim::take().is_none(), "a kept claim is held");

        AkRuntime::relinquish();
        Claim::take().expect("relinquish gives back what keep held");
        AkRuntime::relinquish();
    }
}
