// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The assembly model: parts, instances, identity, and material binding.
//!
//! An [`Assembly`] registers [`PartDef`] values under opaque frontend keys
//! and arranges [`Instance`] values of them in a tree. Structure is
//! append-mostly and validated at every mutation: parents must already
//! exist (cycles are impossible by construction), sibling keys are unique,
//! and slot references are checked against the part's slot table.
//!
//! Material binding is pure structure: rebinding a slot never touches
//! geometry, so compiled tessellations survive any number of binding edits.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use addressable::{AbsoluteAddress, Name};
use exedra_constructive::ir::{Placement3, Recipe};
use hashbrown::HashMap;

/// Type marker for one runtime Exedra assembly address space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssemblySpace {}

/// Stable structured identity of one placed instance.
///
/// [`Assembly::add_instance`] constructs this from validated frontend keys and
/// stores it on the instance. Use it in an exact Addressable locator; runtime
/// handles such as [`InstanceId`] are deliberately not durable identity.
pub type InstanceAddress = AbsoluteAddress<AssemblySpace>;

/// Index of a registered part within an [`Assembly`].
///
/// Handles are only meaningful for the assembly that produced them; they
/// are *not* identity across re-evaluations — part keys are.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PartId(pub u32);

/// Index of an instance within an [`Assembly`].
///
/// Handles are only meaningful for the assembly that produced them; they
/// are *not* identity across re-evaluations — instance addresses are.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InstanceId(pub u32);

/// Index into a part's slot table.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SlotIndex(pub u32);

/// The geometry source of a part.
#[derive(Clone, Debug)]
pub enum PartSource {
    /// A constructive recipe, tessellated on demand.
    Recipe(Recipe),
    /// A baked mesh, used as-is.
    Baked(exedra::Mesh),
}

/// A part definition: geometry source plus declared material slots.
///
/// Slots are declared names; regions of the tessellated geometry map to
/// slots via [`PartDef::region_slot`] entries with a part-wide default.
/// Instances bind slots to opaque material keys; parts may carry default
/// keys per slot.
#[derive(Clone, Debug)]
pub struct PartDef {
    key: String,
    source: PartSource,
    slots: Vec<String>,
    /// `(region, slot)` pairs sorted by region.
    region_slots: Vec<(u32, SlotIndex)>,
    default_slot: Option<SlotIndex>,
    /// Per-slot default material key, indexed by slot.
    default_materials: Vec<Option<String>>,
}

impl PartDef {
    /// The opaque frontend key this part was registered under.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The geometry source.
    #[must_use]
    pub fn source(&self) -> &PartSource {
        &self.source
    }

    /// Declared slot names, in slot-index order.
    #[must_use]
    pub fn slots(&self) -> &[String] {
        &self.slots
    }

    /// Looks up a slot index by name.
    #[must_use]
    pub fn slot_index(&self, name: &str) -> Option<SlotIndex> {
        self.slots
            .iter()
            .position(|s| s == name)
            .map(|i| SlotIndex(crate::len_u32(i)))
    }

    /// The slot a tessellation region resolves to: an explicit region
    /// mapping if present, else the part default slot, else none.
    #[must_use]
    pub fn region_slot(&self, region: u32) -> Option<SlotIndex> {
        match self.region_slots.binary_search_by_key(&region, |e| e.0) {
            Ok(i) => Some(self.region_slots[i].1),
            Err(_) => self.default_slot,
        }
    }

    /// All explicit region-to-slot mappings, sorted by region.
    #[must_use]
    pub fn region_slots(&self) -> &[(u32, SlotIndex)] {
        &self.region_slots
    }

    /// The part-wide default slot for unmapped regions, if set.
    #[must_use]
    pub fn default_slot(&self) -> Option<SlotIndex> {
        self.default_slot
    }

    /// Per-slot default material keys, indexed by slot.
    #[must_use]
    pub fn default_materials(&self) -> &[Option<String>] {
        &self.default_materials
    }

    /// The part-default material key for a slot, if any.
    #[must_use]
    pub fn default_material(&self, slot: SlotIndex) -> Option<&str> {
        self.default_materials
            .get(slot.0 as usize)
            .and_then(|m| m.as_deref())
    }
}

