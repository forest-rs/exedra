// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Placement sets: one part placed many times without per-placement nodes.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::siblings::Sibling;
use super::{Assembly, AssemblyError, InstanceId, InstancePath, PartId, SlotIndex};
use exedra_constructive::ir::Placement3;

/// Index of a placement set within an [`Assembly`].
///
/// Handles are only meaningful for the assembly that produced them; set paths
/// and placement indices are the identity.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlacementSetId(pub u32);

/// One part placed many times under one parent, stored as arrays.
///
/// A set is the scalable counterpart of many sibling instances of one part:
/// it holds one key, one binding table and one metadata table, and a dense
/// array of local placements. Optional per-placement seeds and tints travel
/// alongside for consumers that vary appearance per placement. Placement `i`
/// is addressed as [`PlacementPath`] `parent/key#i`; it has no node, key
/// string or binding table of its own.
#[derive(Clone, Debug)]
pub struct PlacementSet {
    pub(crate) key: String,
    pub(crate) part: PartId,
    pub(crate) parent: Option<InstanceId>,
    pub(crate) placements: Vec<Placement3>,
    pub(crate) seeds: Option<Arc<[u64]>>,
    pub(crate) tints: Option<Arc<[[f32; 4]]>>,
    /// `(slot, material key)` pairs sorted by slot, shared by every placement.
    pub(crate) bindings: Vec<(SlotIndex, String)>,
    /// Opaque metadata pairs in insertion order (last write per key wins).
    pub(crate) metadata: Vec<(String, String)>,
}

impl PlacementSet {
    /// The frontend-supplied key, unique among the parent's children.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The part every placement places.
    #[must_use]
    pub fn part(&self) -> PartId {
        self.part
    }

    /// The parent instance, or `None` for a root-level set.
    #[must_use]
    pub fn parent(&self) -> Option<InstanceId> {
        self.parent
    }

    /// Placements relative to the parent (or the world for root-level sets).
    #[must_use]
    pub fn placements(&self) -> &[Placement3] {
        &self.placements
    }

    /// Number of placements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.placements.len()
    }

    /// True when the set places nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    /// Per-placement seeds, parallel to [`Self::placements`], if assigned.
    #[must_use]
    pub fn seeds(&self) -> Option<&[u64]> {
        self.seeds.as_deref()
    }

    /// Per-placement linear RGBA tints, parallel to [`Self::placements`], if
    /// assigned.
    #[must_use]
    pub fn tints(&self) -> Option<&[[f32; 4]]> {
        self.tints.as_deref()
    }

    /// Slot bindings shared by every placement, sorted by slot index.
    #[must_use]
    pub fn bindings(&self) -> &[(SlotIndex, String)] {
        &self.bindings
    }

    /// The material key this set binds to `slot`, if any.
    #[must_use]
    pub fn binding(&self, slot: SlotIndex) -> Option<&str> {
        match self.bindings.binary_search_by_key(&slot, |b| b.0) {
            Ok(i) => Some(&self.bindings[i].1),
            Err(_) => None,
        }
    }

    /// Opaque metadata pairs in insertion order.
    #[must_use]
    pub fn metadata(&self) -> &[(String, String)] {
        &self.metadata
    }
}

/// The stable identity of one placement in a set: the set's path and the
/// placement's index.
///
/// Keys never contain `/` or `#`, so the `Display` form `root/set#12` is
/// unambiguous.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlacementPath {
    /// The path of the set: its parent's path followed by the set key.
    pub set: InstancePath,
    /// The placement's index within the set.
    pub index: u32,
}

impl core::fmt::Display for PlacementPath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}#{}", self.set, self.index)
    }
}

impl Assembly {
    /// Adds a placement set: `part` placed at every entry of `placements`
    /// under `parent` (or at the root when `parent` is `None`).
    ///
    /// The set shares its parent's key namespace with instances. It adds no
    /// per-placement node, so large scatters cost one record plus the
    /// placement array.
    ///
    /// # Errors
    ///
    /// Fails on unknown part/parent, invalid or duplicate sibling key, an
    /// empty placement list, or a non-finite placement.
    pub fn add_placement_set(
        &mut self,
        parent: Option<InstanceId>,
        key: &str,
        part: PartId,
        placements: Vec<Placement3>,
    ) -> Result<PlacementSetId, AssemblyError> {
        super::validate_key(key)?;
        if self.parts.get(part.0 as usize).is_none() {
            return Err(AssemblyError::UnknownPart(part));
        }
        if let Some(parent) = parent
            && self.instances.get(parent.0 as usize).is_none()
        {
            return Err(AssemblyError::UnknownInstance(parent));
        }
        if placements.is_empty() {
            return Err(AssemblyError::EmptyPlacementSet);
        }
        if !placements.iter().all(super::finite_placement) {
            return Err(AssemblyError::NonFinitePlacement);
        }
        if self.find_sibling(parent, key).is_some() {
            return Err(AssemblyError::DuplicateChildKey {
                parent,
                key: key.to_string(),
            });
        }
        let id = PlacementSetId(crate::len_u32(self.placement_sets.len()));
        self.placement_sets.push(PlacementSet {
            key: key.to_string(),
            part,
            parent,
            placements,
            seeds: None,
            tints: None,
            bindings: Vec::new(),
            metadata: Vec::new(),
        });
        match parent {
            Some(p) => self.instances[p.0 as usize].placement_sets.push(id),
            None => self.root_placement_sets.push(id),
        }
        self.index_sibling(Sibling::Set(id));
        Ok(id)
    }

