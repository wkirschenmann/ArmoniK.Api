use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard};
use std::time::Duration;

use tokio::sync::{oneshot, watch};

use crate::abi::{ak_error_kind, ak_event_kind, ak_host_debt, ak_runtime_state, ak_status};
use crate::call::CallServices;
use crate::held::Held;
use crate::host::Host;
use crate::ledger::Ledger;
use crate::refusal::Refusal;

static LIVE: AtomicBool = AtomicBool::new(false);

pub(crate) struct AkRuntime {
    tokio: Mutex<Option<tokio::runtime::Runtime>>,
    spawner: tokio::runtime::Handle,
    host: Arc<Host>,
    ledger: Arc<Ledger>,
    state: AtomicI32,
    gate: RwLock<()>,
    /// The thread that finishes the shutdown, once there is one.
    ///
    /// Not a tokio task: it emits the last event and then shuts tokio down, which tokio refuses
    /// from inside itself. Its having finished is what QUIESCENT means - see `state`.
    teardown: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The channels' threads, each a current-thread runtime that runs its channel's connection
    /// and calls, so that a call never moves between threads. Joined by the teardown, which is
    /// what keeps QUIESCENT meaning that no thread of this runtime is left.
    channel_threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
    /// Raised by the teardown, which every channel's thread stops on.
    stop_channels: watch::Sender<bool>,
    /// Numbers the channels' threads, so that each is told apart by its name.
    channels_started: AtomicU64,
}

/// A channel's thread, from the channel's side: where its work runs, and what stops it when the
/// channel goes.
pub(crate) struct ChannelThread {
    pub(crate) spawner: tokio::runtime::Handle,
    pub(crate) stop: oneshot::Sender<()>,
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

// The OS's reason is not kept: whichever it is, the host's remedy is fewer channels.
const CHANNEL_THREAD_REFUSED: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INTERNAL,
    ak_error_kind::AK_ERROR_NONE,
    "too many channels",
);

/// How long a channel's thread waits, once its channel is gone, for the channel's leftover work.
const CHANNEL_WIND_DOWN: Duration = Duration::from_secs(1);

const THRESHOLDS_CROSSED: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "memory_hard_ceiling is below the first threshold in force: memory_ceiling, or this library's own when that is zero",
);

impl AkRuntime {
    pub(crate) fn relinquish() {
        LIVE.store(false, Ordering::Release);
    }

    pub(crate) fn new(
        memory_ceiling: u64,
        memory_hard_ceiling: u64,
        host: Host,
    ) -> Result<Arc<Self>, Refusal> {
        let ledger =
            Ledger::new(memory_ceiling, memory_hard_ceiling).map_err(|_| THRESHOLDS_CROSSED)?;

        // One worker: what runs here is the shutdown's orchestration, the channels' work running
        // on threads of their own.
        let tokio = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("armonik-runtime")
            .enable_all()
            .build()
            .map_err(|_| ak_status::AK_STATUS_INTERNAL)?;
        let spawner = tokio.handle().clone();

        Ok(Arc::new(Self {
            tokio: Mutex::new(Some(tokio)),
            spawner,
            host: Arc::new(host),
            ledger: Arc::new(ledger),
            state: AtomicI32::new(ak_runtime_state::AK_RUNTIME_RUNNING as i32),
            gate: RwLock::new(()),
            teardown: Mutex::new(None),
            channel_threads: Mutex::new(Vec::new()),
            stop_channels: watch::channel(false).0,
            channels_started: AtomicU64::new(0),
        }))
    }

    /// Starts a channel's thread: a current-thread runtime that runs until the channel drops the
    /// `stop` it is handed, or the teardown stops every channel.
    ///
    /// Stopped either way, the thread first gives what the channel left running - the close of
    /// its connection, a cancelled call's cleanup - up to `CHANNEL_WIND_DOWN` to finish, so that
    /// a peer sees the connection closed rather than dropped. The teardown comes after the
    /// shutdown has closed every channel, so its connections are closing too.
    pub(crate) fn start_channel_thread(&self) -> Result<ChannelThread, Refusal> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let mut all_stop = self.stop_channels.subscribe();
        let thread = std::thread::Builder::new()
            .name(format!(
                "armonik-channel-{}",
                self.channels_started.fetch_add(1, Ordering::Relaxed)
            ))
            .spawn(move || {
                #[cfg(feature = "test-hooks")]
                let _alive = crate::hooks::ChannelThreadAlive::new();
                let tokio = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(tokio) => tokio,
                    Err(_) => return,
                };
                let (stop, stopped) = oneshot::channel();
                if ready_tx.send((tokio.handle().clone(), stop)).is_err() {
                    return;
                }
                tokio.block_on(async move {
                    tokio::select! {
                        _ = stopped => {}
                        _ = all_stop.wait_for(|stopping| *stopping) => {}
                    }
                    // Polled, because tokio offers no wait on a runtime's tasks.
                    let left = tokio::runtime::Handle::current();
                    let _ = tokio::time::timeout(CHANNEL_WIND_DOWN, async {
                        while left.metrics().num_alive_tasks() > 0 {
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                    })
                    .await;
                });
            })
            .map_err(|_| CHANNEL_THREAD_REFUSED)?;
        let Ok((spawner, stop)) = ready_rx.recv() else {
            let _ = thread.join();
            return Err(CHANNEL_THREAD_REFUSED);
        };

