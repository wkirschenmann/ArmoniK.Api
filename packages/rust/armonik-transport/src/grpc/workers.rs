//! The threads a runtime's connections run on, and which one a new connection is given.
//!
//! A connection is given a worker when it is dialled and keeps it for its whole life, and every
//! call on it runs there too: the dial, the connection's task, hyper's tasks for each call, the
//! call's own task. A worker carries the connections of any channel that shares the pool.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::runtime::Handle;

use crate::options::WorkerAssignment;

/// Where a channel's connections and calls run: one runtime the caller gives, or a pool of
/// threads several channels share.
///
/// Cheap to clone: every clone is the same pool.
#[derive(Clone)]
pub struct Workers(Arc<Pool>);

/// What starts the worker of an index and answers with its runtime, or with none when the system
/// gives no thread.
type Start = Box<dyn Fn(usize) -> Option<Handle> + Send + Sync>;

struct Pool {
    assignment: WorkerAssignment,
    /// None for a runtime the caller gave, which is never started.
    start: Option<Start>,
    slots: Mutex<Slots>,
}

struct Slots {
    slots: Vec<Slot>,
    /// The next worker `RoundRobin` gives.
    next: usize,
    /// The workers whose thread the system refused, not reported yet: the report waits until no
    /// lock is held, since a host's log callback runs inside the emitting call.
    unreported: Vec<usize>,
}

#[derive(Default)]
struct Slot {
    handle: Option<Handle>,
    /// The connections it carries, opened or being dialled.
    connections: usize,
    /// The system gave no thread for it, and it is not asked again.
    refused: bool,
}

/// A worker: its place in the pool and the runtime its work is spawned on.
#[derive(Clone, Debug)]
pub(crate) struct Worker {
    pub(crate) index: usize,
    pub(crate) handle: Handle,
}

/// A connection's claim on its worker, counted against it until dropped: when the dial fails, or
/// with the task that drives the connection.
pub(crate) struct Claim {
    worker: Worker,
    pool: Workers,
}

impl Claim {
    pub(crate) fn worker(&self) -> &Worker {
        &self.worker
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut slots = self.pool.slots();
        let slot = &mut slots.slots[self.worker.index];
        slot.connections = slot.connections.saturating_sub(1);
    }
}

impl Workers {
    /// A pool of `count` workers, at least one, which `assignment` gives connections and calls to,
    /// and which `start` starts as each is first chosen. The first is started here, and none is
    /// returned when it cannot be: a pool with no thread runs nothing.
    pub fn new(
        count: usize,
        assignment: WorkerAssignment,
        start: impl Fn(usize) -> Option<Handle> + Send + Sync + 'static,
    ) -> Option<Self> {
        let first = start(0)?;
        let mut slots: Vec<Slot> = std::iter::repeat_with(Slot::default)
            .take(count.max(1))
            .collect();
        slots[0].handle = Some(first);
        Some(Self(Arc::new(Pool {
            assignment,
            start: Some(Box::new(start)),
            slots: Mutex::new(Slots {
                slots,
                next: 0,
                unreported: Vec::new(),
            }),
        })))
    }

    /// Whether every connection and call runs on the one runtime a caller gave.
    pub(crate) fn is_one_runtime(&self) -> bool {
        self.0.start.is_none()
    }

    /// Logs the workers whose thread the system refused since the last report. Called where no
    /// lock is held.
    pub(crate) fn report_refusals(&self) {
        let refused = std::mem::take(&mut self.slots().unreported);
        for worker in refused {
            tracing::warn!(
                worker,
                "the system gave no thread for a worker; its connections go to the others"
            );
        }
    }

    fn slots(&self) -> MutexGuard<'_, Slots> {
        self.0.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many threads the pool has started.
    #[cfg(test)]
    fn started(&self) -> usize {
        self.slots()
            .slots
            .iter()
            .filter(|slot| slot.handle.is_some())
            .count()
    }

    /// The connections each worker carries, by index.
    pub(crate) fn connections(&self) -> Vec<usize> {
        self.slots()
            .slots
            .iter()
            .map(|slot| slot.connections)
            .collect()
    }

    /// The worker of `index`, which a connection or a dial already runs on.
    pub(crate) fn worker(&self, index: usize) -> Worker {
        worker(&self.slots(), index)
    }

    /// The worker a call that holds no connection yet runs on, by the pool's rule, without
    /// counting a connection against it: the connection it dials later goes there. It takes a
    /// turn of `RoundRobin` as that connection would.
    pub(crate) fn choose(&self) -> Worker {
        let mut slots = self.slots();
        let index = self.pick(&mut slots);
        worker(&slots, index)
    }

    /// A claim for a new connection on `on`, or on the worker the pool's rule gives when `on` is
    /// none.
    pub(crate) fn claim(&self, on: Option<usize>) -> Claim {
        let mut slots = self.slots();
        let index = match on {
            Some(index) => index,
            None => self.pick(&mut slots),
        };
        slots.slots[index].connections += 1;
        let worker = worker(&slots, index);
        Claim {
            worker,
            pool: self.clone(),
        }
    }