    /// Assigns per-placement seeds, one per placement.
    ///
    /// # Errors
    ///
    /// Fails on an unknown set or a length that differs from the placement
    /// count.
    pub fn set_placement_seeds(
        &mut self,
        set: PlacementSetId,
        seeds: Vec<u64>,
    ) -> Result<(), AssemblyError> {
        let entry = self.placement_set_mut(set)?;
        check_length(entry.placements.len(), seeds.len())?;
        entry.seeds = Some(seeds.into());
        Ok(())
    }

    /// Assigns per-placement linear RGBA tints, one per placement.
    ///
    /// # Errors
    ///
    /// Fails on an unknown set, a length that differs from the placement
    /// count, or a non-finite component.
    pub fn set_placement_tints(
        &mut self,
        set: PlacementSetId,
        tints: Vec<[f32; 4]>,
    ) -> Result<(), AssemblyError> {
        let entry = self.placement_set_mut(set)?;
        check_length(entry.placements.len(), tints.len())?;
        if !tints.iter().flatten().all(|v| v.is_finite()) {
            return Err(AssemblyError::NonFiniteTint);
        }
        entry.tints = Some(tints.into());
        Ok(())
    }

    /// Binds (or rebinds) a slot of every placement in a set to an opaque
    /// material key.
    ///
    /// # Errors
    ///
    /// Fails when the set is unknown or its part declares no such slot.
    pub fn bind_placement_material(
        &mut self,
        set: PlacementSetId,
        slot: &str,
        material: &str,
    ) -> Result<(), AssemblyError> {
        let part = self.placement_set_mut(set)?.part;
        let index = self.parts[part.0 as usize]
            .slot_index(slot)
            .ok_or_else(|| AssemblyError::UnknownSlot {
                part,
                slot: slot.to_string(),
            })?;
        let bindings = &mut self.placement_sets[set.0 as usize].bindings;
        match bindings.binary_search_by_key(&index, |b| b.0) {
            Ok(i) => bindings[i].1 = material.to_string(),
            Err(i) => bindings.insert(i, (index, material.to_string())),
        }
        Ok(())
    }

    /// Sets an opaque metadata entry on a set (last write wins).
    ///
    /// # Errors
    ///
    /// Fails when the set is unknown.
    pub fn set_placement_set_metadata(
        &mut self,
        set: PlacementSetId,
        key: &str,
        value: &str,
    ) -> Result<(), AssemblyError> {
        let entry = self.placement_set_mut(set)?;
        if let Some(pair) = entry.metadata.iter_mut().find(|(k, _)| k == key) {
            pair.1 = value.to_string();
        } else {
            entry.metadata.push((key.to_string(), value.to_string()));
        }
        Ok(())
    }

    /// Looks up a placement set.
    #[must_use]
    pub fn placement_set(&self, id: PlacementSetId) -> Option<&PlacementSet> {
        self.placement_sets.get(id.0 as usize)
    }

    /// All placement sets in insertion order.
    #[must_use]
    pub fn placement_sets(&self) -> &[PlacementSet] {
        &self.placement_sets
    }

    /// Root-level placement sets in insertion order.
    #[must_use]
    pub fn root_placement_sets(&self) -> &[PlacementSetId] {
        &self.root_placement_sets
    }

    /// The identity path of a set: its parent's path followed by its key.
    #[must_use]
    pub fn placement_set_path(&self, id: PlacementSetId) -> Option<InstancePath> {
        let set = self.placement_sets.get(id.0 as usize)?;
        let mut path = match set.parent {
            Some(parent) => self.path_of(parent)?,
            None => InstancePath(Vec::new()),
        };
        path.0.push(set.key.clone());
        Some(path)
    }

    /// Resolves a placement path to its set and in-range index.
    #[must_use]
    pub fn resolve_placement(&self, path: &PlacementPath) -> Option<(PlacementSetId, u32)> {
        let (key, parents) = path.set.0.split_last()?;
        let parent = if parents.is_empty() {
            None
        } else {
            Some(self.resolve_path(&InstancePath(parents.to_vec()))?)
        };
        match self.find_sibling(parent, key)? {
            Sibling::Set(id)
                if (path.index as usize) < self.placement_sets[id.0 as usize].len() =>
            {
                Some((id, path.index))
            }
            _ => None,
        }
    }

    /// The material key a slot of a set resolves to: the set binding when
    /// present, else the part default.
    #[must_use]
    pub fn resolved_placement_material(
        &self,
        set: PlacementSetId,
        slot: SlotIndex,
    ) -> Option<&str> {
        let entry = self.placement_sets.get(set.0 as usize)?;
        entry
            .binding(slot)
            .or_else(|| self.parts[entry.part.0 as usize].default_material(slot))
    }

    fn placement_set_mut(
        &mut self,
        set: PlacementSetId,
    ) -> Result<&mut PlacementSet, AssemblyError> {
        self.placement_sets
            .get_mut(set.0 as usize)
            .ok_or(AssemblyError::UnknownPlacementSet(set))
    }
}

fn check_length(expected: usize, found: usize) -> Result<(), AssemblyError> {
    if expected == found {
        Ok(())
    } else {
        Err(AssemblyError::PlacementDataLength { expected, found })
    }
}

#[cfg(test)]
mod tests;