        let mut threads = Held::new(
            self.channel_threads
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        // The threads of channels already gone, reaped as new ones start, so that the handles
        // kept are bounded by the channels open at the last start rather than by every channel
        // the process ever had.
        let (finished, running): (Vec<_>, Vec<_>) = threads
            .drain(..)
            .partition(std::thread::JoinHandle::is_finished);
        for done in finished {
            let _ = done.join();
        }
        *threads = running;
        threads.push(thread);
        Ok(ChannelThread { spawner, stop })
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
        }
    }

    /// What the host reads from `ak_runtime_status`.
    ///
    /// QUIESCENT is not stored anywhere: it is the teardown thread having finished. Asked of the
    /// thread, it says the last event has been delivered, its callback has returned, and no
    /// thread of this runtime is left - which is what the header promises a host that reads it
    /// before unloading the library.
    pub(crate) fn state(&self) -> ak_runtime_state {
        let stored = ak_runtime_state::from_repr(self.state.load(Ordering::Acquire))
            .unwrap_or(ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED);
        if stored != ak_runtime_state::AK_RUNTIME_GRPC_STOPPED {
            return stored;
        }
        match Held::new(self.teardown.lock().unwrap_or_else(PoisonError::into_inner)).as_ref() {
            Some(thread) if thread.is_finished() => ak_runtime_state::AK_RUNTIME_QUIESCENT,
            _ => stored,
        }
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
    pub(crate) fn pass_the_gate(&self) -> Option<Held<RwLockReadGuard<'_, ()>>> {
        let pass = Held::new(self.gate.read().unwrap_or_else(PoisonError::into_inner));
        (self.state() == ak_runtime_state::AK_RUNTIME_RUNNING).then_some(pass)
    }

    /// Taken and dropped for the wait alone: once this returns, every pass handed out before the
    /// state changed has been given back.
    pub(crate) fn close_the_gate(&self) {
        drop(Held::new(
            self.gate.write().unwrap_or_else(PoisonError::into_inner),
        ));
    }

    /// Hands the rest of the shutdown to a thread of its own.
    ///
    /// Everything after this happens outside tokio: the last event goes out, and then tokio is
    /// shut down, which it refuses from inside itself. What the thread finishing means is that
    /// there is nothing left of this runtime, which is what `state` reports as QUIESCENT - so
    /// putting the shutdown here rather than in a task is what makes that word true.
    pub(crate) fn tear_down(self: &Arc<Self>, owed: bool) {
        let runtime = Arc::clone(self);
        let thread = std::thread::Builder::new()
            .name("armonik-teardown".to_owned())
            .spawn(move || {
                if owed {
                    runtime.host.signal_runtime(
                        ak_event_kind::AK_EVENT_RESOURCES_RELEASED,
                        ak_host_debt::AK_HOST_NOTHING_TO_RETURN,
                    );
                }
                runtime.release_threads();
            });

        // QUIESCENT is this thread having finished, so a runtime that cannot get one will never
        // reach it, and the state says so rather than leaving a host to wait for a step nobody
        // takes. Not QUIESCENT, which would promise it may unload a library whose workers are
        // still running, and no RESOURCES_RELEASED either: a failure suspends what is owed.
        match thread {
            Ok(thread) => {
                *Held::new(self.teardown.lock().unwrap_or_else(PoisonError::into_inner)) =
                    Some(thread);
            }
            Err(_) => self.set_state(ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED),
        }
    }

    /// Waits for every worker to stop, which only a thread outside tokio may do.
    fn release_threads(&self) {
        // The channels' threads first, each after its wind-down. Every call has reached its
        // terminal with no callback in flight, and no task waits for a call's reclaim: the thread
        // that pays a call's last debt removes it from the calls table, and
        // `ak_runtime_destroy` removes what is still owed.
        self.stop_channels.send_replace(true);
        let channel_threads = std::mem::take(&mut *Held::new(
            self.channel_threads
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        ));
        for thread in channel_threads {
            let _ = thread.join();
        }

        let taken = Held::new(self.tokio.lock().unwrap_or_else(PoisonError::into_inner)).take();
        if let Some(tokio) = taken {
            // Dropped rather than given a deadline. This thread finishing is what `state` reports
            // as QUIESCENT, and the header promises that state alone permits `ak_runtime_destroy`
            // or unloading the library - so a deadline that expired with work still running would
            // make the promise a lie exactly when it matters. A shutdown that does not end leaves
            // the state at GRPC_STOPPED, which refuses the destroy and says so; the host's own
            // patience is the host's to bound.
            drop(tokio);
        }
    }

    /// Reaps the teardown thread, which quiescence says has finished.
    pub(crate) fn join_teardown(&self) {
        let taken = Held::new(self.teardown.lock().unwrap_or_else(PoisonError::into_inner)).take();
        if let Some(thread) = taken {
            let _ = thread.join();
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

    #[test]
    fn a_second_threshold_below_the_first_is_refused_with_its_reason() {
        let refused = AkRuntime::new(64, 32, Host::new(never_called, std::ptr::null_mut()))
            .err()
            .expect("a second threshold below the first is refused");

        assert_eq!(refused.status(), ak_status::AK_STATUS_INVALID_ARG);
        assert!(
            format!("{refused:?}").contains("memory_hard_ceiling is below"),
            "{refused:?}"
        );
    }

    /// The refusal happens before a runtime exists, so nothing ever emits through this.
    extern "C" fn never_called(
        _runtime_ctx: *mut std::ffi::c_void,
        _call_ctx: *mut std::ffi::c_void,
        _events: *const crate::abi::ak_event,
        _count: usize,
    ) {
        unreachable!("a refused runtime emits nothing")
    }
}
