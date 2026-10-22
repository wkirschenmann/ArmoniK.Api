//! The deadlines of a channel's calls whose task is not spawned yet.
//!
//! A call that sends one request is spawned by its commit, which saves the call's worker a wake
//! for a task that would only wait for the request. Its deadline still runs from its start, so a
//! call its host neither commits nor cancels is spawned when the deadline passes, and its task
//! ends it DEADLINE_EXCEEDED at once. One watcher per channel holds those deadlines, rather than a
//! task per call: a call costs it an insertion at its start and a removal at its spawn, and the
//! watcher is woken when the earliest deadline it sleeps towards moves earlier, or when it is idle.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use tokio::sync::Notify;
use tokio::time::Instant;

use crate::abi::ak_handle;
use crate::call::CallState;
use crate::held::Held;

pub(crate) struct Deadlines {
    watched: Mutex<Watched>,
    wake: Notify,
}

#[derive(Default)]
struct Watched {
    /// By deadline, then by handle so that two calls with one deadline are two entries.
    calls: BTreeMap<(Instant, ak_handle), Weak<CallState>>,
    /// When the watcher wakes next, or None while it sleeps until it is told. Never later than the
    /// earliest deadline of `calls`.
    armed: Option<Instant>,
    /// The channel is closing: it cancels its calls itself, and the watcher ends.
    closed: bool,
}

impl Deadlines {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            watched: Mutex::new(Watched::default()),
            wake: Notify::new(),
        })
    }

    fn watched(&self) -> Held<MutexGuard<'_, Watched>> {
        Held::new(self.watched.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Spawns `call` at `at`, unless `forget` comes first.
    pub(crate) fn watch(&self, at: Instant, handle: ak_handle, call: Weak<CallState>) {
        let mut watched = self.watched();
        if watched.closed {
            return;
        }
        watched.calls.insert((at, handle), call);
        if watched.armed.is_some_and(|armed| armed <= at) {
            return;
        }
        watched.armed = Some(at);
        drop(watched);
        self.wake.notify_one();
    }

    /// The call is spawned: its task watches its deadline from here.
    pub(crate) fn forget(&self, at: Instant, handle: ak_handle) {
        self.watched().calls.remove(&(at, handle));
    }

    /// The channel is closing, and the watcher ends.
    pub(crate) fn close(&self) {
        {
            let mut watched = self.watched();
            watched.closed = true;
            watched.calls.clear();
        }
        self.wake.notify_one();
    }

    /// The watcher: spawns each call whose deadline has passed, then sleeps until the next one or
    /// until an earlier one is watched.
    pub(crate) async fn run(self: Arc<Self>) {
        loop {
            let mut due = Vec::new();
            let next = {
                let mut watched = self.watched();
                if watched.closed {
                    return;
                }
                let now = Instant::now();
                while let Some(entry) = watched.calls.first_entry() {
                    if entry.key().0 > now {
                        break;
                    }
                    due.push(entry.remove());
                }
                watched.armed = watched.calls.keys().next().map(|(at, _)| *at);
                watched.armed
            };
            // Outside the lock, which a spawn takes again to forget the call.
            for call in due.iter().filter_map(Weak::upgrade) {
                call.spawn_at_deadline();
            }
            drop(due);

            // A watch after the lock was let go and before this waits leaves its permit.
            let told = self.wake.notified();
            match next {
                Some(at) => {
                    tokio::select! {
                        () = tokio::time::sleep_until(at) => {}
                        () = told => {}
                    }
                }
                None => told.await,
            }
        }
    }
}