/// One placed use of a part in the instance tree.
#[derive(Clone, Debug)]
pub struct Instance {
    key: String,
    address: InstanceAddress,
    part: PartId,
    parent: Option<InstanceId>,
    placement: Placement3,
    /// `(slot, material key)` pairs sorted by slot.
    bindings: Vec<(SlotIndex, String)>,
    /// Opaque metadata pairs in insertion order (last write per key wins).
    metadata: Vec<(String, String)>,
    children: Vec<InstanceId>,
}

impl Instance {
    /// The frontend-supplied key, unique among this instance's siblings.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns this instance's stable exact Addressable address.
    #[must_use]
    pub const fn address(&self) -> &InstanceAddress {
        &self.address
    }

    /// The part this instance places.
    #[must_use]
    pub fn part(&self) -> PartId {
        self.part
    }

    /// The parent instance, or `None` for a root.
    #[must_use]
    pub fn parent(&self) -> Option<InstanceId> {
        self.parent
    }

    /// The placement relative to the parent (or to the world for roots).
    #[must_use]
    pub fn placement(&self) -> &Placement3 {
        &self.placement
    }

    /// Slot bindings sorted by slot index.
    #[must_use]
    pub fn bindings(&self) -> &[(SlotIndex, String)] {
        &self.bindings
    }

    /// The material key this instance binds to `slot`, if any.
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

    /// Child instances in insertion order.
    #[must_use]
    pub fn children(&self) -> &[InstanceId] {
        &self.children
    }
}

/// Typed assembly mutation/lookup failure.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AssemblyError {
    /// A part with this key is already registered.
    DuplicatePartKey(String),
    /// The part handle does not refer to a registered part.
    UnknownPart(PartId),
    /// The instance handle does not refer to an existing instance.
    UnknownInstance(InstanceId),
    /// The parent already has a child with this key.
    DuplicateChildKey {
        /// The parent (or `None` for the root level).
        parent: Option<InstanceId>,
        /// The offending key.
        key: String,
    },
    /// Keys must be valid Addressable name segments.
    InvalidKey(String),
    /// The part does not declare a slot with this name.
    UnknownSlot {
        /// The part whose slot table was consulted.
        part: PartId,
        /// The unknown slot name.
        slot: String,
    },
    /// A slot name appears twice in a declaration.
    DuplicateSlot(String),
    /// The placement contains non-finite values.
    NonFinitePlacement,
}

impl core::fmt::Display for AssemblyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DuplicatePartKey(key) => write!(f, "part key {key:?} already registered"),
            Self::UnknownPart(id) => write!(f, "unknown part {id:?}"),
            Self::UnknownInstance(id) => write!(f, "unknown instance {id:?}"),
            Self::DuplicateChildKey { parent, key } => {
                write!(f, "parent {parent:?} already has a child keyed {key:?}")
            }
            Self::InvalidKey(key) => {
                write!(f, "key {key:?} is not a valid Addressable name segment")
            }
            Self::UnknownSlot { part, slot } => {
                write!(f, "part {part:?} declares no slot named {slot:?}")
            }
            Self::DuplicateSlot(slot) => write!(f, "slot {slot:?} declared twice"),
            Self::NonFinitePlacement => write!(f, "placement contains non-finite values"),
        }
    }
}

impl core::error::Error for AssemblyError {}

/// A validated scene: parts plus an instance tree.
///
/// All iteration orders are deterministic (registration/insertion order);
/// the internal key lookup map is never iterated.
#[derive(Clone, Debug, Default)]
pub struct Assembly {
    parts: Vec<PartDef>,
    part_lookup: HashMap<String, PartId>,
    instances: Vec<Instance>,
    instance_lookup: HashMap<InstanceAddress, InstanceId>,
    roots: Vec<InstanceId>,
    /// Monotonic content generation: bumped by every part-content change
    /// (not by binding or metadata edits).
    content_generation: u64,
}

impl Assembly {
    /// Creates an empty assembly.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a recipe-backed part; slots come from the recipe's slot
    /// table. A recipe with exactly one slot uses that slot as its default.
    ///
    /// # Errors
    ///
    /// Fails on a duplicate or invalid key.
    pub fn add_recipe_part(&mut self, key: &str, recipe: Recipe) -> Result<PartId, AssemblyError> {
        let slots = recipe.slots().to_vec();
        let default_slot = (slots.len() == 1).then_some(SlotIndex(0));
        self.add_part_inner(key, PartSource::Recipe(recipe), slots, default_slot)
    }

