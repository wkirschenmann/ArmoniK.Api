use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard};
use std::time::Duration;

use armonik_transport::grpc::Workers;
use armonik_transport::metrics::Metrics;
use armonik_transport::options::{ChannelOptions, RuntimeOptions};
use tokio::sync::watch;

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
    /// What `ak_runtime_stats` reads: what the runtime counts for itself, such as its memory
    /// ceiling's waits, and every endpoint's registry, which are its children.
    metrics: Metrics,
    /// The registry of each endpoint a channel was created on, by its host and port, which lives
    /// as long as the runtime so that a closed channel's counts stay in what its endpoint read.
    endpoints: Mutex<HashMap<String, Metrics>>,
    state: AtomicI32,
    gate: RwLock<()>,
    /// The thread that finishes the shutdown, once there is one.
    ///
    /// Not a tokio task: it emits the last event and then shuts tokio down, which tokio refuses
    /// from inside itself. Its having finished is what QUIESCENT means - see `state`.
    teardown: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The threads every channel's connections run on, each a current-thread runtime that runs
    /// the connections it is given and the calls on them, so that a call never moves between
    /// threads.
    workers: Workers,
    /// The workers' threads, joined by the teardown, which is what keeps QUIESCENT meaning that no
    /// thread of this runtime is left.
    worker_threads: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    /// Raised by the teardown, which every worker stops on. Dropped with a runtime that never
    /// reached its teardown, which stops them too.
    stop_workers: watch::Sender<bool>,
    /// What the host configured: the endpoint a channel created with none reaches, and the
    /// options every channel's own are merged over.
    options: RuntimeOptions,
}

/// The teardown thread, parked until `start`.
pub(crate) struct Teardown(std::sync::mpsc::Sender<()>);

impl Teardown {
    pub(crate) fn start(self) {
        let _ = self.0.send(());
    }
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

/// How long a worker waits, once it is told to stop, for the work left on it: the close of its
/// connections, a cancelled call's cleanup.
const WORKER_WIND_DOWN: Duration = Duration::from_secs(1);

const THRESHOLDS_CROSSED: Refusal = Refusal::fixed(
    ak_status::AK_STATUS_INVALID_ARG,
    ak_error_kind::AK_ERROR_USAGE,
    "memory_hard_ceiling is below the first threshold in force: memory_ceiling, or this library's own when that is zero",
);

impl AkRuntime {
    pub(crate) fn relinquish() {
        // Before the claim is open to another runtime, whose sink this must not take away.
        crate::log::detach();
        LIVE.store(false, Ordering::Release);
    }

    /// A ceiling of zero bytes, soft then hard, is the ledger's own.
    pub(crate) fn new(
        options: RuntimeOptions,
        (ceiling, hard_ceiling): (u64, u64),
        host: Host,
    ) -> Result<Arc<Self>, Refusal> {
        let metrics = Metrics::new();
        let ledger = Ledger::counting_in(ceiling, hard_ceiling, metrics.clone())
            .map_err(|_| THRESHOLDS_CROSSED)?;

        // One worker: what runs here is the shutdown's orchestration, the channels' work running
        // on threads of their own.
        let tokio = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("armonik-runtime")
            .enable_all()
            .build()
            .map_err(|_| ak_status::AK_STATUS_INTERNAL)?;
        let spawner = tokio.handle().clone();

        let stop_workers = watch::channel(false).0;
        let worker_threads = Arc::new(Mutex::new(Vec::new()));
        let workers = {
            let stop = stop_workers.subscribe();
            let threads = Arc::clone(&worker_threads);
            let spin = options.spin_window();
            Workers::new(
                options.worker_count(),
                options.worker_assignment(),
                move |index| {
                    // Under the lock the teardown takes the threads under, after it raised the
                    // stop: a worker is either joined by it or never started.
                    let mut threads =
                        Held::new(threads.lock().unwrap_or_else(PoisonError::into_inner));
                    if *stop.borrow() {
                        return None;
                    }
                    let (handle, thread) = start_worker(index, spin, stop.clone())?;
                    threads.push(thread);
                    Some(handle)
                },
            )
            .ok_or(ak_status::AK_STATUS_INTERNAL)?
        };

        Ok(Arc::new(Self {
            tokio: Mutex::new(Some(tokio)),
            spawner,
            host: Arc::new(host),
            ledger: Arc::new(ledger),
            metrics,
            endpoints: Mutex::new(HashMap::new()),
            state: AtomicI32::new(ak_runtime_state::AK_RUNTIME_RUNNING as i32),
            gate: RwLock::new(()),
            teardown: Mutex::new(None),
            workers,
            worker_threads,
            stop_workers,
            options,
        }))
    }

