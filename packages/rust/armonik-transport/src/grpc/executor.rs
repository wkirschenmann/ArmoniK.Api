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
///
/// Spawning answers nothing: a task the engine started is one it ends by other means - a call by
/// its terminal, the session by its last sender being dropped - so a handle to abort it would be
/// a second way to stop something that already has one, and the two could disagree.
pub trait Executor: Send + Sync + 'static {
    fn spawn(&self, future: BoxedTask);
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
    fn spawn(&self, future: BoxedTask) {
        self.handle.spawn(future);
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