    /// Registers a baked-mesh part with explicitly declared slots.
    ///
    /// # Errors
    ///
    /// Fails on a duplicate/invalid key or duplicate slot names.
    pub fn add_baked_part(
        &mut self,
        key: &str,
        mesh: exedra::Mesh,
        slots: &[&str],
    ) -> Result<PartId, AssemblyError> {
        let mut seen = Vec::new();
        for slot in slots {
            if seen.contains(slot) {
                return Err(AssemblyError::DuplicateSlot((*slot).to_string()));
            }
            seen.push(slot);
        }
        let slots = slots.iter().map(|s| (*s).to_string()).collect();
        self.add_part_inner(key, PartSource::Baked(mesh), slots, None)
    }

    fn add_part_inner(
        &mut self,
        key: &str,
        source: PartSource,
        slots: Vec<String>,
        default_slot: Option<SlotIndex>,
    ) -> Result<PartId, AssemblyError> {
        if key.is_empty() {
            return Err(AssemblyError::InvalidKey(key.to_string()));
        }
        if self.part_lookup.contains_key(key) {
            return Err(AssemblyError::DuplicatePartKey(key.to_string()));
        }
        let id = PartId(crate::len_u32(self.parts.len()));
        let default_materials = alloc::vec![None; slots.len()];
        self.parts.push(PartDef {
            key: key.to_string(),
            source,
            slots,
            region_slots: Vec::new(),
            default_slot,
            default_materials,
        });
        self.part_lookup.insert(key.to_string(), id);
        self.content_generation += 1;
        Ok(id)
    }

    /// Replaces a part's geometry source, keeping its key and slot
    /// bindings. Slots are re-derived for recipe sources; explicit baked
    /// slot tables are preserved when the name sets match, otherwise the
    /// new recipe's table wins and stale bindings resolve to nothing.
    ///
    /// This is the content edit that invalidates compiled caches (the
    /// compile layer subscribes via its parts channel).
    ///
    /// # Errors
    ///
    /// Fails when `part` is unknown.
    pub fn replace_part_source(
        &mut self,
        part: PartId,
        source: PartSource,
    ) -> Result<(), AssemblyError> {
        let def = self
            .parts
            .get_mut(part.0 as usize)
            .ok_or(AssemblyError::UnknownPart(part))?;
        if let PartSource::Recipe(recipe) = &source {
            def.slots = recipe.slots().to_vec();
            def.default_materials.resize(def.slots.len(), None);
        }
        def.source = source;
        self.content_generation += 1;
        Ok(())
    }

    /// The current content generation (bumped by part-content changes
    /// only, never by binding or metadata edits).
    #[must_use]
    pub fn content_generation(&self) -> u64 {
        self.content_generation
    }

    /// Looks up a part definition.
    #[must_use]
    pub fn part(&self, id: PartId) -> Option<&PartDef> {
        self.parts.get(id.0 as usize)
    }

    /// Looks up a part by its registration key.
    #[must_use]
    pub fn part_by_key(&self, key: &str) -> Option<PartId> {
        self.part_lookup.get(key).copied()
    }

    /// All parts in registration order.
    #[must_use]
    pub fn parts(&self) -> &[PartDef] {
        &self.parts
    }

    /// Maps a tessellation region of a part to a slot name.
    ///
    /// # Errors
    ///
    /// Fails when the part or slot is unknown.
    pub fn bind_region_slot(
        &mut self,
        part: PartId,
        region: u32,
        slot: &str,
    ) -> Result<(), AssemblyError> {
        let (def, index) = self.part_slot_mut(part, slot)?;
        match def.region_slots.binary_search_by_key(&region, |e| e.0) {
            Ok(i) => def.region_slots[i].1 = index,
            Err(i) => def.region_slots.insert(i, (region, index)),
        }
        Ok(())
    }

    /// Sets the part-wide default slot for regions without an explicit
    /// mapping.
    ///
    /// # Errors
    ///
    /// Fails when the part or slot is unknown.
    pub fn set_default_slot(&mut self, part: PartId, slot: &str) -> Result<(), AssemblyError> {
        let (def, index) = self.part_slot_mut(part, slot)?;
        def.default_slot = Some(index);
        Ok(())
    }

    /// Sets a part-default material key for a slot (instances may
    /// override).
    ///
    /// # Errors
    ///
    /// Fails when the part or slot is unknown.
    pub fn set_part_material(
        &mut self,
        part: PartId,
        slot: &str,
        material: &str,
    ) -> Result<(), AssemblyError> {
        let (def, index) = self.part_slot_mut(part, slot)?;
        def.default_materials[index.0 as usize] = Some(material.to_string());
        Ok(())
    }

