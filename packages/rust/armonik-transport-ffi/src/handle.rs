//! Detecting a stale pointer at the ABI boundary instead of dereferencing it.
//!
//! `ak_client` and `ak_call` are plain heap pointers handed to the caller, in the ordinary
//! `Box::into_raw`/`Box::from_raw` style. On their own, a double `_free` call or a use of a handle
//! after it was freed would dereference already-released memory: undefined behaviour, not a
//! reportable error. [`LiveSet`] closes most of that gap cheaply: every live handle's address is
//! recorded on creation and removed on free, and every entry point checks membership before
//! touching the pointer.
//!
//! This is a pragmatic middle ground, not a complete guarantee, and that limit is worth stating
//! plainly: if a handle is freed and the allocator later hands the exact same address to an
//! unrelated new handle, a stale caller using the old value would be validated against the *new*
//! object rather than rejected. Closing that gap fully needs a generation-tagged slot table that
//! never actually deallocates a slot, which is a larger structure than the two call sites here
//! warrant. What this does catch, cleanly and without touching freed memory, is the common cases: a
//! double free, and a use-after-free before that address has been reused for anything else.

use std::collections::HashSet;
use std::sync::{PoisonError, RwLock};

/// Addresses of the live instances of one handle type (`ak_client`, `ak_call`, ...).
///
/// Wrapped in a `OnceLock` by every call site (rather than being a plain `static LiveSet =
/// LiveSet::new()`) because `HashSet::new` reads from the OS to seed its hasher and so is not a
/// `const fn`, which a `static` initializer requires.
///
/// An [`RwLock`] rather than a `Mutex`: [`Self::contains`] runs on the hot path — a caller polls
/// `ak_call_try_recv` in a loop, across every concurrent call — while insertion and removal happen
/// once per handle. A mutex would serialise all of those polls against each other for no reason.
pub(crate) struct LiveSet {
    live: RwLock<HashSet<usize>>,
}

impl LiveSet {
    pub(crate) fn new() -> Self {
        Self {
            live: RwLock::new(HashSet::new()),
        }
    }

    /// Record `ptr` as live. Panics if it was already recorded, since that would mean the same
    /// address was allocated twice without an intervening free — a bug in this crate's allocator
    /// usage, not a caller mistake.
    pub(crate) fn insert<T>(&self, ptr: *mut T) {
        let inserted = self
            .live
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ptr as usize);
        assert!(inserted, "a handle was inserted while already live");
    }

    /// Remove `ptr` from the live set if present, reporting whether it was there.
    ///
    /// `false` means this exact address is not currently a live handle of this type — either it was
    /// already freed, or it never was one — and the caller must not proceed to free or otherwise
    /// touch the underlying memory.
    pub(crate) fn remove<T>(&self, ptr: *mut T) -> bool {
        self.live
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&(ptr as usize))
    }

    /// Whether `ptr` is currently a live handle of this type.
    pub(crate) fn contains<T>(&self, ptr: *const T) -> bool {
        self.live
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&(ptr as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_pointer_is_not_live() {
        let set = LiveSet::new();
        let value = 0u8;
        assert!(!set.contains(std::ptr::addr_of!(value)));
    }

    #[test]
    fn inserted_pointers_are_live_until_removed() {
        let set = LiveSet::new();
        let value = 0u8;
        let ptr = std::ptr::addr_of!(value).cast_mut();

        set.insert(ptr);
        assert!(set.contains(ptr));

        assert!(set.remove(ptr));
        assert!(!set.contains(ptr));
    }

    #[test]
    fn removing_an_absent_pointer_is_reported_rather_than_silently_ignored() {
        let set = LiveSet::new();
        let value = 0u8;
        let ptr = std::ptr::addr_of!(value).cast_mut();

        // Never inserted: this is what catches a double-free or a use of a bogus pointer.
        assert!(!set.remove(ptr));
    }

    #[test]
    fn a_double_remove_only_succeeds_once() {
        let set = LiveSet::new();
        let value = 0u8;
        let ptr = std::ptr::addr_of!(value).cast_mut();

        set.insert(ptr);
        assert!(set.remove(ptr));
        assert!(
            !set.remove(ptr),
            "the second free of the same handle must be rejected"
        );
    }
}
