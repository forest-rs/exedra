// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hash index of sibling keys.
//!
//! Instances and placement sets share one key namespace per parent. The index
//! stores only handles; keys and parents are read back from the owning
//! vectors, so no key string is duplicated. It is never iterated, so its
//! randomly seeded hasher cannot affect output order.

use core::hash::BuildHasher;

use hashbrown::{DefaultHashBuilder, HashTable};

use super::{Instance, InstanceId, PlacementSet, PlacementSetId};

/// One keyed child of a parent: an instance or a placement set.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum Sibling {
    Instance(InstanceId),
    Set(PlacementSetId),
}

/// Lookup from `(parent, key)` to the sibling holding that key.
#[derive(Clone, Debug, Default)]
pub(crate) struct SiblingIndex {
    table: HashTable<Sibling>,
    hasher: DefaultHashBuilder,
}

/// The records a sibling handle refers to.
#[derive(Copy, Clone)]
pub(crate) struct Records<'a> {
    pub(crate) instances: &'a [Instance],
    pub(crate) sets: &'a [PlacementSet],
}

impl Records<'_> {
    fn parent_and_key(&self, sibling: Sibling) -> (Option<InstanceId>, &str) {
        match sibling {
            Sibling::Instance(id) => {
                let instance = &self.instances[id.0 as usize];
                (instance.parent, &instance.key)
            }
            Sibling::Set(id) => {
                let set = &self.sets[id.0 as usize];
                (set.parent, &set.key)
            }
        }
    }
}

impl SiblingIndex {
    fn hash(&self, parent: Option<InstanceId>, key: &str) -> u64 {
        self.hasher.hash_one((parent.map(|p| p.0), key))
    }

    /// The sibling keyed `key` under `parent`, if any.
    pub(crate) fn find(
        &self,
        records: Records<'_>,
        parent: Option<InstanceId>,
        key: &str,
    ) -> Option<Sibling> {
        let hash = self.hash(parent, key);
        self.table
            .find(hash, |&sibling| {
                records.parent_and_key(sibling) == (parent, key)
            })
            .copied()
    }

    /// Records a sibling whose record is already stored in `records`.
    ///
    /// Callers check uniqueness first with [`Self::find`].
    pub(crate) fn insert(&mut self, records: Records<'_>, sibling: Sibling) {
        let (parent, key) = records.parent_and_key(sibling);
        let hash = self.hash(parent, key);
        let hasher = &self.hasher;
        self.table.insert_unique(hash, sibling, |&existing| {
            let (parent, key) = records.parent_and_key(existing);
            hasher.hash_one((parent.map(|p| p.0), key))
        });
    }
}
