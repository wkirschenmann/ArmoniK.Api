//! Handles are tokens, not pointers.
//!
//! Each is a slot index and a generation. A token naming a slot that has since been reused fails
//! its generation check, so a downcall on a reclaimed object reports a status instead of reaching
//! into whatever took its place - which closes ABA on a reused slot, and lets the runtime reclaim
//! a call without asking the host first.

use std::sync::{Arc, Mutex, PoisonError};

use crate::abi::{ak_handle, AK_HANDLE_NONE};

struct Slot<T> {
    generation: u32,
    value: Option<Arc<T>>,
    /// Named and not yet published. Keeps a second reservation off the same slot.
    reserved: bool,
}

/// The live objects of one kind, addressed by token.
pub(crate) struct Registry<T> {
    slots: Mutex<Vec<Slot<T>>>,
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self {
            slots: Mutex::new(Vec::new()),
        }
    }
}

impl<T> Registry<T> {
    /// Takes a free slot and names it, without putting anything in it yet.
    ///
    /// Two phases because an object has to know its own handle before anything can find it, and
    /// because its tasks must not run before it is findable: a value published after its own
    /// reclamation has already looked would stay for the life of the process. Every caller
    /// publishes on the next statement, with nothing fallible in between.
    pub(crate) fn reserve(&self) -> ak_handle {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);

        let index = slots
            .iter()
            .position(|slot| slot.value.is_none() && !slot.reserved);
        let index = match index {
            Some(index) => {
                // A reused slot advances, so every token it ever named but the newest is stale.
                slots[index].generation = slots[index].generation.wrapping_add(1).max(1);
                index
            }
            None => {
                slots.push(Slot {
                    generation: 1,
                    value: None,
                    reserved: false,
                });
                slots.len() - 1
            }
        };
        slots[index].reserved = true;
        token(index, slots[index].generation)
    }

    /// Puts `value` in the slot `handle` names.
    pub(crate) fn publish(&self, handle: ak_handle, value: Arc<T>) {
        let Some((index, generation)) = parts(handle) else {
            return;
        };
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = slots.get_mut(index) {
            if slot.generation == generation {
                slot.value = Some(value);
            }
        }
    }

    /// What the token names, if it still names anything.
    pub(crate) fn get(&self, handle: ak_handle) -> Option<Arc<T>> {
        let (index, generation) = parts(handle)?;
        let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = slots.get(index)?;
        if slot.generation != generation {
            return None;
        }
        slot.value.clone()
    }

    /// Empties the slot, so every token naming it goes stale, and hands back what was in it.
    pub(crate) fn remove(&self, handle: ak_handle) -> Option<Arc<T>> {
        let (index, generation) = parts(handle)?;
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = slots.get_mut(index)?;
        if slot.generation != generation {
            return None;
        }
        slot.reserved = false;
        slot.value.take()
    }

    /// Every value currently held, as a snapshot.
    pub(crate) fn values(&self) -> Vec<Arc<T>> {
        let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        slots.iter().filter_map(|slot| slot.value.clone()).collect()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        slots.iter().filter(|slot| slot.value.is_some()).count()
    }
}

/// The token for a slot at a generation. Generations start at one, so no live token is
/// [`AK_HANDLE_NONE`].
fn token(index: usize, generation: u32) -> ak_handle {
    ((generation as u64) << 32) | (index as u64 & 0xffff_ffff)
}

fn parts(handle: ak_handle) -> Option<(usize, u32)> {
    if handle == AK_HANDLE_NONE {
        return None;
    }
    let generation = (handle >> 32) as u32;
    if generation == 0 {
        return None;
    }
    Some(((handle & 0xffff_ffff) as usize, generation))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two phases as every caller uses them.
    fn insert<T>(registry: &Registry<T>, value: Arc<T>) -> ak_handle {
        let handle = registry.reserve();
        registry.publish(handle, value);
        handle
    }

    #[test]
    fn a_token_names_what_was_put_in_it() {
        let registry = Registry::default();
        let handle = insert(&registry, Arc::new(7u32));

        assert_eq!(registry.get(handle).as_deref(), Some(&7));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_token_for_a_reused_slot_names_nothing() {
        let registry = Registry::default();
        let first = insert(&registry, Arc::new(1u32));
        registry.remove(first);

        let second = insert(&registry, Arc::new(2u32));
        assert_ne!(first, second, "the slot came back at a new generation");
        assert!(registry.get(first).is_none(), "the old token is stale");
        assert_eq!(registry.get(second).as_deref(), Some(&2));
    }

    #[test]
    fn the_null_token_and_a_made_up_one_name_nothing() {
        let registry = Registry::<u32>::default();
        assert!(registry.get(AK_HANDLE_NONE).is_none());
        assert!(registry.get(u64::MAX).is_none());
        assert!(registry.get(token(0, 1)).is_none());
    }

    #[test]
    fn a_reserved_slot_holds_nothing_until_it_is_published() {
        let registry = Registry::<u32>::default();
        let handle = registry.reserve();

        assert!(registry.get(handle).is_none());
        assert!(registry.values().is_empty());
        // A second reservation takes a different slot rather than the one being named.
        assert_ne!(registry.reserve(), handle);

        registry.publish(handle, Arc::new(3));
        assert_eq!(registry.get(handle).as_deref(), Some(&3));
    }

    #[test]
    fn a_snapshot_holds_what_the_registry_holds() {
        let registry = Registry::default();
        let first = insert(&registry, Arc::new(1u32));
        insert(&registry, Arc::new(2u32));
        registry.remove(first);

        let values: Vec<u32> = registry.values().iter().map(|value| **value).collect();
        assert_eq!(values, vec![2]);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn removing_twice_answers_once() {
        let registry = Registry::default();
        let handle = insert(&registry, Arc::new(1u32));

        assert!(registry.remove(handle).is_some());
        assert!(registry.remove(handle).is_none());
    }
}
