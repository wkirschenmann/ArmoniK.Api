//! Where the channel's background work runs.
//!
//! The engine spawns rather than blocks: a connection is driven by a task, and so is each call.
//! Which runtime those tasks land on is the embedder's decision, which is what this trait leaves
//! open.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// A future the engine hands over to be run to completion.
pub type BoxedTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Somewhere to run the engine's background tasks.
pub trait Executor: Send + Sync + 'static {
    /// Runs `future` to completion, concurrently with the caller.
    fn spawn(&self, future: BoxedTask) -> TaskHandle;
}

/// A spawned task, insofar as the engine can still act on it.
pub struct TaskHandle {
    cancel: Option<Box<dyn Fn() + Send + Sync>>,
}

impl TaskHandle {
    /// A handle whose `cancel` runs `cancel`.
    pub fn new(cancel: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            cancel: Some(Box::new(cancel)),
        }
    }

    /// A handle for an executor that cannot stop a task once it is running.
    pub fn detached() -> Self {
        Self { cancel: None }
    }

    /// Stops the task, if the executor can. Idempotent.
    pub fn cancel(&self) {
        if let Some(cancel) = &self.cancel {
            cancel();
        }
    }
}

impl std::fmt::Debug for TaskHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskHandle")
            .field("cancellable", &self.cancel.is_some())
            .finish()
    }
}

/// Runs the engine's tasks on a Tokio runtime.
///
/// It holds a handle rather than reaching for the current one at spawn time, so a downcall from a
/// thread the runtime knows nothing about - which is what an FFI thread is - still spawns.
#[derive(Clone, Debug)]
pub struct TokioExecutor {
    handle: tokio::runtime::Handle,
}

impl TokioExecutor {
    /// Runs tasks on the runtime `handle` belongs to.
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

/// The engine's executor, in the shape hyper asks for.
///
/// hyper spawns futures of its own concrete types rather than boxed ones, so this stands between
/// the two and boxes them.
#[derive(Clone)]
pub(crate) struct HyperExecutor(pub(crate) Arc<dyn Executor>);

impl<F> hyper::rt::Executor<F> for HyperExecutor
where
    F: Future<Output = ()> + Send + 'static,
{
    fn execute(&self, future: F) {
        self.0.spawn(Box::pin(future));
    }
}