    pub(crate) fn options(&self) -> &RuntimeOptions {
        &self.options
    }

    pub(crate) fn channel_defaults(&self) -> Option<&ChannelOptions> {
        self.options.channel_defaults.as_ref()
    }

    /// The threads every channel's connections and calls run on.
    pub(crate) fn workers(&self) -> &Workers {
        &self.workers
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

    pub(crate) fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// The registry every channel on `endpoint` counts into, made on the first.
    pub(crate) fn metrics_of(&self, endpoint: &str) -> Metrics {
        self.endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(endpoint.to_owned())
            .or_insert_with(|| self.metrics.child())
            .clone()
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

    /// GRPC_STOPPING -> FAILED_UNQUIESCED, and nothing from any other state: the model fails a
    /// runtime only while it runs or stops.
    pub(crate) fn fail_while_stopping(&self) {
        let _ = self.state.compare_exchange(
            ak_runtime_state::AK_RUNTIME_GRPC_STOPPING as i32,
            ak_runtime_state::AK_RUNTIME_FAILED_UNQUIESCED as i32,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn is_stopped(&self) -> bool {
        self.state.load(Ordering::Acquire) == ak_runtime_state::AK_RUNTIME_GRPC_STOPPED as i32
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

    /// The thread that finishes the shutdown, parked until it is started, or None when the system
    /// gives none.
    ///
    /// Everything it does happens outside tokio: it waits for the host to give back what it
    /// owes, the last event goes out, and then tokio is shut down, which it refuses from inside
    /// itself. What the thread finishing means is that there is nothing left of this runtime,
    /// which is what `state` reports as QUIESCENT - so putting the shutdown here rather than in a
    /// task is what makes that word true. It goes on only from GRPC_STOPPED: started, or let go
    /// by a shutdown that panicked after the store; let go before it, it ends at once.
    pub(crate) fn prepare_teardown(self: &Arc<Self>, owed: bool) -> Option<Teardown> {
        #[cfg(feature = "test-hooks")]
        if crate::hooks::refuses_teardown_thread() {
            return None;
        }
        let (start, started) = std::sync::mpsc::channel::<()>();
        let runtime = Arc::clone(self);
        let thread = std::thread::Builder::new()
            .name("armonik-teardown".to_owned())
            .spawn(move || {
                let _ = started.recv();
                if !runtime.is_stopped() {
                    return;
                }
                if owed {
                    runtime.spawner.block_on(runtime.ledger.drained());
                    runtime.host.signal_runtime(
                        ak_event_kind::AK_EVENT_RESOURCES_RELEASED,
                        ak_host_debt::AK_HOST_NOTHING_TO_RETURN,
                    );
                }
                runtime.release_threads();
            })
            .ok()?;
        *Held::new(self.teardown.lock().unwrap_or_else(PoisonError::into_inner)) = Some(thread);
        Some(Teardown(start))
    }

    /// Waits for every worker to stop, which only a thread outside tokio may do.
    fn release_threads(&self) {
        // The workers first, each after its wind-down, which they wait out together. Every call
        // has reached its terminal with no callback in flight, and no task waits for a call's
        // reclaim: the thread that pays a call's last debt removes it from the calls table, and
        // `ak_runtime_destroy` removes what is still owed.
        self.stop_workers.send_replace(true);
        let worker_threads = std::mem::take(&mut *Held::new(
            self.worker_threads
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        ));
        for thread in worker_threads {
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

/// Starts worker `index`: a current-thread runtime on a thread of its own that runs until `stop`
/// is raised, or its sender is dropped. With a spin window, it polls for work for that window
/// before it sleeps (`crate::spin`). None when the system gives no thread, or no runtime on it.
///
/// Stopped, the worker first gives the work left on it - the close of its connections, a
/// cancelled call's cleanup - up to `WORKER_WIND_DOWN` to finish, so that a peer sees a connection
/// closed rather than dropped. The teardown comes after the shutdown has closed every channel, so
/// their connections are closing too.
fn start_worker(
    index: usize,
    spin: Option<Duration>,
    mut stop: watch::Receiver<bool>,
) -> Option<(tokio::runtime::Handle, std::thread::JoinHandle<()>)> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let spin = spin.map(crate::spin::Spin::new);
    let thread = std::thread::Builder::new()
        .name(format!("armonik-worker-{index}"))
        .spawn(move || {
            #[cfg(feature = "test-hooks")]
            let _alive = crate::hooks::WorkerAlive::new();
            let mut builder = tokio::runtime::Builder::new_current_thread();
            builder.enable_all();
            if let Some(spin) = &spin {
                spin.install(&mut builder);
            }
            let Ok(tokio) = builder.build() else {
                return;
            };
            if ready_tx.send(tokio.handle().clone()).is_err() {
                return;
            }
            let poller = spin.map(|spin| tokio.spawn(spin.poller()));
            tokio.block_on(async move {
                let _ = stop.wait_for(|stopping| *stopping).await;
                // Before the wind-down, which waits for every task to end.
                if let Some(poller) = poller {
                    poller.abort();
                }
                // Polled, because tokio offers no wait on a runtime's tasks.
                let left = tokio::runtime::Handle::current();
                let _ = tokio::time::timeout(WORKER_WIND_DOWN, async {
                    while left.metrics().num_alive_tasks() > 0 {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await;
            });
        })
        .ok()?;
    match ready_rx.recv() {
        Ok(handle) => Some((handle, thread)),
        Err(_) => {
            let _ = thread.join();
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing else in this crate's unit tests touches `LIVE`; the lock is the log's, since giving
    /// the claim back detaches the log's sink, which another unit test attaches. An integration
    /// test would need `ak_runtime_destroy` to give the claim back.
    #[test]
    fn a_claim_not_kept_is_given_back_however_the_holder_leaves() {
        let _sink = crate::log::SINK_TESTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
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
        let refused = AkRuntime::new(
            RuntimeOptions::default(),
            (64, 32),
            Host::new(never_called, std::ptr::null_mut()),
        )
        .err()
        .expect("a second threshold below the first is refused");

        assert_eq!(refused.status(), ak_status::AK_STATUS_INVALID_ARG);
        assert!(
            format!("{refused:?}").contains("memory_hard_ceiling is below"),
            "{refused:?}"
        );
    }

    /// Stops `thread`'s worker and answers how long it took to end.
    fn stopped(stop: &watch::Sender<bool>, thread: &std::thread::JoinHandle<()>) -> Duration {
        let stopped = std::time::Instant::now();
        stop.send_replace(true);
        while !thread.is_finished() && stopped.elapsed() < 2 * WORKER_WIND_DOWN {
            std::thread::sleep(Duration::from_millis(5));
        }
        stopped.elapsed()
    }

    /// The poller is a task of the worker's runtime, aborted before the wind-down, which waits up
    /// to `WORKER_WIND_DOWN` for every task to end.
    #[test]
    fn a_spinning_worker_stops_without_waiting_out_its_wind_down() {
        let stop = watch::channel(false).0;
        let (_, thread) =
            start_worker(0, Some(Duration::from_micros(100)), stop.subscribe()).expect("a worker");

        let took = stopped(&stop, &thread);
        assert!(
            took < WORKER_WIND_DOWN / 2,
            "the worker took {took:?} to stop"
        );
    }

    #[test]
    fn a_worker_that_sleeps_at_once_runs_its_work_and_stops() {
        let stop = watch::channel(false).0;
        let (handle, thread) = start_worker(0, None, stop.subscribe()).expect("a worker");

        let (ran, done) = std::sync::mpsc::channel();
        handle.spawn(async move {
            let _ = ran.send(std::thread::current().name().map(str::to_owned));
        });
        let name = done
            .recv_timeout(Duration::from_secs(10))
            .expect("the worker runs what it is given");
        assert_eq!(name.as_deref(), Some("armonik-worker-0"));

        stopped(&stop, &thread);
        assert!(thread.is_finished(), "the worker stops when told");
    }

    #[test]
    fn a_worker_stops_when_what_would_tell_it_is_gone() {
        let stop = watch::channel(false).0;
        let (_, thread) = start_worker(0, None, stop.subscribe()).expect("a worker");

        drop(stop);
        let dropped = std::time::Instant::now();
        while !thread.is_finished() && dropped.elapsed() < 2 * WORKER_WIND_DOWN {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            thread.is_finished(),
            "a runtime dropped unstopped leaves no worker"
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
