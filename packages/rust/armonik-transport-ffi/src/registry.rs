//! Handles are tokens, not pointers.
//!
//! Each is a slot index and a generation. A token naming a slot that has since been reused fails
//! its generation check, so a downcall on a reclaimed object reports a status instead of reaching
//! into whatever took its place - which closes ABA on a reused slot, and lets the runtime reclaim
//! a call without asking the host first.

use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::abi::{ak_handle, AK_HANDLE_NONE};

struct Slot<T> {
    generation: u32,
    value: Option<Arc<T>>,
    /// Named and not yet published. Keeps a second reservation off the same slot.
    reserved: bool,
}

impl<T> Slot<T> {
    /// Whether anything holds this slot: a value in it, or a reservation naming it.
    fn occupied(&self) -> bool {
        self.reserved || self.value.is_some()
    }
}

/// The slots and the indices nobody holds, which have to move together.
struct Slots<T> {
    slots: Vec<Slot<T>>,
    /// Slots holding nothing and named by nobody. A reservation pops one rather than scanning,
    /// so the ten-thousandth call starts as cheaply as the first.
    free: Vec<usize>,
}

/// The live objects of one kind, addressed by token.
///
/// Behind a read/write lock and not a mutex: every downcall resolves its handle here first, and
/// that is a read. Two sends on two calls have no reason to take turns.
pub(crate) struct Registry<T> {
    slots: RwLock<Slots<T>>,
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self {
            slots: RwLock::new(Slots {
                slots: Vec::new(),
                free: Vec::new(),
            }),
        }
    }
}

