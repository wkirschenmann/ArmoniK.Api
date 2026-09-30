use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::task::Poll;

/// Runs `body` to its end, or to `None` if it panics; the panic is caught in the poll it happens
/// in.
///
/// For a task that owes someone an outcome: a panic ends the task in silence, and whoever waits
/// for that outcome waits for good.
pub(crate) async fn contained<T>(body: impl Future<Output = T>) -> Option<T> {
    let mut body = std::pin::pin!(body);
    std::future::poll_fn(
        |cx| match catch_unwind(AssertUnwindSafe(|| body.as_mut().poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => Poll::Ready(Some(value)),
            Err(_) => Poll::Ready(None),
        },
    )
    .await
}
