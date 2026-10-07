//! Point 1: a runtime whose threads all carry its own dispatcher.
//!
//! Two kinds of thread, as the FFI crate's runtime has them: a small multi-thread tokio runtime for
//! the shutdown's orchestration, and a std thread per channel running a current-thread runtime.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use tokio::sync::oneshot;
use tracing::dispatcher::{self, DefaultGuard};
use tracing::Dispatch;

use crate::obs::{bare_dispatch, layered_dispatch, RtObs};

thread_local! {
    static GUARD: RefCell<Option<DefaultGuard>> = const { RefCell::new(None) };
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Front {
    Bare,
    Layered,
}

pub struct ObsRuntime {
    pub obs: Arc<RtObs>,
    pub dispatch: Dispatch,
    pub tokio: tokio::runtime::Runtime,
}

pub struct ChannelThread {
    pub handle: tokio::runtime::Handle,
    stop: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

/// Installs `dispatch` on the calling thread until the thread ends or `leave_thread` runs.
fn enter_thread(dispatch: &Dispatch) {
    let guard = dispatcher::set_default(dispatch);
    GUARD.with(|slot| *slot.borrow_mut() = Some(guard));
}

fn leave_thread() {
    GUARD.with(|slot| drop(slot.borrow_mut().take()));
}

/// A dispatcher kept for the life of the process that wants nothing.
///
/// `tracing` registers a callsite against every live dispatcher, but with exactly one it asks only
/// the dispatcher of the thread that hits the callsite first: a thread under no runtime then
/// caches `never` for a callsite the runtime's own threads emit. A second, uninterested dispatcher
/// keeps the list at two or more, so that every callsite is asked of every runtime. It says
/// `never`, which leaves a callsite no runtime wants at `never`, and makes one a runtime wants
/// `sometimes`: its `enabled` is asked of the emitting thread's dispatcher.
struct Sentinel;

impl tracing::Subscriber for Sentinel {
    fn register_callsite(&self, _: &'static tracing::Metadata<'static>) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::never()
    }
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        false
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

static SENTINEL: std::sync::OnceLock<Dispatch> = std::sync::OnceLock::new();

/// Whether new runtimes register the sentinel first; tests turn it off to show the trap.
pub static USE_SENTINEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn install_sentinel() {
    SENTINEL.get_or_init(|| Dispatch::new(Sentinel));
}

impl ObsRuntime {
    pub fn new(front: Front) -> Self {
        if USE_SENTINEL.load(Ordering::Relaxed) {
            install_sentinel();
        }
        let obs = RtObs::new(NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let dispatch = match front {
            Front::Bare => bare_dispatch(&obs),
            Front::Layered => layered_dispatch(&obs),
        };
        let for_start = dispatch.clone();
        let tokio = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("spike-runtime")
            .enable_all()
            // Worker and blocking-pool threads alike.
            .on_thread_start(move || enter_thread(&for_start))
            .on_thread_stop(leave_thread)
            .build()
            .expect("a runtime");
        ObsRuntime {
            obs,
            dispatch,
            tokio,
        }
    }

    /// Inside an `ak_*` call on a host's thread.
    pub fn scope(&self) -> DefaultGuard {
        dispatcher::set_default(&self.dispatch)
    }

    pub fn start_channel_thread(&self) -> ChannelThread {
        let dispatch = self.dispatch.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let join = std::thread::Builder::new()
            .name(format!("spike-channel-{}", self.obs.id))
            .spawn(move || {
                // Scoped to the whole thread body; its blocking-pool threads take it from the builder.
                dispatcher::with_default(&dispatch, || {
                    let for_start = dispatch.clone();
                    let tokio = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .on_thread_start(move || enter_thread(&for_start))
                        .on_thread_stop(leave_thread)
                        .build()
                        .expect("a runtime");
                    let (stop, stopped) = oneshot::channel::<()>();
                    ready_tx.send((tokio.handle().clone(), stop)).unwrap();
                    tokio.block_on(async move {
                        let _ = stopped.await;
                    });
                })
            })
            .expect("a thread");
        let (handle, stop) = ready_rx.recv().unwrap();
        ChannelThread {
            handle,
            stop: Some(stop),
            join: Some(join),
        }
    }
}

impl Drop for ChannelThread {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
