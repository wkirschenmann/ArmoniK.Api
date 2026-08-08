//! Keeping a handle alive for as long as a call is using it.
//!
//! `ak_client` and `ak_request` reach the caller as opaque pointers, and every entry point has to
//! turn one back into something it can read. Doing that with `Box::into_raw`/`Box::from_raw` and a
//! set of live addresses is not enough, and the gap is not theoretical: the check that a pointer is
//! live and the use of what it points at are two separate moments, so a `_free` on another thread
//! can land between them and deallocate the object a call is halfway through reading. A host
//! application whose UI thread cancels and releases a request while a pool thread is still writing
//! to it does exactly that.
//!
//! So the registry *owns* the handles. Every live handle is an [`Arc`] the registry holds, and an
//! entry point takes a counted reference for the duration of its call. `_free` drops the registry's
//! reference; the allocation itself goes away when the last call using it has returned. A caller can
//! free a handle from one thread while another is mid-call and observe nothing worse than that call
//! finishing normally.
//!
//! One gap remains, and it is worth stating plainly: a pointer used after its `_free` is rejected
//! only until the allocator hands that address to a new handle of the same type, at which point the
//! stale pointer would resolve to the new object rather than being refused. Closing that needs
//! generation-tagged slots that are never reused, which is a larger structure than these two call
//! sites warrant.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

/// The live handles of one type (`ak_client`, `ak_request`), keyed by the address the caller holds.
///
/// Wrapped in a `OnceLock` by every call site (rather than being a plain `static Registry =
/// Registry::new()`) because `HashMap::new` reads from the OS to seed its hasher and so is not a
/// `const fn`, which a `static` initializer requires.
///
/// An [`RwLock`] rather than a `Mutex`: [`Self::get`] runs on the hot path - every
/// `ak_request_read` and `ak_request_write`, on every concurrent request - while insertion and
/// removal happen once per handle. A mutex would serialise all of those against each other for no
/// reason.
pub(crate) struct Registry<T> {
    live: RwLock<HashMap<usize, Arc<T>>>,
}

impl<T> Registry<T> {
    pub(crate) fn new() -> Self {
        Self {
            live: RwLock::new(HashMap::new()),
        }
    }

    /// Take ownership of `value` and return the address the caller will identify it by.
    ///
    /// The address is the `Arc`'s payload, which does not move for the life of the allocation.
    pub(crate) fn insert(&self, value: T) -> *const T {
        let value = Arc::new(value);
        let ptr = Arc::as_ptr(&value);
        let previous = self
            .live
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ptr as usize, value);
        assert!(
            previous.is_none(),
            "a handle was registered at an address already in use"
        );
        ptr
    }

    /// A counted reference to the live handle at `ptr`, or `None` if there is none.
    ///
    /// Holding the returned `Arc` is what makes it safe to keep reading the handle after this
    /// returns: a concurrent `_free` drops the registry's reference, not this one.
    pub(crate) fn get(&self, ptr: *const T) -> Option<Arc<T>> {
        self.live
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(ptr as usize))
            .cloned()
    }

    /// Give up the registry's reference to the handle at `ptr`.
    ///
    /// `None` means this address is not currently a live handle of this type - either it was
    /// already freed, or it never was one - which is what catches a double free. The returned `Arc`
    /// is the registry's own reference; dropping it releases the allocation only if no call is
    /// still using it.
    pub(crate) fn remove(&self, ptr: *const T) -> Option<Arc<T>> {
        self.live
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&(ptr as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_pointer_is_not_live() {
        let registry = Registry::<u32>::new();
        let value = 0u32;
        assert!(registry.get(std::ptr::addr_of!(value)).is_none());
    }

    #[test]
    fn a_registered_handle_is_reachable_until_it_is_removed() {
        let registry = Registry::new();
        let ptr = registry.insert(7u32);

        assert_eq!(registry.get(ptr).map(|value| *value), Some(7));
        assert!(registry.remove(ptr).is_some());
        assert!(registry.get(ptr).is_none());
    }

    #[test]
    fn removing_an_absent_pointer_is_reported_rather_than_silently_ignored() {
        let registry = Registry::<u32>::new();
        let value = 0u32;

        // Never registered: this is what catches a double free or a bogus pointer.
        assert!(registry.remove(std::ptr::addr_of!(value)).is_none());
    }

    #[test]
    fn a_double_remove_only_succeeds_once() {
        let registry = Registry::new();
        let ptr = registry.insert(7u32);

        assert!(registry.remove(ptr).is_some());
        assert!(
            registry.remove(ptr).is_none(),
            "the second free of the same handle must be rejected"
        );
    }

    #[test]
    fn a_handle_freed_while_a_call_holds_it_stays_readable() {
        // The race the whole module exists for. A borrowed handle has to survive a `_free` that
        // lands while the call is still using it; under the old set-of-addresses scheme this read
        // was a use-after-free.
        let registry = Registry::new();
        let ptr = registry.insert(String::from("still here"));

        let borrowed = registry.get(ptr).expect("live");
        assert!(registry.remove(ptr).is_some());
        assert!(
            registry.get(ptr).is_none(),
            "the handle is gone as far as any new call is concerned"
        );
        assert_eq!(borrowed.as_str(), "still here");
    }
}