    fn part_slot_mut(
        &mut self,
        part: PartId,
        slot: &str,
    ) -> Result<(&mut PartDef, SlotIndex), AssemblyError> {
        let def = self
            .parts
            .get_mut(part.0 as usize)
            .ok_or(AssemblyError::UnknownPart(part))?;
        let index = def
            .slot_index(slot)
            .ok_or_else(|| AssemblyError::UnknownSlot {
                part,
                slot: slot.to_string(),
            })?;
        Ok((def, index))
    }

    /// Adds an instance of `part` under `parent` (or as a root when
    /// `parent` is `None`).
    ///
    /// Cycles are impossible by construction: the parent must already
    /// exist, and instances are never re-parented.
    ///
    /// # Errors
    ///
    /// Fails on unknown part/parent, invalid or duplicate sibling key, or
    /// a non-finite placement.
    pub fn add_instance(
        &mut self,
        parent: Option<InstanceId>,
        key: &str,
        part: PartId,
        placement: Placement3,
    ) -> Result<InstanceId, AssemblyError> {
        let name = Name::new(key).map_err(|_| AssemblyError::InvalidKey(key.to_string()))?;
        if self.parts.get(part.0 as usize).is_none() {
            return Err(AssemblyError::UnknownPart(part));
        }
        if !placement
            .rows
            .iter()
            .all(|r| r.iter().all(|v| v.is_finite()))
        {
            return Err(AssemblyError::NonFinitePlacement);
        }
        let address = match parent {
            Some(parent) => {
                let parent = self
                    .instances
                    .get(parent.0 as usize)
                    .ok_or(AssemblyError::UnknownInstance(parent))?;
                AbsoluteAddress::from_names(parent.address.segments().iter().cloned().chain([name]))
            }
            None => AbsoluteAddress::from_names([name]),
        };
        if self.instance_lookup.contains_key(&address) {
            return Err(AssemblyError::DuplicateChildKey {
                parent,
                key: key.to_string(),
            });
        }
        let id = InstanceId(crate::len_u32(self.instances.len()));
        self.instances.push(Instance {
            key: key.to_string(),
            address: address.clone(),
            part,
            parent,
            placement,
            bindings: Vec::new(),
            metadata: Vec::new(),
            children: Vec::new(),
        });
        match parent {
            Some(p) => self.instances[p.0 as usize].children.push(id),
            None => self.roots.push(id),
        }
        self.instance_lookup.insert(address, id);
        Ok(id)
    }

    /// Binds (or rebinds) a slot of an instance to an opaque material key.
    ///
    /// Pure structure: never touches geometry or compiled caches.
    ///
    /// # Errors
    ///
    /// Fails when the instance is unknown or its part declares no such
    /// slot.
    pub fn bind_material(
        &mut self,
        instance: InstanceId,
        slot: &str,
        material: &str,
    ) -> Result<(), AssemblyError> {
        let part = self
            .instances
            .get(instance.0 as usize)
            .ok_or(AssemblyError::UnknownInstance(instance))?
            .part;
        let index = self.parts[part.0 as usize]
            .slot_index(slot)
            .ok_or_else(|| AssemblyError::UnknownSlot {
                part,
                slot: slot.to_string(),
            })?;
        let bindings = &mut self.instances[instance.0 as usize].bindings;
        match bindings.binary_search_by_key(&index, |binding| binding.0) {
            Ok(binding) => bindings[binding].1 = material.to_string(),
            Err(binding) => bindings.insert(binding, (index, material.to_string())),
        }
        Ok(())
    }

    /// Sets an opaque metadata entry on an instance (last write wins).
    ///
    /// # Errors
    ///
    /// Fails when the instance is unknown.
    pub fn set_metadata(
        &mut self,
        instance: InstanceId,
        key: &str,
        value: &str,
    ) -> Result<(), AssemblyError> {
        let inst = self
            .instances
            .get_mut(instance.0 as usize)
            .ok_or(AssemblyError::UnknownInstance(instance))?;
        if let Some(entry) = inst.metadata.iter_mut().find(|(k, _)| k == key) {
            entry.1 = value.to_string();
        } else {
            inst.metadata.push((key.to_string(), value.to_string()));
        }
        Ok(())
    }

    /// Looks up an instance.
    #[must_use]
    pub fn instance(&self, id: InstanceId) -> Option<&Instance> {
        self.instances.get(id.0 as usize)
    }

