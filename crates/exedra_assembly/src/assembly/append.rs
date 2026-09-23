// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{format, vec};

use super::*;
use crate::compose;

/// Source-to-destination handles from an assembly append.
///
/// Handles are indexed by the source assembly snapshot used for the append.
/// Omitted instances and unreferenced parts have no destination handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendMap {
    parts: Vec<Option<PartId>>,
    instances: Vec<Option<InstanceId>>,
    placement_sets: Vec<Option<PlacementSetId>>,
}

impl AppendMap {
    /// Destination handle for a copied source part, or `None` if unused or unknown.
    #[must_use]
    pub fn part(&self, source: PartId) -> Option<PartId> {
        self.parts.get(source.0 as usize).copied().flatten()
    }

    /// Destination handle for a copied source instance, or `None` if omitted or unknown.
    #[must_use]
    pub fn instance(&self, source: InstanceId) -> Option<InstanceId> {
        self.instances.get(source.0 as usize).copied().flatten()
    }

    /// Destination handle for a copied source placement set, or `None` if
    /// omitted or unknown.
    #[must_use]
    pub fn placement_set(&self, source: PlacementSetId) -> Option<PlacementSetId> {
        self.placement_sets
            .get(source.0 as usize)
            .copied()
            .flatten()
    }
}

impl Assembly {
    /// Appends source instance trees under `parent`, or at the root when `None`.
    ///
    /// Copies each referenced source part once, with the lower levels of its
    /// level-of-detail chain, preserving sharing, slot order,
    /// region mappings and default materials. Instance bindings and metadata are
    /// copied unchanged. Unreferenced part definitions are omitted. Existing
    /// destination parts are not interned or reused by this operation.
    ///
    /// Source root placements are relative to the destination parent. Source
    /// frames retain their identity and do not require an artificial part.
    /// Part keys and the keys of source root instances and root placement sets
    /// become `"{prefix}-{source_key}"`; descendant keys and local placements
    /// retain their source values. The supplied placement is composed onto
    /// source roots and onto every placement of root placement sets only.
    /// Placement sets follow their parent instance; root placement sets are
    /// always copied. The prefix must be nonempty and contain no `/` or `#`. Returned handles associate source records
    /// with their copies without relying on insertion order or derived names.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown parent, an invalid prefix, a destination key collision,
    /// a nonfinite supplied or composed root placement, or exhausted handles.
    /// On any returned error, the destination remains unchanged.
    pub fn append(
        &mut self,
        parent: Option<InstanceId>,
        source: &Self,
        prefix: &str,
        placement: Placement3,
    ) -> Result<AppendMap, AssemblyError> {
        self.append_selected(parent, source, prefix, placement, |_, _| true)
    }