impl<T> Registry<T> {
    /// The poison is taken rather than reported: a panic under this lock leaves the slots
    /// consistent, and refusing every later downcall over it would be the worse failure.
    fn read(&self) -> RwLockReadGuard<'_, Slots<T>> {
        self.slots.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Slots<T>> {
        self.slots.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Puts a value in the registry, built knowing the handle it will answer to.
    ///
    /// One call and not two, because the halves must not be separated: an object has to know its
    /// own handle before anything can find it, and its tasks must not run before it is findable -
    /// a value published after its own reclamation has already looked would stay for the life of
    /// the process. Whatever `build` makes besides the value itself comes back alongside the
    /// handle, so nothing has to happen in between.
    ///
    /// `build` must not reach back into this registry: the reservation is not published yet, and
    /// the lock is not held while it runs.
    pub(crate) fn insert_with<R>(
        &self,
        build: impl FnOnce(ak_handle) -> (Arc<T>, R),
    ) -> (ak_handle, R) {
        let handle = self.reserve();
        let (value, rest) = build(handle);
        self.publish(handle, value);
        (handle, rest)
    }

    /// The same, for a value that does not need to know its own handle.
    pub(crate) fn insert(&self, value: Arc<T>) -> ak_handle {
        self.insert_with(|_| (value, ())).0
    }

    /// Takes a free slot and names it, without putting anything in it yet.
    fn reserve(&self) -> ak_handle {
        let mut held = self.write();
        let held = &mut *held;

        let index = match held.free.pop() {
            Some(index) => {
                // A reused slot advances, so every token it ever named but the newest is stale.
                held.slots[index].generation = held.slots[index].generation.wrapping_add(1).max(1);
                index
            }
            None => {
                held.slots.push(Slot {
                    generation: 1,
                    value: None,
                    reserved: false,
                });
                held.slots.len() - 1
            }
        };
        held.slots[index].reserved = true;
        token(index, held.slots[index].generation)
    }

    /// Puts `value` in the slot `handle` names.
    fn publish(&self, handle: ak_handle, value: Arc<T>) {
        let Some((index, generation)) = parts(handle) else {
            return;
        };
        let mut held = self.write();
        if let Some(slot) = held.slots.get_mut(index) {
            if slot.generation == generation {
                slot.value = Some(value);
            }
        }
    }

    /// What the token names, if it still names anything.
    pub(crate) fn get(&self, handle: ak_handle) -> Option<Arc<T>> {
        let (index, generation) = parts(handle)?;
        let held = self.read();
        let slot = held.slots.get(index)?;
        if slot.generation != generation {
            return None;
        }
        slot.value.clone()
    }

    /// Empties the slot, so every token naming it goes stale, and hands back what was in it.
    pub(crate) fn remove(&self, handle: ak_handle) -> Option<Arc<T>> {
        let (index, generation) = parts(handle)?;
        let mut held = self.write();
        let held = &mut *held;
        let slot = held.slots.get_mut(index)?;
        if slot.generation != generation {
            return None;
        }
        // A slot goes back on the free list once: a second removal of the same token finds it
        // holding nothing and named by nobody, and a second entry would let two reservations
        // land on it.
        if slot.occupied() {
            held.free.push(index);
        }
        slot.reserved = false;
        slot.value.take()
    }

    /// Every value currently held, as a snapshot.
    pub(crate) fn values(&self) -> Vec<Arc<T>> {
        let held = self.read();
        held.slots
            .iter()
            .filter_map(|slot| slot.value.clone())
            .collect()
    }

    /// Empties the registry, staling every token it ever handed out, and hands back what it held.
    ///
    /// The values come back rather than being dropped under the lock, so a `Drop` that reaches
    /// this registry cannot deadlock against the write this call holds.
    pub(crate) fn drain(&self) -> Vec<Arc<T>> {
        let mut held = self.write();
        let held = &mut *held;
        let mut taken = Vec::new();
        for index in 0..held.slots.len() {
            let slot = &mut held.slots[index];
            // Only a slot this call empties goes back on the free list, and it stops being
            // occupied at the same moment. Both halves matter, and each guards one aliasing:
            // freeing an index whose `publish` has not run would let a later reservation take a
            // slot with a live value in it, and leaving a drained slot occupied would let its
            // own `remove` push the index a second time, so two reservations would name it.
            if let Some(value) = slot.value.take() {
                taken.push(value);
                slot.reserved = false;
                held.free.push(index);
            }
        }
        taken
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        let held = self.read();
        held.slots
            .iter()
            .filter(|slot| slot.value.is_some())
            .count()
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

    #[test]
    fn a_token_names_what_was_put_in_it() {
        let registry = Registry::default();
        let handle = registry.insert(Arc::new(7u32));

        assert_eq!(registry.get(handle).as_deref(), Some(&7));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_token_for_a_reused_slot_names_nothing() {
        let registry = Registry::default();
        let first = registry.insert(Arc::new(1u32));
        registry.remove(first);

        let second = registry.insert(Arc::new(2u32));
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
    fn a_value_knows_the_handle_it_will_answer_to() {
        let registry = Registry::<ak_handle>::default();
        let (handle, extra) = registry.insert_with(|handle| (Arc::new(handle), "beside it"));

        assert_eq!(registry.get(handle).as_deref(), Some(&handle));
        assert_eq!(extra, "beside it");
    }

    #[test]
    fn a_snapshot_holds_what_the_registry_holds() {
        let registry = Registry::default();
        let first = registry.insert(Arc::new(1u32));
        registry.insert(Arc::new(2u32));
        registry.remove(first);

        let values: Vec<u32> = registry.values().iter().map(|value| **value).collect();
        assert_eq!(values, vec![2]);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn removing_twice_answers_once() {
        let registry = Registry::default();
        let handle = registry.insert(Arc::new(1u32));

        assert!(registry.remove(handle).is_some());
        assert!(registry.remove(handle).is_none());

        // The second removal freed nothing, so the slot is reserved once and not twice.
        let reused = registry.insert(Arc::new(2u32));
        assert_ne!(registry.insert(Arc::new(3u32)), reused);
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn draining_stales_every_token_and_hands_back_what_was_held() {
        let registry = Registry::default();
        let first = registry.insert(Arc::new(1u32));
        let second = registry.insert(Arc::new(2u32));

        let mut taken: Vec<u32> = registry.drain().iter().map(|value| **value).collect();
        taken.sort_unstable();
        assert_eq!(taken, vec![1, 2]);
        assert!(registry.get(first).is_none());
        assert!(registry.get(second).is_none());
        assert_eq!(registry.len(), 0);
        assert!(registry.drain().is_empty(), "a second drain finds nothing");

        // A drained token freed its slot once, so removing it frees nothing further: a second
        // entry on the free list would let two reservations land on one slot.
        assert!(registry.remove(first).is_none());
        let reused = registry.insert(Arc::new(3u32));
        assert_ne!(registry.insert(Arc::new(4u32)), reused);
        assert_eq!(registry.len(), 2);
    }
}