    /// All instances in insertion order.
    #[must_use]
    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    /// All instance handles and values in insertion order.
    ///
    /// This is the ID-bearing counterpart to [`Self::instances`]. It avoids
    /// reconstructing opaque handles from slice indices at call sites.
    pub fn instances_with_ids(
        &self,
    ) -> impl DoubleEndedIterator<Item = (InstanceId, &Instance)> + ExactSizeIterator {
        self.instances
            .iter()
            .enumerate()
            .map(|(index, instance)| (InstanceId(crate::len_u32(index)), instance))
    }

    /// Root instances in insertion order.
    #[must_use]
    pub fn roots(&self) -> &[InstanceId] {
        &self.roots
    }

    /// Looks up a runtime instance handle by its stable exact address.
    ///
    /// This uses the assembly's address index rather than walking the tree or
    /// scanning instances.
    #[must_use]
    pub fn instance_by_address(&self, address: &InstanceAddress) -> Option<InstanceId> {
        self.instance_lookup.get(address).copied()
    }

    pub(crate) fn effective_material(&self, instance: InstanceId, slot: SlotIndex) -> Option<&str> {
        let inst = self.instances.get(instance.0 as usize)?;
        if let Some(key) = inst.binding(slot) {
            return Some(key);
        }
        self.parts[inst.part.0 as usize].default_material(slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_constructive::builders;
    use exedra_constructive::ir::{CapMode, NodeKind, RecipeBuilder};

    fn test_recipe() -> Recipe {
        let mut b = RecipeBuilder::new();
        let front = b.material_slot("front");
        let body = b.material_slot("body");
        let profile = b.add_profile(builders::rect(40.0, 20.0).unwrap());
        let _ = body;
        let node = b
            .with_material(front)
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 10.0,
                caps: CapMode::Both,
            })
            .unwrap();
        b.finish(node).unwrap()
    }

