// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Generational arena storage for stable-handle element allocation.

use alloc::vec::Vec;
use core::num::NonZeroU32;

use crate::Id;

#[derive(Clone, Debug)]
enum Slot<T> {
    Occupied {
        generation: NonZeroU32,
        value: T,
    },
    Free {
        generation: NonZeroU32,
        next_free: Option<u32>,
    },
}

/// Dense slot arena with tombstones and generational IDs.
///
/// The arena preserves deterministic slot-order iteration and never compacts
/// implicitly. Removed slots are recycled through a free list with bumped
/// generation counters.
#[derive(Clone, Debug)]
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    free_head: Option<u32>,
    len: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Arena<T> {
    /// Creates an empty arena.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_head: None,
            len: 0,
        }
    }

    /// Returns the number of occupied slots.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns true when the arena has no occupied slots.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns total slot count (occupied + tombstoned).
    ///
    /// This is the index space used by stable IDs.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Returns the total slot capacity (occupied + tombstoned).
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }

    /// Inserts a new value and returns a stable generational ID.
    #[must_use]
    pub fn insert(&mut self, value: T) -> Id {
        self.len += 1;
        if let Some(index) = self.free_head {
            let slot_index = index as usize;
            let (generation, next_free) = match self.slots[slot_index] {
                Slot::Free {
                    generation,
                    next_free,
                } => (generation, next_free),
                Slot::Occupied { .. } => unreachable!("free list must point to free slots"),
            };
            self.free_head = next_free;
            self.slots[slot_index] = Slot::Occupied { generation, value };
            return Id::new(index, generation);
        }

        let generation = NonZeroU32::MIN;
        let index = index_to_u32(self.slots.len());
        self.slots.push(Slot::Occupied { generation, value });
        Id::new(index, generation)
    }

    /// Returns an immutable reference for a live ID, or `None` for stale/dead IDs.
    #[must_use]
    pub fn get(&self, id: Id) -> Option<&T> {
        match self.slots.get(id.index() as usize) {
            Some(Slot::Occupied { generation, value }) if *generation == id.generation() => {
                Some(value)
            }
            _ => None,
        }
    }

    /// Returns a mutable reference for a live ID, or `None` for stale/dead IDs.
    #[must_use]
    pub fn get_mut(&mut self, id: Id) -> Option<&mut T> {
        match self.slots.get_mut(id.index() as usize) {
            Some(Slot::Occupied { generation, value }) if *generation == id.generation() => {
                Some(value)
            }
            _ => None,
        }
    }

    /// Removes a live value and returns it, or `None` if the ID is stale/dead.
    pub fn remove(&mut self, id: Id) -> Option<T> {
        let slot_index = id.index() as usize;
        let slot = self.slots.get_mut(slot_index)?;
        let current_generation = match slot {
            Slot::Occupied { generation, .. } if *generation == id.generation() => *generation,
            _ => return None,
        };

        let next_generation = bump_generation(current_generation);
        let next_free = self.free_head;
        let old_slot = core::mem::replace(
            slot,
            Slot::Free {
                generation: next_generation,
                next_free,
            },
        );
        self.free_head = Some(id.index());
        self.len -= 1;

        match old_slot {
            Slot::Occupied { value, .. } => Some(value),
            Slot::Free { .. } => unreachable!("validated occupied slot must remain occupied"),
        }
    }

    /// Iterates occupied entries in deterministic slot order.
    pub fn iter(&self) -> impl Iterator<Item = (Id, &T)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| match slot {
                Slot::Occupied { generation, value } => {
                    Some((Id::new(index_to_u32(index), *generation), value))
                }
                Slot::Free { .. } => None,
            })
    }

    /// Iterates occupied mutable entries in deterministic slot order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Id, &mut T)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(index, slot)| match slot {
                Slot::Occupied { generation, value } => {
                    Some((Id::new(index_to_u32(index), *generation), value))
                }
                Slot::Free { .. } => None,
            })
    }
}

fn bump_generation(current: NonZeroU32) -> NonZeroU32 {
    let next = current
        .get()
        .checked_add(1)
        .expect("arena generation overflowed u32");
    NonZeroU32::new(next).expect("checked_add from non-zero cannot produce zero")
}

fn index_to_u32(index: usize) -> u32 {
    u32::try_from(index).expect("arena index overflowed u32")
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::vec;
    use alloc::vec::Vec;

    use super::Arena;

    #[test]
    fn insert_get_remove_reuse_slot_with_new_generation() {
        let mut arena = Arena::new();
        let first = arena.insert(10_u32);
        assert_eq!(arena.get(first), Some(&10));

        let removed = arena.remove(first);
        assert_eq!(removed, Some(10));
        assert!(arena.get(first).is_none());

        let reused = arena.insert(20_u32);
        assert_eq!(reused.index(), first.index());
        assert_ne!(reused.generation(), first.generation());
        assert_eq!(arena.get(reused), Some(&20));
    }

    #[test]
    fn stale_id_is_rejected_after_slot_reuse() {
        let mut arena = Arena::new();
        let id = arena.insert(1_i32);
        assert_eq!(arena.remove(id), Some(1));

        let replacement = arena.insert(2_i32);
        assert_ne!(replacement.generation(), id.generation());
        assert!(arena.get(id).is_none());
        assert!(arena.get_mut(id).is_none());
        assert_eq!(arena.remove(id), None);
        assert_eq!(arena.get(replacement), Some(&2));
    }

    #[test]
    fn iter_is_deterministic_and_skips_tombstones() {
        let mut arena = Arena::new();
        let a = arena.insert(10_u32);
        let b = arena.insert(20_u32);
        let c = arena.insert(30_u32);
        assert_eq!(arena.remove(b), Some(20));

        let collected: Vec<_> = arena
            .iter()
            .map(|(id, value)| (id.index(), *value))
            .collect();
        assert_eq!(collected, vec![(a.index(), 10), (c.index(), 30)]);
    }

    #[test]
    fn len_and_capacity_are_reported() {
        let mut arena = Arena::new();
        assert_eq!(arena.len(), 0);
        assert!(arena.is_empty());
        assert_eq!(arena.slot_count(), 0);
        let cap0 = arena.capacity();

        let id = arena.insert(42_u8);
        assert_eq!(arena.len(), 1);
        assert!(!arena.is_empty());
        assert_eq!(arena.slot_count(), 1);
        assert!(arena.capacity() >= cap0);

        assert_eq!(arena.remove(id), Some(42));
        assert_eq!(arena.len(), 0);
        assert!(arena.is_empty());
        assert_eq!(arena.slot_count(), 1);
    }

    #[test]
    fn iter_mut_visits_live_slots_in_order() {
        let mut arena = Arena::new();
        let a = arena.insert(1_u32);
        let b = arena.insert(2_u32);
        let _ = arena.remove(a);

        for (_, value) in arena.iter_mut() {
            *value += 10;
        }

        assert!(arena.get(a).is_none());
        assert_eq!(arena.get(b), Some(&12));
    }
}
