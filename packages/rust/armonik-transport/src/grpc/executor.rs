use std::future::Future;

/// A `tokio` handle in the shape hyper wants for its HTTP/2 connection task.
///
/// Spawning on the handle rather than on the ambient runtime is the point: the caller names which
/// runtime drives the engine, and for the C ABI that is a runtime the host never enters.
#[derive(Clone, Debug)]
pub(crate) struct Spawner(pub(crate) tokio::runtime::Handle);

impl<F> hyper::rt::Executor<F> for Spawner
where
    F: Future<Output = ()> + Send + 'static,
{
    fn execute(&self, future: F) {
        self.0.spawn(future);
    }
}