    fn single_slot_recipe() -> Recipe {
        let mut b = RecipeBuilder::new();
        let surface = b.material_slot("surface");
        let profile = b.add_profile(builders::rect(40.0, 20.0).unwrap());
        let node = b
            .with_material(surface)
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 10.0,
                caps: CapMode::Both,
            })
            .unwrap();
        b.finish(node).unwrap()
    }

    #[test]
    fn part_registration_and_lookup() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", test_recipe()).unwrap();
        assert_eq!(asm.part_by_key("panel"), Some(part));
        assert_eq!(asm.part(part).unwrap().slots(), &["front", "body"]);
        assert_eq!(asm.part(part).unwrap().default_slot(), None);
        assert_eq!(
            asm.add_recipe_part("panel", test_recipe()),
            Err(AssemblyError::DuplicatePartKey("panel".into()))
        );
        assert_eq!(
            asm.add_recipe_part("", test_recipe()),
            Err(AssemblyError::InvalidKey(String::new()))
        );
    }

    #[test]
    fn single_slot_recipe_uses_its_only_slot_as_default() {
        let mut asm = Assembly::new();
        let part = asm
            .add_recipe_part("single-slot", single_slot_recipe())
            .unwrap();
        let def = asm.part(part).unwrap();
        assert_eq!(def.default_slot(), def.slot_index("surface"));
        assert_eq!(def.region_slot(42), def.slot_index("surface"));
    }

    #[test]
    fn instance_addresses_are_stored_and_indexed() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", test_recipe()).unwrap();
        let root = asm
            .add_instance(None, "frame", part, Placement3::IDENTITY)
            .unwrap();
        let crossbar = asm
            .add_instance(
                Some(root),
                "crossbar-1",
                part,
                Placement3::translate(0.0, 0.0, 5.0),
            )
            .unwrap();
        let address = InstanceAddress::parse("/frame/crossbar-1").unwrap();
        assert_eq!(asm.instance(crossbar).unwrap().address(), &address);
        assert_eq!(asm.instance_by_address(&address), Some(crossbar));
        assert_eq!(
            asm.instances_with_ids()
                .map(|(id, instance)| (id, instance.key()))
                .collect::<Vec<_>>(),
            alloc::vec![(root, "frame"), (crossbar, "crossbar-1")]
        );
        assert_eq!(asm.instances_with_ids().len(), asm.instances().len());
        assert_eq!(
            asm.instance_by_address(&InstanceAddress::parse("/frame/missing").unwrap()),
            None
        );
    }

    #[test]
    fn sibling_keys_unique_per_parent() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", test_recipe()).unwrap();
        let root = asm
            .add_instance(None, "frame", part, Placement3::IDENTITY)
            .unwrap();
        asm.add_instance(Some(root), "panel", part, Placement3::IDENTITY)
            .unwrap();
        assert_eq!(
            asm.add_instance(Some(root), "panel", part, Placement3::IDENTITY),
            Err(AssemblyError::DuplicateChildKey {
                parent: Some(root),
                key: "panel".into()
            })
        );
        // Same key under a different parent is fine.
        let other = asm
            .add_instance(None, "island", part, Placement3::IDENTITY)
            .unwrap();
        assert!(
            asm.add_instance(Some(other), "panel", part, Placement3::IDENTITY)
                .is_ok()
        );
        // Separator and empty keys rejected.
        assert_eq!(
            asm.add_instance(None, "a/b", part, Placement3::IDENTITY),
            Err(AssemblyError::InvalidKey("a/b".into()))
        );
        assert_eq!(
            asm.add_instance(None, "..", part, Placement3::IDENTITY),
            Err(AssemblyError::InvalidKey("..".into()))
        );
    }

    #[test]
    fn validation_errors_are_typed() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", test_recipe()).unwrap();
        assert_eq!(
            asm.add_instance(None, "x", PartId(9), Placement3::IDENTITY),
            Err(AssemblyError::UnknownPart(PartId(9)))
        );
        assert_eq!(
            asm.add_instance(Some(InstanceId(3)), "x", part, Placement3::IDENTITY),
            Err(AssemblyError::UnknownInstance(InstanceId(3)))
        );
        let bad = Placement3 {
            rows: [[f64::NAN; 4]; 3],
        };
        assert_eq!(
            asm.add_instance(None, "x", part, bad),
            Err(AssemblyError::NonFinitePlacement)
        );
    }

    #[test]
    fn material_binding_resolves_with_inheritance() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", test_recipe()).unwrap();
        asm.set_part_material(part, "front", "oak").unwrap();
        let a = asm
            .add_instance(None, "a", part, Placement3::IDENTITY)
            .unwrap();
        let b = asm
            .add_instance(None, "b", part, Placement3::IDENTITY)
            .unwrap();
        asm.bind_material(b, "front", "walnut").unwrap();
        let front = asm.part(part).unwrap().slot_index("front").unwrap();
        assert_eq!(asm.effective_material(a, front), Some("oak"));
        assert_eq!(asm.effective_material(b, front), Some("walnut"));
        // Rebinding replaces.
        asm.bind_material(b, "front", "ash").unwrap();
        assert_eq!(asm.effective_material(b, front), Some("ash"));
        assert_eq!(
            asm.bind_material(b, "nope", "x"),
            Err(AssemblyError::UnknownSlot {
                part,
                slot: "nope".into()
            })
        );
    }

    #[test]
    fn region_slot_mapping_with_default() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", test_recipe()).unwrap();
        asm.set_default_slot(part, "body").unwrap();
        asm.bind_region_slot(part, 0, "front").unwrap();
        let def = asm.part(part).unwrap();
        let front = def.slot_index("front").unwrap();
        let body = def.slot_index("body").unwrap();
        assert_eq!(def.region_slot(0), Some(front));
        assert_eq!(def.region_slot(7), Some(body));
    }

    #[test]
    fn binding_edits_do_not_bump_content_generation() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", test_recipe()).unwrap();
        let a = asm
            .add_instance(None, "a", part, Placement3::IDENTITY)
            .unwrap();
        let generation = asm.content_generation();
        asm.bind_material(a, "front", "oak").unwrap();
        asm.set_metadata(a, "variant", "A-1").unwrap();
        assert_eq!(asm.content_generation(), generation);
        asm.replace_part_source(part, PartSource::Recipe(test_recipe()))
            .unwrap();
        assert!(asm.content_generation() > generation);
    }

    #[test]
    fn metadata_last_write_wins() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", test_recipe()).unwrap();
        let a = asm
            .add_instance(None, "a", part, Placement3::IDENTITY)
            .unwrap();
        asm.set_metadata(a, "variant", "A-1").unwrap();
        asm.set_metadata(a, "note", "demo").unwrap();
        asm.set_metadata(a, "variant", "A-2").unwrap();
        assert_eq!(
            asm.instance(a).unwrap().metadata(),
            &[
                ("variant".into(), "A-2".into()),
                ("note".into(), "demo".into())
            ]
        );
    }
}
