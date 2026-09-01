//! Where the channel's background work runs.
//!
//! The engine spawns rather than blocks: a task drives the HTTP/2 session, and one drives each
//! call. Which runtime they land on is the embedder's decision, which is what this trait leaves
//! open - the FFI runtime supplies its own.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type BoxedTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Somewhere to run the engine's background tasks.
pub trait Executor: Send + Sync + 'static {
    fn spawn(&self, future: BoxedTask) -> TaskHandle;
}

/// A spawned task, insofar as the engine can still act on it.
pub struct TaskHandle {
    cancel: Box<dyn Fn() + Send + Sync>,
}

impl TaskHandle {
    pub fn new(cancel: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            cancel: Box::new(cancel),
        }
    }

    /// Stops the task, insofar as the executor can. Idempotent.
    pub fn cancel(&self) {
        (self.cancel)();
    }
}

#[derive(Clone, Debug)]
/// Runs the engine's tasks on a Tokio runtime.
///
/// It holds a handle rather than reaching for the current one at spawn time, so a downcall from a
/// thread the runtime knows nothing about - which is what an FFI thread is - still spawns.
pub struct TokioExecutor {
    handle: tokio::runtime::Handle,
}

impl TokioExecutor {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self { handle }
    }
}

impl Executor for TokioExecutor {
    fn spawn(&self, future: BoxedTask) -> TaskHandle {
        let task = self.handle.spawn(future);
        TaskHandle::new(move || task.abort())
    }
}

#[derive(Clone)]
/// The engine's executor in the shape hyper asks for: hyper spawns futures of its own concrete
/// types rather than boxed ones.
pub(crate) struct HyperExecutor(pub(crate) Arc<dyn Executor>);

impl<F> hyper::rt::Executor<F> for HyperExecutor
where
    F: Future<Output = ()> + Send + 'static,
{
    fn execute(&self, future: F) {
        self.0.spawn(Box::pin(future));
    }
}
