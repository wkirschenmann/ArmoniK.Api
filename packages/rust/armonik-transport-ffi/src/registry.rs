use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::abi::ak_handle;

/// The three handle spaces, disjoint, so a handle of one kind is absent from the others' tables
/// rather than naming whatever lives there.
///
/// Each is sized by how many of that kind a process ever asks for, not by how many are live at
/// once: a handle is never reused, so the range bounds the total. Runtimes are the narrow one and
/// still take four billion, which is a hundred and thirty years at one create-and-destroy cycle a
/// second. Calls take the top half, so the test on the hottest path is the sign bit.
///
/// `AK_HANDLE_NONE` is zero and no range starts there, so the null token is refused by the same
/// comparison as any other handle from the wrong space.
pub(crate) const RUNTIMES: Range<u64> = 1..1 << 32;
pub(crate) const CHANNELS: Range<u64> = 1 << 32..1 << 63;
pub(crate) const CALLS: Range<u64> = 1 << 63..u64::MAX;

/// A handle is already a perfect key: it comes from a counter, so its low bits choose a bucket with
/// no collision at all until a table has been handed more handles than it holds.
///
/// The multiplication is for the other end of the word. hashbrown takes its control byte from the
/// top seven bits, and a counter has none of them set - every entry would carry the same control
/// byte, and the group scan that should reject on one comparison would compare every occupied key
/// instead. Multiplying by an odd constant fills the top and leaves the low bits a bijection, so
/// the bucket stays as evenly chosen as it was.
#[derive(Default)]
pub(crate) struct Spread(u64);

