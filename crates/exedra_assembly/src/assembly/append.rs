// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{format, vec};
use hashbrown::HashSet;

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
}

impl Assembly {
    /// Appends the source instance trees under a key prefix and root placement.
    ///
    /// Copies each referenced source part once, preserving sharing, slot order,
    /// region mappings and default materials. Instance bindings and metadata are
    /// copied unchanged. Unreferenced part definitions are omitted. Existing
    /// destination parts are not interned or reused by this operation.
    ///
    /// Part keys and root-instance keys become `"{prefix}-{source_key}"`;
    /// descendant keys and local placements retain their source values. The
    /// supplied placement is composed onto source roots only. The prefix must
    /// be nonempty and contain no `/`. Returned handles associate source records
    /// with their copies without relying on insertion order or derived names.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid prefix, a destination key collision,
    /// a nonfinite supplied or composed root placement, or exhausted handles.
    /// On any returned error, the destination remains unchanged.
    pub fn append(
        &mut self,
        source: &Self,
        prefix: &str,
        placement: Placement3,
    ) -> Result<AppendMap, AssemblyError> {
        self.append_selected(source, prefix, placement, |_, _| true)
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
        source: &Self,
        prefix: &str,
        placement: Placement3,
        mut include: impl FnMut(InstanceId, &Instance) -> bool,
    ) -> Result<AppendMap, AssemblyError> {
        if prefix.is_empty() || prefix.contains('/') {
            return Err(AssemblyError::InvalidKey(prefix.to_string()));
        }
        if !finite_placement(&placement) {
            return Err(AssemblyError::NonFinitePlacement);
        }
        let mut selected = vec![false; source.instances.len()];
        let mut used = vec![false; source.parts.len()];
        for (i, instance) in source.instances.iter().enumerate() {
            if instance.parent.is_some_and(|p| !selected[p.0 as usize]) {
                continue;
            }
            if include(InstanceId(crate::len_u32(i)), instance) {
                selected[i] = true;
                used[instance.part.0 as usize] = true;
            }
        }
        let part_count = used.iter().filter(|v| **v).count();
        let instance_count = selected.iter().filter(|v| **v).count();
        check_capacity(self.parts.len(), part_count)?;
        check_capacity(self.instances.len(), instance_count)?;

        let mut map = AppendMap {
            parts: vec![None; source.parts.len()],
            instances: vec![None; source.instances.len()],
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
        let mut instances: Vec<Instance> = Vec::with_capacity(instance_count);
        let mut roots = Vec::new();
        let root_keys: HashSet<&str> = self
            .roots
            .iter()
            .map(|root| self.instances[root.0 as usize].key.as_str())
            .collect();
        for (i, instance) in source.instances.iter().enumerate() {
            if !selected[i] {
                continue;
            }
            let id = InstanceId(crate::len_u32(self.instances.len() + instances.len()));
            let mut copied = instance.clone();
            copied.part = map
                .part(instance.part)
                .expect("selected instance uses a copied part");
            copied.parent = instance
                .parent
                .map(|p| map.instance(p).expect("selected ancestor was copied first"));
            copied.children.clear();
            if let Some(parent) = copied.parent {
                let parent = &mut instances[parent.0 as usize - self.instances.len()];
                parent.children.push(id);
            } else {
                copied.key = format!("{prefix}-{}", instance.key);
                if root_keys.contains(copied.key.as_str()) {
                    return Err(AssemblyError::DuplicateChildKey {
                        parent: None,
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
        drop(root_keys);

        // All fallible validation has finished. Stage only the incoming data;
        // appending does not clone an already large destination for rollback.
        for part in parts {
            let id = PartId(crate::len_u32(self.parts.len()));
            self.part_lookup.insert(part.key.clone(), id);
            self.parts.push(part);
        }
        self.instances.extend(instances);
        self.roots.extend(roots);
        self.content_generation += u64::from(crate::len_u32(part_count));
        Ok(map)
    }
}

fn finite_placement(placement: &Placement3) -> bool {
    placement.rows.iter().flatten().all(|v| v.is_finite())
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
