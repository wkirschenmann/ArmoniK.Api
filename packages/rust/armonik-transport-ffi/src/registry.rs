use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::abi::{ak_handle, AK_HANDLE_NONE};

struct Slot<T> {
    generation: u32,
    value: Option<Arc<T>>,
    reserved: bool,
}

impl<T> Slot<T> {
    fn occupied(&self) -> bool {
        self.reserved || self.value.is_some()
    }
}

struct Slots<T> {
    slots: Vec<Slot<T>>,
    free: Vec<usize>,
}

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
    fn read(&self) -> RwLockReadGuard<'_, Slots<T>> {
        self.slots.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Slots<T>> {
        self.slots.write().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn insert_with<R>(
        &self,
        build: impl FnOnce(ak_handle) -> (Arc<T>, R),
    ) -> (ak_handle, R) {
        let handle = self.reserve();
        let (value, rest) = build(handle);
        self.publish(handle, value);
        (handle, rest)
    }

    pub(crate) fn insert(&self, value: Arc<T>) -> ak_handle {
        self.insert_with(|_| (value, ())).0
    }

    fn reserve(&self) -> ak_handle {
        let mut held = self.write();
        let held = &mut *held;

        let index = match held.free.pop() {
            Some(index) => {
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

    pub(crate) fn get(&self, handle: ak_handle) -> Option<Arc<T>> {
        let (index, generation) = parts(handle)?;
        let held = self.read();
        let slot = held.slots.get(index)?;
        if slot.generation != generation {
            return None;
        }
        slot.value.clone()
    }

    pub(crate) fn remove(&self, handle: ak_handle) -> Option<Arc<T>> {
        let (index, generation) = parts(handle)?;
        let mut held = self.write();
        let held = &mut *held;
        let slot = held.slots.get_mut(index)?;
        if slot.generation != generation {
            return None;
        }
        if slot.occupied() {
            held.free.push(index);
        }
        slot.reserved = false;
        slot.value.take()
    }

    pub(crate) fn values(&self) -> Vec<Arc<T>> {
        let held = self.read();
        held.slots
            .iter()
            .filter_map(|slot| slot.value.clone())
            .collect()
    }

    pub(crate) fn drain(&self) -> Vec<Arc<T>> {
        let mut held = self.write();
        let held = &mut *held;
        let mut taken = Vec::new();
        for index in 0..held.slots.len() {
            let slot = &mut held.slots[index];
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

        assert!(registry.remove(first).is_none());
        let reused = registry.insert(Arc::new(3u32));
        assert_ne!(registry.insert(Arc::new(4u32)), reused);
        assert_eq!(registry.len(), 2);
    }
}