    /// The index the rule gives, its worker started, or the started worker with the fewest
    /// connections when the system refuses the thread, which is reported later.
    ///
    /// The thread starts under the lock, so that two dials do not both start the same worker: a
    /// start runs no code of this crate's and takes no lock of it, and is once per worker.
    fn pick(&self, slots: &mut Slots) -> usize {
        let count = slots.slots.len();
        let chosen = match self.0.assignment {
            WorkerAssignment::LeastConnections => least_connections(slots, |slot| !slot.refused),
            WorkerAssignment::RoundRobin => {
                let mut index = slots.next;
                // Past the refused ones; the first worker always started, so one is found.
                while slots.slots[index].refused {
                    index = (index + 1) % count;
                }
                slots.next = (index + 1) % count;
                index
            }
        };
        if slots.slots[chosen].handle.is_some() {
            return chosen;
        }
        let started = self.0.start.as_ref().and_then(|start| start(chosen));
        match started {
            Some(handle) => {
                slots.slots[chosen].handle = Some(handle);
                chosen
            }
            None => {
                slots.slots[chosen].refused = true;
                slots.unreported.push(chosen);
                least_connections(slots, |slot| slot.handle.is_some())
            }
        }
    }
}

/// The eligible worker with the fewest connections, the first of equals.
fn least_connections(slots: &Slots, eligible: impl Fn(&Slot) -> bool) -> usize {
    slots
        .slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| eligible(slot))
        .min_by_key(|(index, slot)| (slot.connections, *index))
        .map_or(0, |(index, _)| index)
}

fn worker(slots: &Slots, index: usize) -> Worker {
    Worker {
        index,
        handle: slots.slots[index]
            .handle
            .clone()
            .expect("a worker is started before it is given work"),
    }
}

/// One worker, the runtime given: every connection and call of the channel runs on it, as that
/// runtime schedules them.
impl From<Handle> for Workers {
    fn from(handle: Handle) -> Self {
        Self(Arc::new(Pool {
            assignment: WorkerAssignment::LeastConnections,
            start: None,
            slots: Mutex::new(Slots {
                slots: vec![Slot {
                    handle: Some(handle),
                    connections: 0,
                    refused: false,
                }],
                next: 0,
                unreported: Vec::new(),
            }),
        }))
    }
}

impl std::fmt::Debug for Workers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workers")
            .field("assignment", &self.0.assignment)
            .field("connections", &self.connections())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn pool(count: usize, assignment: WorkerAssignment) -> (Workers, Arc<AtomicUsize>) {
        let handle = Handle::current();
        let started = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&started);
        let workers = Workers::new(count, assignment, move |_| {
            counted.fetch_add(1, Ordering::Relaxed);
            Some(handle.clone())
        })
        .expect("a first worker");
        (workers, started)
    }

    #[tokio::test]
    async fn the_fewest_connections_take_the_next_and_a_worker_starts_when_first_given_one() {
        let (workers, started) = pool(3, WorkerAssignment::LeastConnections);
        assert_eq!(
            started.load(Ordering::Relaxed),
            1,
            "the first starts with the pool"
        );

        let claims: Vec<Claim> = (0..7).map(|_| workers.claim(None)).collect();
        assert_eq!(workers.connections(), [3, 2, 2]);
        assert_eq!(started.load(Ordering::Relaxed), 3);

        // The second worker's connections close: it is the next one given.
        let left: Vec<Claim> = claims
            .into_iter()
            .filter(|claim| claim.worker().index != 1)
            .collect();
        assert_eq!(workers.connections(), [3, 0, 2]);
        assert_eq!(workers.claim(None).worker().index, 1);
        drop(left);
    }

    #[tokio::test]
    async fn round_robin_gives_each_in_turn_whatever_it_carries() {
        let (workers, _) = pool(3, WorkerAssignment::RoundRobin);
        let first: Vec<Claim> = (0..3).map(|_| workers.claim(None)).collect();
        drop(first);
        let indices: Vec<usize> = (0..4).map(|_| workers.claim(None).worker().index).collect();
        assert_eq!(indices, [0, 1, 2, 0]);
    }

    #[tokio::test]
    async fn a_call_without_a_connection_is_given_a_worker_and_counts_nothing() {
        let (workers, started) = pool(2, WorkerAssignment::LeastConnections);
        let _claim = workers.claim(None);
        assert_eq!(workers.choose().index, 1);
        assert_eq!(workers.connections(), [1, 0]);
        assert_eq!(started.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn a_connection_on_a_named_worker_counts_there() {
        let (workers, _) = pool(2, WorkerAssignment::LeastConnections);
        let _first = workers.claim(None);
        let _second = workers.claim(Some(0));
        assert_eq!(workers.connections(), [2, 0]);
    }

    #[tokio::test]
    async fn a_thread_the_system_refuses_sends_its_connections_to_the_others() {
        let handle = Handle::current();
        let workers = Workers::new(3, WorkerAssignment::LeastConnections, move |index| {
            (index != 1).then(|| handle.clone())
        })
        .expect("a first worker");
        let claims: Vec<Claim> = (0..4).map(|_| workers.claim(None)).collect();
        assert_eq!(workers.connections(), [2, 0, 2]);
        assert_eq!(workers.started(), 2);
        drop(claims);
    }

    #[test]
    fn a_pool_whose_first_thread_is_refused_is_none() {
        assert!(Workers::new(2, WorkerAssignment::default(), |_| None).is_none());
    }
}
