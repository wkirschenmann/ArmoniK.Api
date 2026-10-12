//! A channel's thread polling for work for a while before it sleeps.
//!
//! The thread is a tokio current-thread runtime, which sleeps in its I/O driver when it has no
//! task to run, and wakes when a socket is ready, a timer is due, or another thread hands it work.
//! A wake costs the operating system's scheduling of the thread. What the host does with an event,
//! such as its next request, a buffer given back or its next message once the last is written,
//! comes back to the thread soon after the callback returns, so for a window after it last called
//! the host, the thread polls the driver without blocking instead, which sees all three sources.
//!
//! Tokio has no hook into its driver's park, but it calls `on_thread_park` before it parks and
//! does not park when that callback has scheduled a task. The callback wakes a task kept for it,
//! the poller, which yields: a yielded task has the scheduler poll the driver with a timeout of
//! zero and run what that found before it looks again.

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Notify;

thread_local! {
    /// When this thread last called the host, until its window has passed.
    static CALLED: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Marks this thread as having called the host, which opens its window anew.
pub(crate) fn host_called() {
    CALLED.set(Some(Instant::now()));
}

/// The window, and the poller's wake-up.
#[derive(Clone)]
pub(crate) struct Spin {
    window: Duration,
    poll: Arc<Notify>,
}

impl Spin {
    pub(crate) fn new(window: Duration) -> Self {
        Self {
            window,
            poll: Arc::new(Notify::new()),
        }
    }

    /// Run by tokio before the thread parks: keeps it awake while the window is open.
    pub(crate) fn before_park(&self) {
        let Some(called) = CALLED.get() else {
            return;
        };
        if called.elapsed() < self.window {
            self.poll.notify_one();
        } else {
            CALLED.set(None);
        }
    }

    /// The poller: each wake-up is one poll of the driver.
    pub(crate) fn poller(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        let poll = Arc::clone(&self.poll);
        async move {
            loop {
                poll.notified().await;
                tokio::task::yield_now().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The park count is what tokio submits each time it polls its driver, parked or not, so it is
    /// current whenever a task reads it.
    #[test]
    fn a_thread_polls_through_its_window_and_then_sleeps() {
        let spin = Spin::new(Duration::from_millis(200));
        let hook = spin.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .on_thread_park(move || hook.before_park())
            .build()
            .expect("a runtime");
        runtime.spawn(spin.poller());

        let (within, after) = runtime.block_on(async {
            let parks = || {
                tokio::runtime::Handle::current()
                    .metrics()
                    .worker_park_count(0)
            };
            host_called();
            let opened = parks();
            tokio::time::sleep(Duration::from_millis(50)).await;
            let within = parks() - opened;
            tokio::time::sleep(Duration::from_millis(400)).await;
            (within, parks() - opened)
        });

        assert_eq!(within, 0, "the thread does not sleep inside its window");
        assert!(
            (1..10).contains(&after),
            "past its window the thread sleeps until the timer, rather than polling: {after} parks"
        );
    }

    #[test]
    fn a_thread_that_has_not_called_the_host_sleeps_at_once() {
        let spin = Spin::new(Duration::from_secs(10));
        let hook = spin.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .on_thread_park(move || hook.before_park())
            .build()
            .expect("a runtime");
        runtime.spawn(spin.poller());

        let parks = runtime.block_on(async {
            CALLED.set(None);
            tokio::time::sleep(Duration::from_millis(50)).await;
            tokio::runtime::Handle::current()
                .metrics()
                .worker_park_count(0)
        });

        assert!(parks >= 1, "a thread with nothing to wait for sleeps");
    }
}
