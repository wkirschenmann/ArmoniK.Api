//! How many of this crate's locks the current thread holds, checked where a callback starts.
//!
//! A callback runs host code, and host code may make a downcall. A downcall that needs a lock the
//! calling thread holds waits for itself, and one that needs a lock another thread holds while
//! waiting for the callback to return closes a cycle across the ABI. So no callback starts with a
//! lock of this crate held. Counted in debug builds only, where the whole test suite then checks
//! the rule at every event it emits.

use std::ops::{Deref, DerefMut};

#[cfg(debug_assertions)]
thread_local! {
    static HELD: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A lock guard, counted as held for as long as it lives.
pub(crate) struct Held<G>(G);

impl<G> Held<G> {
    pub(crate) fn new(guard: G) -> Self {
        #[cfg(debug_assertions)]
        HELD.with(|held| held.set(held.get() + 1));
        Self(guard)
    }
}

impl<G> Drop for Held<G> {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        HELD.with(|held| held.set(held.get() - 1));
    }
}

impl<G: Deref> Deref for Held<G> {
    type Target = G::Target;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<G: DerefMut> DerefMut for Held<G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Panics, in a debug build, when this thread holds a lock of this crate.
pub(crate) fn assert_none_held() {
    #[cfg(debug_assertions)]
    {
        let held = HELD.with(std::cell::Cell::get);
        assert_eq!(
            held, 0,
            "a callback starts with {held} of this crate's locks held"
        );
    }
}

// Debug builds only, like the count they check.
#[cfg(all(test, debug_assertions))]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    #[should_panic(expected = "a callback starts with 1 of this crate's locks held")]
    fn a_callback_with_a_lock_held_is_refused() {
        let lock = Mutex::new(());
        let _held = Held::new(lock.lock().expect("not poisoned"));
        assert_none_held();
    }

    #[test]
    fn a_lock_given_back_is_no_longer_counted() {
        let lock = Mutex::new(());
        drop(Held::new(lock.lock().expect("not poisoned")));
        assert_none_held();
    }
}
