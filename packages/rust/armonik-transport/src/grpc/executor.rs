use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type BoxedTask = Pin<Box<dyn Future<Output = ()> + Send>>;

pub trait Executor: Send + Sync + 'static {
    fn spawn(&self, future: BoxedTask);
}

#[derive(Clone, Debug)]
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
pub(crate) struct HyperExecutor(pub(crate) Arc<dyn Executor>);

impl<F> hyper::rt::Executor<F> for HyperExecutor
where
    F: Future<Output = ()> + Send + 'static,
{
    fn execute(&self, future: F) {
        self.0.spawn(Box::pin(future));
    }
}