    /// Appends selected source instance subtrees, with the same contract as [`Self::append`].
    ///
    /// Calls `include` in source insertion order, once for each instance whose
    /// ancestors were retained. Returning `false` omits the instance and its
    /// whole subtree; the predicate is not called for those descendants.
    /// Only parts referenced by retained instances are copied. The predicate's
    /// own side effects are outside the destination's atomic error guarantee.
    ///
    /// # Errors
    ///
    /// Reports the same errors as [`Self::append`], without changing the destination.
    pub fn append_selected(
        &mut self,
        parent: Option<InstanceId>,
        source: &Self,
        prefix: &str,
        placement: Placement3,
        mut include: impl FnMut(InstanceId, &Instance) -> bool,
    ) -> Result<AppendMap, AssemblyError> {
        validate_key(prefix)?;
        if !finite_placement(&placement) {
            return Err(AssemblyError::NonFinitePlacement);
        }
        if let Some(parent) = parent
            && self.instance(parent).is_none()
        {
            return Err(AssemblyError::UnknownInstance(parent));
        }
        let mut selected = vec![false; source.instances.len()];
        let mut used = vec![false; source.parts.len()];
        for (i, instance) in source.instances.iter().enumerate() {
            if instance.parent.is_some_and(|p| !selected[p.0 as usize]) {
                continue;
            }
            if include(InstanceId(crate::len_u32(i)), instance) {
                selected[i] = true;
                if let Some(part) = instance.part {
                    used[part.0 as usize] = true;
                }
            }
        }
        let set_selected: Vec<bool> = source
            .placement_sets
            .iter()
            .map(|set| set.parent.is_none_or(|p| selected[p.0 as usize]))
            .collect();
        for (set, keep) in source.placement_sets.iter().zip(&set_selected) {
            if *keep {
                used[set.part.0 as usize] = true;
            }
        }
        // A used part brings its lower levels of detail.
        for (i, def) in source.parts.iter().enumerate() {
            if used[i] {
                for level in def.lods.iter().skip(1) {
                    used[level.part.0 as usize] = true;
                }
            }
        }
        let part_count = used.iter().filter(|v| **v).count();
        let instance_count = selected.iter().filter(|v| **v).count();
        let set_count = set_selected.iter().filter(|v| **v).count();
        check_capacity(self.parts.len(), part_count)?;
        check_capacity(self.instances.len(), instance_count)?;
        check_capacity(self.placement_sets.len(), set_count)?;

        let mut map = AppendMap {
            parts: vec![None; source.parts.len()],
            instances: vec![None; source.instances.len()],
            placement_sets: vec![None; source.placement_sets.len()],
        };
        let mut parts = Vec::with_capacity(part_count);
        for (i, def) in source.parts.iter().enumerate() {
            if !used[i] {
                continue;
            }
            let key = format!("{prefix}-{}", def.key);
            if self.part_lookup.contains_key(&key) {
                return Err(AssemblyError::DuplicatePartKey(key));
            }
            let id = PartId(crate::len_u32(self.parts.len() + parts.len()));
            let mut copied = def.clone();
            copied.key = key;
            map.parts[i] = Some(id);
            parts.push(copied);
        }
        for copied in &mut parts {
            for level in &mut copied.lods {
                level.part = map.part(level.part).expect("lower levels are copied");
            }
        }
        let mut instances: Vec<Instance> = Vec::with_capacity(instance_count);
        let mut roots = Vec::new();
        let taken = |key: &str| self.find_sibling(parent, key).is_some();
        for (i, instance) in source.instances.iter().enumerate() {
            if !selected[i] {
                continue;
            }
            let id = InstanceId(crate::len_u32(self.instances.len() + instances.len()));
            let mut copied = instance.clone();
            copied.part = instance.part.map(|part| {
                map.part(part)
                    .expect("selected instance uses a copied part")
            });
            copied.parent = instance
                .parent
                .map(|p| map.instance(p).expect("selected ancestor was copied first"));
            copied.children.clear();
            copied.placement_sets.clear();
            if let Some(parent) = copied.parent {
                let parent = &mut instances[parent.0 as usize - self.instances.len()];
                parent.children.push(id);
            } else {
                copied.parent = parent;
                copied.key = format!("{prefix}-{}", instance.key);
                if taken(&copied.key) {
                    return Err(AssemblyError::DuplicateChildKey {
                        parent,
                        key: copied.key,
                    });
                }
                copied.placement = compose(&placement, &instance.placement);
                if !finite_placement(&copied.placement) {
                    return Err(AssemblyError::NonFinitePlacement);
                }
                roots.push(id);
            }
            map.instances[i] = Some(id);
            instances.push(copied);
        }

        let mut sets: Vec<PlacementSet> = Vec::with_capacity(set_count);
        let mut root_sets = Vec::new();
        for (i, set) in source.placement_sets.iter().enumerate() {
            if !set_selected[i] {
                continue;
            }
            let id = PlacementSetId(crate::len_u32(self.placement_sets.len() + sets.len()));
            let mut copied = set.clone();
            copied.part = map.part(set.part).expect("selected set uses a copied part");
            if let Some(source_parent) = set.parent {
                let parent = map
                    .instance(source_parent)
                    .expect("selected set parent was copied");
                copied.parent = Some(parent);
                instances[parent.0 as usize - self.instances.len()]
                    .placement_sets
                    .push(id);
            } else {
                copied.parent = parent;
                copied.key = format!("{prefix}-{}", set.key);
                // Source root instances and root sets share one key namespace,
                // so the common prefix keeps them distinct from each other.
                if taken(&copied.key) {
                    return Err(AssemblyError::DuplicateChildKey {
                        parent,
                        key: copied.key,
                    });
                }
                for local in &mut copied.placements {
                    *local = compose(&placement, local);
                    if !finite_placement(local) {
                        return Err(AssemblyError::NonFinitePlacement);
                    }
                }
                root_sets.push(id);
            }
            map.placement_sets[i] = Some(id);
            sets.push(copied);
        }

        // All fallible validation has finished. Stage only the incoming data;
        // appending does not clone an already large destination for rollback.
        for part in parts {
            let id = PartId(crate::len_u32(self.parts.len()));
            self.part_lookup.insert(part.key.clone(), id);
            self.parts.push(part);
        }
        let first_instance = self.instances.len();
        let first_set = self.placement_sets.len();
        self.instances.extend(instances);
        self.placement_sets.extend(sets);
        match parent {
            Some(parent) => {
                let parent = &mut self.instances[parent.0 as usize];
                parent.children.extend(roots);
                parent.placement_sets.extend(root_sets);
            }
            None => {
                self.roots.extend(roots);
                self.root_placement_sets.extend(root_sets);
            }
        }
        for index in first_instance..self.instances.len() {
            self.index_sibling(Sibling::Instance(InstanceId(crate::len_u32(index))));
        }
        for index in first_set..self.placement_sets.len() {
            self.index_sibling(Sibling::Set(PlacementSetId(crate::len_u32(index))));
        }
        self.content_generation += u64::from(crate::len_u32(part_count));
        Ok(map)
    }
}

fn check_capacity(existing: usize, added: usize) -> Result<(), AssemblyError> {
    if existing
        .checked_add(added)
        .is_none_or(|total| u32::try_from(total).is_err())
    {
        return Err(AssemblyError::CapacityExceeded);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