impl Hasher for Spread {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, _: &[u8]) {
        unreachable!("a handle is hashed as one u64")
    }

    fn write_u64(&mut self, handle: u64) {
        self.0 = handle.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

type Handles<T> = HashMap<ak_handle, Arc<T>, BuildHasherDefault<Spread>>;

/// The live objects of one kind, each under the handle it was given and no other, ever.
pub(crate) struct Registry<T> {
    first: u64,
    past: u64,
    next: AtomicU64,
    live: RwLock<Handles<T>>,
}

impl<T> Registry<T> {
    pub(crate) fn new(space: Range<u64>) -> Self {
        Self {
            first: space.start,
            past: space.end,
            next: AtomicU64::new(space.start),
            live: RwLock::new(Handles::default()),
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, Handles<T>> {
        self.live.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Handles<T>> {
        self.live.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether this handle was drawn from this table's space.
    ///
    /// Not what makes a handle of another kind safe - the tables are disjoint, so such a handle is
    /// simply absent. What this saves is the lookup, and what it could say is which kind the caller
    /// passed, which absence cannot.
    fn holds(&self, handle: ak_handle) -> bool {
        self.first <= handle && handle < self.past
    }

    /// The next handle of this space, or nothing when the space is spent.
    ///
    /// The counter keeps climbing past the end and every claim after it is refused, which is what
    /// stops an exhausted space from spilling into the next one. Reaching it takes 2^32 runtimes or
    /// 2^63 calls.
    fn claim(&self) -> Option<ak_handle> {
        let handle = self.next.fetch_add(1, Ordering::Relaxed);
        self.holds(handle).then_some(handle)
    }

    pub(crate) fn insert_with<R>(
        &self,
        build: impl FnOnce(ak_handle) -> (Arc<T>, R),
    ) -> Option<(ak_handle, R)> {
        let handle = self.claim()?;

        // Built before the lock is taken: nothing can look the handle up until it is inserted, so
        // there is no half-made state for a reader to find and no reason to hold a writer out
        // while a channel or a call is assembled.
        let (value, rest) = build(handle);
        self.write().insert(handle, value);
        Some((handle, rest))
    }

    pub(crate) fn insert(&self, value: Arc<T>) -> Option<ak_handle> {
        self.insert_with(|_| (value, ())).map(|(handle, ())| handle)
    }

    pub(crate) fn get(&self, handle: ak_handle) -> Option<Arc<T>> {
        if !self.holds(handle) {
            return None;
        }
        self.read().get(&handle).cloned()
    }

    pub(crate) fn remove(&self, handle: ak_handle) -> Option<Arc<T>> {
        if !self.holds(handle) {
            return None;
        }
        self.write().remove(&handle)
    }

    pub(crate) fn values(&self) -> Vec<Arc<T>> {
        self.read().values().cloned().collect()
    }

    pub(crate) fn drain(&self) -> Vec<Arc<T>> {
        self.write().drain().map(|(_, value)| value).collect()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calls() -> Registry<u32> {
        Registry::new(CALLS)
    }

    #[test]
    fn a_handle_answers_with_what_it_was_given() {
        let registry = calls();
        let handle = registry.insert(Arc::new(7u32)).expect("a handle");

        assert_eq!(registry.get(handle).as_deref(), Some(&7));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_removed_handle_names_nothing_and_never_will_again() {
        let registry = calls();
        let first = registry.insert(Arc::new(1u32)).expect("a handle");

        assert_eq!(registry.remove(first).as_deref(), Some(&1));
        assert!(registry.get(first).is_none());
        assert!(registry.remove(first).is_none(), "removed once");

        // The whole point of a counter over a slot map: nothing comes back to the freed name, so
        // the handle above cannot ever be the one this hands out.
        let second = registry.insert(Arc::new(2u32)).expect("a handle");
        assert_ne!(first, second);
        assert!(registry.get(first).is_none(), "still nothing");
    }

    #[test]
    fn a_handle_of_another_kind_is_refused_rather_than_resolved() {
        let calls = calls();
        let channels = Registry::new(CHANNELS);
        let runtimes = Registry::new(RUNTIMES);

        let call = calls.insert(Arc::new(1u32)).expect("a handle");
        let channel = channels.insert(Arc::new(2u32)).expect("a handle");
        let runtime = runtimes.insert(Arc::new(3u32)).expect("a handle");

        assert!(channels.get(call).is_none());
        assert!(runtimes.get(call).is_none());
        assert!(calls.get(channel).is_none());
        assert!(calls.get(runtime).is_none());
        assert!(channels.get(runtime).is_none());
    }

    #[test]
    fn the_null_token_names_nothing_in_any_space() {
        assert!(calls().get(0).is_none());
        assert!(Registry::<u32>::new(CHANNELS).get(0).is_none());
        assert!(Registry::<u32>::new(RUNTIMES).get(0).is_none());
    }

    #[test]
    fn a_spent_space_refuses_rather_than_spilling_into_the_next() {
        // The last handle of the runtime space, so the claim after it is the first one past.
        let registry: Registry<u32> = Registry::new(RUNTIMES.end - 1..RUNTIMES.end);

        let last = registry.insert(Arc::new(1u32)).expect("the last handle");
        assert!(RUNTIMES.contains(&last));
        assert!(
            registry.insert(Arc::new(2u32)).is_none(),
            "the space is spent"
        );
        assert!(
            registry.insert(Arc::new(3u32)).is_none(),
            "and stays spent rather than wrapping onto a live handle"
        );
    }

    #[test]
    fn drain_takes_everything_and_leaves_the_handles_stale() {
        let registry = calls();
        let first = registry.insert(Arc::new(1u32)).expect("a handle");
        let second = registry.insert(Arc::new(2u32)).expect("a handle");

        let mut taken: Vec<u32> = registry.drain().iter().map(|value| **value).collect();
        taken.sort_unstable();
        assert_eq!(taken, vec![1, 2]);

        assert_eq!(registry.len(), 0);
        assert!(registry.get(first).is_none());
        assert!(registry.get(second).is_none());
        assert!(registry.drain().is_empty(), "a second drain finds nothing");
    }

    #[test]
    fn the_spaces_are_disjoint_and_ordered() {
        assert_eq!(RUNTIMES.end, CHANNELS.start);
        assert_eq!(CHANNELS.end, CALLS.start);
        assert!(!RUNTIMES.contains(&0), "the null token belongs to no space");
    }
}
