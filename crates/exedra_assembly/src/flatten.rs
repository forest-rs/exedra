// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The renderer seam: flattening an assembly into a [`RenderList`].
//!
//! [`flatten`] walks the instance tree depth-first in insertion order,
//! composes f64 world placements down the tree, and resolves every
//! range's material key from its authored slot or explicit part fallback
//! through the binding chain (instance binding wins
//! over part default). The result is a flat, deterministic list that
//! renderers and exporters consume; it carries no geometry of its own —
//! items reference compiled bodies in the [`CompiledParts`] set.
//!
//! Instances become [`RenderItem`]s. [`PlacementSet`](crate::PlacementSet)s
//! become [`RenderBatch`]es: one per compiled body, carrying every world
//! placement and resolving materials once, so a scatter of thousands of
//! placements costs arrays rather than per-placement items. Consumers must
//! handle both lists.
//!
//! Every placed body records world-space bounds chosen by [`BoundsPolicy`]:
//! by default the part-local box transformed by the placement, which costs
//! O(bodies) per placement instead of O(vertices).
//!
//! Parts with a level-of-detail chain ([`crate::LodLevel`]) emit their finest
//! level by default, tagged with a [`LodTag`], so consumers without LOD
//! support draw full detail. [`LodEmission::AllLevels`] emits every level,
//! each tagged with its coverage range, for renderers that select or
//! crossfade levels.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use exedra_constructive::evaluate::Aabb3;
use exedra_constructive::ir::Placement3;

use crate::assembly::{
    Assembly, InstanceId, InstancePath, LodLevel, PartId, PlacementSetId, SlotIndex,
};
use crate::compile::{CompiledBody, CompiledParts};

/// One index range of a rendered body with its resolved material key.
/// Several ranges may share a region ID when their authored slots differ.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedRegion {
    /// The `FACE_REGION` value.
    pub region: u32,
    /// First index (multiple of 3) in the compiled body's index buffer.
    pub start: u32,
    /// Number of indices (multiple of 3).
    pub count: u32,
    /// The material key this region resolves to, if any binding applies.
    pub material: Option<String>,
}

/// One drawable: a compiled body of a part placed in world space.
#[derive(Clone, Debug)]
pub struct RenderItem {
    /// Stable identity of the owning instance.
    pub path: InstancePath,
    /// The owning instance handle (valid for the source assembly only).
    pub instance: InstanceId,
    /// World placement composed down the tree (f64).
    pub world: Placement3,
    /// The part whose compiled entry holds this body's buffers. Under
    /// [`LodEmission::AllLevels`] this is the level's part, which differs from
    /// the placed part for lower levels.
    pub part: PartId,
    /// Index into the compiled part's body list.
    pub body: u32,
    /// World-space bounds of this body under the flatten's [`BoundsPolicy`].
    /// An empty body has no bounds.
    pub world_bounds: Option<Aabb3>,
    /// Index ranges with resolved material keys, in compiled region/slot order
    /// and covering the whole index buffer. Region IDs need not be unique.
    pub regions: Vec<ResolvedRegion>,
    /// The level of detail this item draws, when the instance's part has a
    /// level-of-detail chain.
    pub lod: Option<LodTag>,
}

/// Where a drawable sits in its part's level-of-detail chain.
///
/// A renderer draws the drawable while the occurrence's screen coverage is in
/// `[min_coverage, max_coverage)` and fades it out over `crossfade` below
/// `min_coverage`; see [`crate::LodLevel`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LodTag {
    /// Level index, 0 being the finest.
    pub level: u32,
    /// Number of levels in the chain.
    pub levels: u32,
    /// Smallest coverage at which this level is drawn.
    pub min_coverage: f32,
    /// Coverage at which the finer level takes over, or `None` for level 0.
    pub max_coverage: Option<f32>,
    /// Fade band below `min_coverage`.
    pub crossfade: f32,
}

/// One compiled body of a placement set, placed at every set placement.
#[derive(Clone, Debug)]
pub struct RenderBatch {
    /// Stable identity of the set; placement `i` is `path#i`.
    pub path: InstancePath,
    /// The owning set handle (valid for the source assembly only).
    pub set: PlacementSetId,
    /// The part whose compiled entry holds this body's buffers. Under
    /// [`LodEmission::AllLevels`] this is the level's part, which differs from
    /// the placed part for lower levels.
    pub part: PartId,
    /// Index into the compiled part's body list.
    pub body: u32,
    /// World placements, one per set placement, in set order. Shared by
    /// every body (and level) the set emits.
    pub world: Arc<[Placement3]>,
    /// Per-placement seeds, parallel to [`Self::world`], if the set has them.
    /// Shared with the source set; no per-batch copy.
    pub seeds: Option<Arc<[u64]>>,
    /// Per-placement linear RGBA tints, parallel to [`Self::world`], if the
    /// set has them. Shared with the source set; no per-batch copy.
    pub tints: Option<Arc<[[f32; 4]]>>,
    /// World-space bounds per placement under the flatten's [`BoundsPolicy`];
    /// empty when the body has no geometry.
    pub world_bounds: Vec<Aabb3>,
    /// Index ranges with resolved material keys, shared by every placement.
    pub regions: Vec<ResolvedRegion>,
    /// The level of detail this batch draws, when the set's part has a
    /// level-of-detail chain.
    pub lod: Option<LodTag>,
}

impl RenderBatch {
    /// Number of placed occurrences.
    #[must_use]
    pub fn len(&self) -> usize {
        self.world.len()
    }

    /// True when the batch places nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.world.is_empty()
    }
}

/// How [`flatten_with`] computes world-space bounds.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum BoundsPolicy {
    /// Transform the body's part-local bounds as a box: the tightest box
    /// around the transformed corners. Costs O(1) per placed body after one
    /// O(vertices) pass per compiled body. Exact under translation, axis
    /// permutation and axis-aligned scale; under other rotations or shear it
    /// can exceed the exact bounds of the placed geometry, never undercut it.
    #[default]
    TransformedBox,
    /// Transform every emitted position: exact bounds at O(vertices) per
    /// placed body.
    Exact,
}

/// Which levels of detail [`flatten_with`] emits for parts with a chain.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum LodEmission {
    /// Only level 0, the part itself: consumers without level-of-detail
    /// support draw full detail and never overlap levels.
    #[default]
    BaseLevel,
    /// Every level, each tagged with its coverage range. Consumers must select
    /// or crossfade levels; drawing all of them overlaps the geometry.
    AllLevels,
}

/// Options for [`flatten_with`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct FlattenOptions {
    /// How world-space bounds are computed.
    pub bounds: BoundsPolicy,
    /// Which levels of detail are emitted.
    pub lods: LodEmission,
}

impl FlattenOptions {
    /// These options with the given bounds policy.
    #[must_use]
    pub fn with_bounds(mut self, bounds: BoundsPolicy) -> Self {
        self.bounds = bounds;
        self
    }

    /// These options with the given level-of-detail emission.
    #[must_use]
    pub fn with_lods(mut self, lods: LodEmission) -> Self {
        self.lods = lods;
        self
    }
}

/// A flat, deterministic list of drawables.
#[derive(Clone, Debug, Default)]
pub struct RenderList {
    /// Instance drawables in depth-first insertion order.
    pub items: Vec<RenderItem>,
    /// Placement-set drawables: root-level sets first, then the sets of each
    /// instance in depth-first insertion order; bodies in body order.
    pub batches: Vec<RenderBatch>,
}

impl RenderList {
    /// Total placed triangle count, including every instance occurrence and
    /// every placement of every set. Under [`LodEmission::AllLevels`] every
    /// emitted level counts.
    ///
    /// Unlike [`crate::CompiledPart::triangle_count`], this count follows the
    /// flattened drawables: placing one compiled part twice counts its
    /// triangles twice.
    #[must_use]
    pub fn triangle_count(&self) -> u64 {
        let triangles = |regions: &[ResolvedRegion]| {
            regions
                .iter()
                .map(|region| u64::from(region.count) / 3)
                .sum::<u64>()
        };
        let items: u64 = self.items.iter().map(|item| triangles(&item.regions)).sum();
        let batches: u64 = self
            .batches
            .iter()
            .map(|batch| triangles(&batch.regions) * batch.len() as u64)
            .sum();
        items + batches
    }

    /// Number of placed bodies: items plus every batch placement.
    #[must_use]
    pub fn placed_body_count(&self) -> u64 {
        self.items.len() as u64 + self.batches.iter().map(|b| b.len() as u64).sum::<u64>()
    }

    /// Axis-aligned union of all placed geometry in world space.
    ///
    /// Unlike [`crate::CompiledPart::bounds`], instance placements have been
    /// applied. Empty bodies do not affect the union; an empty render list
    /// returns `None`.
    #[must_use]
    pub fn bounds(&self) -> Option<Aabb3> {
        let mut bounds = Aabb3::EMPTY;
        for item in &self.items {
            if let Some(item_bounds) = item.world_bounds {
                bounds.union(&item_bounds);
            }
        }
        for batch in &self.batches {
            for placement_bounds in &batch.world_bounds {
                bounds.union(placement_bounds);
            }
        }
        (!bounds.is_empty()).then_some(bounds)
    }
}

/// Composes two placements: `outer` applied after `inner`
/// (`world = outer ∘ inner`).
#[must_use]
pub fn compose(outer: &Placement3, inner: &Placement3) -> Placement3 {
    let a = &outer.rows;
    let b = &inner.rows;
    let mut rows = [[0.0; 4]; 3];
    for (i, row) in rows.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate().take(3) {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
        row[3] = a[i][0] * b[0][3] + a[i][1] * b[1][3] + a[i][2] * b[2][3] + a[i][3];
    }
    Placement3 { rows }
}

/// Flattens `assembly` against its compiled parts with default options:
/// [`BoundsPolicy::TransformedBox`] bounds.
///
/// See [`flatten_with`].
#[must_use]
pub fn flatten(assembly: &Assembly, compiled: &CompiledParts) -> RenderList {
    flatten_with(assembly, compiled, &FlattenOptions::default())
}

/// Flattens `assembly` against its compiled parts.
///
/// Instances and sets whose part has no compiled entry are skipped (this
/// only happens when `compiled` came from a different assembly state); with
/// a matching [`CompiledParts`] every instance contributes one item per
/// compiled body, and every placement set one batch per compiled body.
#[must_use]
pub fn flatten_with(
    assembly: &Assembly,
    compiled: &CompiledParts,
    options: &FlattenOptions,
) -> RenderList {
    let mut flattener = Flattener {
        assembly,
        compiled,
        policy: options.bounds,
        lods: options.lods,
        local_bounds: Vec::new(),
        list: RenderList::default(),
    };
    for &set in assembly.root_placement_sets() {
        flattener.emit_set(set, &Placement3::IDENTITY);
    }
    // Explicit stack, children pushed in reverse so they pop in insertion
    // order: depth-first preorder.
    let mut stack: Vec<(InstanceId, Placement3)> = Vec::new();
    for &root in assembly.roots().iter().rev() {
        if let Some(inst) = assembly.instance(root) {
            stack.push((root, *inst.placement()));
        }
    }
    while let Some((id, world)) = stack.pop() {
        let Some(inst) = assembly.instance(id) else {
            continue;
        };
        flattener.emit_instance(id, &world);
        for &set in inst.placement_sets() {
            flattener.emit_set(set, &world);
        }
        for &child in inst.children().iter().rev() {
            if let Some(c) = assembly.instance(child) {
                stack.push((child, compose(&world, c.placement())));
            }
        }
    }
    flattener.list
}

struct Flattener<'a> {
    assembly: &'a Assembly,
    compiled: &'a CompiledParts,
    policy: BoundsPolicy,
    lods: LodEmission,
    /// Part-local body bounds, computed once per part on first use.
    local_bounds: Vec<Option<Vec<Option<Aabb3>>>>,
    list: RenderList,
}

impl Flattener<'_> {
    /// The `(part, tag)` pairs to emit for an occurrence of `part`, without
    /// allocating: this runs once per placed occurrence.
    fn levels<'a>(
        assembly: &'a Assembly,
        lods: LodEmission,
        part: PartId,
    ) -> impl Iterator<Item = (PartId, Option<LodTag>)> + 'a {
        let chain = assembly.part(part).map_or(&[][..], |def| def.lods());
        let count = match lods {
            LodEmission::BaseLevel => 1,
            LodEmission::AllLevels => chain.len(),
        };
        let unchained = chain.is_empty().then_some((part, None));
        unchained.into_iter().chain(
            chain
                .iter()
                .take(count)
                .enumerate()
                .map(move |(i, level)| (level.part, Some(lod_tag(chain, i)))),
        )
    }

    /// The material a lower level's `slot` resolves to: the occurrence's
    /// binding of the owning slot with the same name, else the owning part's
    /// default for that slot, else the level part's own default.
    fn level_material<'a>(
        assembly: &'a Assembly,
        owner: PartId,
        level: PartId,
        slot: SlotIndex,
        binding: impl FnOnce(SlotIndex) -> Option<&'a str>,
    ) -> Option<&'a str> {
        let owned = assembly.owner_slot(owner, level, slot);
        owned
            .and_then(binding)
            .or_else(|| owned.and_then(|o| assembly.part(owner)?.default_material(o)))
            .or_else(|| assembly.part(level)?.default_material(slot))
    }

    fn emit_instance(&mut self, id: InstanceId, world: &Placement3) {
        let assembly = self.assembly;
        let Some(owner) = assembly.instance(id).and_then(|inst| inst.part()) else {
            return;
        };
        let path = assembly
            .path_of(id)
            .unwrap_or_else(|| InstancePath(Vec::new()));
        for (part, lod) in Self::levels(assembly, self.lods, owner) {
            let (Some(def), Some(entry)) = (assembly.part(part), self.compiled.part(part)) else {
                continue;
            };
            for (body_index, body) in entry.bodies.iter().enumerate() {
                let world_bounds = self.placed_bounds(part, body_index, body, world);
                let regions = resolve_regions(
                    body,
                    |region| def.region_slot(region),
                    |slot| {
                        Self::level_material(assembly, owner, part, slot, |owned| {
                            assembly.instance(id)?.binding(owned)
                        })
                    },
                );
                self.list.items.push(RenderItem {
                    path: path.clone(),
                    instance: id,
                    world: *world,
                    part,
                    body: crate::len_u32(body_index),
                    world_bounds,
                    regions,
                    lod,
                });
            }
        }
    }

    fn emit_set(&mut self, id: PlacementSetId, parent_world: &Placement3) {
        let assembly = self.assembly;
        let Some(set) = assembly.placement_set(id) else {
            return;
        };
        let owner = set.part();
        let path = assembly
            .placement_set_path(id)
            .unwrap_or_else(|| InstancePath(Vec::new()));
        let world: Arc<[Placement3]> = set
            .placements()
            .iter()
            .map(|local| compose(parent_world, local))
            .collect();
        for (part, lod) in Self::levels(assembly, self.lods, owner) {
            let (Some(def), Some(entry)) = (assembly.part(part), self.compiled.part(part)) else {
                continue;
            };
            for (body_index, body) in entry.bodies.iter().enumerate() {
                let world_bounds = world
                    .iter()
                    .filter_map(|placement| self.placed_bounds(part, body_index, body, placement))
                    .collect();
                let regions = resolve_regions(
                    body,
                    |region| def.region_slot(region),
                    |slot| {
                        Self::level_material(assembly, owner, part, slot, |owned| {
                            set.binding(owned)
                        })
                    },
                );
                self.list.batches.push(RenderBatch {
                    path: path.clone(),
                    set: id,
                    part,
                    body: crate::len_u32(body_index),
                    world: Arc::clone(&world),
                    seeds: set.seeds.clone(),
                    tints: set.tints.clone(),
                    world_bounds,
                    regions,
                    lod,
                });
            }
        }
    }

    fn placed_bounds(
        &mut self,
        part: PartId,
        body_index: usize,
        body: &CompiledBody,
        world: &Placement3,
    ) -> Option<Aabb3> {
        match self.policy {
            BoundsPolicy::Exact => exact_bounds(&body.tri.positions, world),
            BoundsPolicy::TransformedBox => {
                let slot = part.0 as usize;
                if self.local_bounds.len() <= slot {
                    self.local_bounds.resize(slot + 1, None);
                }
                let bodies = self.local_bounds[slot].get_or_insert_with(|| {
                    self.compiled
                        .part(part)
                        .map(|entry| entry.bodies.iter().map(CompiledBody::bounds).collect())
                        .unwrap_or_default()
                });
                bodies
                    .get(body_index)
                    .copied()
                    .flatten()
                    .map(|local| transformed_box(&local, world))
            }
        }
    }
}

fn lod_tag(chain: &[LodLevel], level: usize) -> LodTag {
    LodTag {
        level: crate::len_u32(level),
        levels: crate::len_u32(chain.len()),
        min_coverage: chain[level].min_coverage,
        max_coverage: level.checked_sub(1).map(|finer| chain[finer].min_coverage),
        crossfade: chain[level].crossfade,
    }
}

fn resolve_regions<'a>(
    body: &CompiledBody,
    region_slot: impl Fn(u32) -> Option<SlotIndex>,
    material: impl Fn(SlotIndex) -> Option<&'a str>,
) -> Vec<ResolvedRegion> {
    body.regions
        .iter()
        .map(|range| ResolvedRegion {
            region: range.region,
            start: range.start,
            count: range.count,
            material: range
                .material_slot
                .or_else(|| region_slot(range.region))
                .and_then(&material)
                .map(ToString::to_string),
        })
        .collect()
}

/// The tightest box around `local`'s corners after `world`.
///
/// Per output axis, each linear term contributes its smaller and larger
/// product with the local extent; translation-only placements therefore
/// reproduce the exact per-vertex bounds bit for bit.
fn transformed_box(local: &Aabb3, world: &Placement3) -> Aabb3 {
    let mut out = Aabb3 {
        min: [0.0; 3],
        max: [0.0; 3],
    };
    for (axis, row) in world.rows.iter().enumerate() {
        let mut lo = 0.0;
        let mut hi = 0.0;
        for ((m, min), max) in row.iter().zip(local.min).zip(local.max) {
            let a = m * min;
            let b = m * max;
            lo += a.min(b);
            hi += a.max(b);
        }
        out.min[axis] = lo + row[3];
        out.max[axis] = hi + row[3];
    }
    out
}

fn exact_bounds(positions: &[[f32; 3]], world: &Placement3) -> Option<Aabb3> {
    let mut positions = positions.iter();
    let first = transform_position(*positions.next()?, world);
    let mut bounds = Aabb3 {
        min: first,
        max: first,
    };
    for &position in positions {
        let position = transform_position(position, world);
        for (axis, value) in position.into_iter().enumerate() {
            bounds.min[axis] = bounds.min[axis].min(value);
            bounds.max[axis] = bounds.max[axis].max(value);
        }
    }
    Some(bounds)
}

fn transform_position(position: [f32; 3], placement: &Placement3) -> [f64; 3] {
    let [x, y, z] = position.map(f64::from);
    placement
        .rows
        .map(|row| row[0] * x + row[1] * y + row[2] * z + row[3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{CompilePolicy, PartCompiler};
    use exedra_constructive::builders;
    use exedra_constructive::ir::{CapMode, NodeKind, Recipe, RecipeBuilder};
    use exedra_constructive::tessellate::EvalPolicy;

    fn slotted_recipe() -> Recipe {
        let mut b = RecipeBuilder::new();
        let front = b.material_slot("front");
        let body = b.material_slot("body");
        let _ = (front, body);
        let profile = b.add_profile(builders::rect_from_corner(40.0, 20.0).unwrap());
        let node = b
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 10.0,
                caps: CapMode::Both,
            })
            .unwrap();
        b.finish(node).unwrap()
    }

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} != {b}");
    }

    #[test]
    fn n_instances_flatten_from_one_compilation() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", slotted_recipe()).unwrap();
        for i in 0..5 {
            asm.add_instance(
                None,
                &alloc::format!("p{i}"),
                part,
                Placement3::translate(f64::from(i) * 50.0, 0.0, 0.0),
            )
            .unwrap();
        }
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let list = flatten(&asm, &compiled);
        assert_eq!(compiler.counters().parts_compiled, 1);
        assert_eq!(list.items.len(), 5);
        for (i, item) in list.items.iter().enumerate() {
            assert_eq!(item.part, part);
            approx(item.world.rows[0][3], i as f64 * 50.0);
            assert_eq!(
                item.path,
                InstancePath::from_segments(&[&alloc::format!("p{i}")])
            );
            // Every region range covers whole triangles and the union is
            // the full index buffer.
            let body = &compiled.part(part).unwrap().bodies[item.body as usize];
            let total: u32 = item.regions.iter().map(|r| r.count).sum();
            assert_eq!(total as usize, body.tri.indices.len());
        }
    }

    #[test]
    fn placed_accounting_counts_instances_and_world_bounds() {
        // Compiled accounting describes one part in local space. Placed
        // accounting must count both occurrences and include their exact
        // translated extrema in the world-space union.
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", slotted_recipe()).unwrap();
        asm.add_instance(None, "local", part, Placement3::IDENTITY)
            .unwrap();
        asm.add_instance(
            None,
            "translated",
            part,
            Placement3::translate(100.0, -5.0, 2.0),
        )
        .unwrap();
        let compiled = PartCompiler::new()
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let list = flatten(&asm, &compiled);
        let compiled_part = compiled.part(part).unwrap();

        assert_eq!(list.triangle_count(), 2 * compiled_part.triangle_count());
        assert_eq!(
            list.bounds(),
            Some(Aabb3 {
                min: [0.0, -5.0, 0.0],
                max: [140.0, 20.0, 12.0],
            })
        );
    }

    #[test]
    fn placement_sets_flatten_to_batches_under_their_parent() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", slotted_recipe()).unwrap();
        asm.set_default_slot(part, "body").unwrap();
        asm.set_part_material(part, "body", "mdf").unwrap();
        let frame = asm
            .add_frame(None, "forest", Placement3::translate(0.0, 100.0, 0.0))
            .unwrap();
        let placements: Vec<Placement3> = (0..4)
            .map(|i| Placement3::translate(f64::from(i) * 50.0, 0.0, 0.0))
            .collect();
        let set = asm
            .add_placement_set(Some(frame), "panels", part, placements)
            .unwrap();
        asm.bind_placement_material(set, "body", "oak").unwrap();
        asm.add_instance(None, "single", part, Placement3::IDENTITY)
            .unwrap();
        let compiled = PartCompiler::new()
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let list = flatten(&asm, &compiled);

        assert_eq!(list.items.len(), 1);
        assert_eq!(list.batches.len(), 1);
        let batch = &list.batches[0];
        assert_eq!(
            batch.path,
            InstancePath::from_segments(&["forest", "panels"])
        );
        assert_eq!(batch.len(), 4);
        approx(batch.world[3].rows[0][3], 150.0);
        approx(batch.world[3].rows[1][3], 100.0);
        assert!(
            batch
                .regions
                .iter()
                .filter(|r| r.region == 2)
                .all(|r| r.material.as_deref() == Some("oak"))
        );
        let per_body = compiled.part(part).unwrap().triangle_count();
        assert_eq!(list.triangle_count(), 5 * per_body);
        assert_eq!(list.placed_body_count(), 5);
        let bounds = list.bounds().unwrap();
        assert_eq!(bounds.max[0], 190.0);
        assert_eq!(bounds.max[1], 120.0);
    }

    #[test]
    fn transformed_box_bounds_are_exact_under_translation_and_contain_rotations() {
        let positions = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let local = Aabb3 {
            min: [0.0, 0.0, 0.0],
            max: [2.0, 1.0, 0.0],
        };
        let shifted = Placement3::translate(3.0, -2.0, 5.0);
        assert_eq!(
            transformed_box(&local, &shifted),
            exact_bounds(&positions, &shifted).unwrap()
        );
        let rotated =
            Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_4, 3.0, -2.0, 5.0);
        let loose = transformed_box(&local, &rotated);
        let exact = exact_bounds(&positions, &rotated).unwrap();
        for axis in 0..3 {
            assert!(loose.min[axis] <= exact.min[axis]);
            assert!(loose.max[axis] >= exact.max[axis]);
        }
        // The rotated triangle occupies only part of its box: the box policy
        // is looser than exact on y.
        assert!(loose.max[1] > exact.max[1] + 0.5);

        // Mirrors, axis permutations and non-uniform axis-aligned scale stay
        // exact.
        let mirrored = Placement3 {
            rows: [
                [0.0, -3.0, 0.0, 1.0],
                [2.5, 0.0, 0.0, -4.0],
                [0.0, 0.0, -0.5, 2.0],
            ],
        };
        assert_eq!(
            transformed_box(&local, &mirrored),
            exact_bounds(&positions, &mirrored).unwrap()
        );
    }

    #[test]
    fn bounds_policy_selects_exact_or_box_bounds() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", slotted_recipe()).unwrap();
        asm.add_instance(
            None,
            "turned",
            part,
            Placement3::rotate_z_then_translate(0.3, 0.0, 0.0, 0.0),
        )
        .unwrap();
        let compiled = PartCompiler::new()
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let boxed = flatten(&asm, &compiled).bounds().unwrap();
        let exact = flatten_with(
            &asm,
            &compiled,
            &FlattenOptions::default().with_bounds(BoundsPolicy::Exact),
        )
        .bounds()
        .unwrap();
        // An extruded rectangle is its own box: both policies agree up to
        // rounding.
        for axis in 0..3 {
            assert!((boxed.min[axis] - exact.min[axis]).abs() < 1e-9);
            assert!((boxed.max[axis] - exact.max[axis]).abs() < 1e-9);
        }
    }

    #[test]
    fn exact_bounds_transform_emitted_positions_not_local_aabb_corners() {
        // A rotated non-box triangle occupies only part of its transformed
        // local AABB. Computing from emitted vertices keeps the public bound
        // exact instead of returning that larger box.
        let positions = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let world =
            Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_4, 3.0, -2.0, 5.0);
        let bounds = exact_bounds(&positions, &world).expect("three emitted positions");
        let q = core::f64::consts::FRAC_1_SQRT_2;

        approx(bounds.min[0], 3.0 - q);
        approx(bounds.max[0], 3.0 + 2.0 * q);
        approx(bounds.min[1], -2.0);
        approx(bounds.max[1], -2.0 + 2.0 * q);
        assert_eq!(bounds.min[2], 5.0);
        assert_eq!(bounds.max[2], 5.0);
    }

    #[test]
    fn empty_render_list_has_zero_work_and_no_bounds() {
        // Empty placed geometry is the identity for both accounting folds.
        let list = RenderList::default();
        assert_eq!(list.triangle_count(), 0);
        assert_eq!(list.bounds(), None);
    }

    #[test]
    fn world_placements_compose_down_the_tree() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", slotted_recipe()).unwrap();
        let root = asm
            .add_instance(
                None,
                "root",
                part,
                Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_2, 0.0, 0.0, 3.0),
            )
            .unwrap();
        let child = asm
            .add_instance(
                Some(root),
                "child",
                part,
                Placement3::translate(10.0, 0.0, 0.0),
            )
            .unwrap();
        let _ = child;
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let list = flatten(&asm, &compiled);
        assert_eq!(list.items.len(), 2);
        let child_item = &list.items[1];
        assert_eq!(
            child_item.path,
            InstancePath::from_segments(&["root", "child"])
        );
        // Rz(90°) maps +X to +Y: child origin lands at (0, 10, 3).
        approx(child_item.world.rows[0][3], 0.0);
        approx(child_item.world.rows[1][3], 10.0);
        approx(child_item.world.rows[2][3], 3.0);
    }

    #[test]
    fn material_resolution_with_overrides() {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", slotted_recipe()).unwrap();
        asm.set_default_slot(part, "body").unwrap();
        asm.bind_region_slot(part, 0, "front").unwrap(); // start cap
        asm.set_part_material(part, "front", "oak").unwrap();
        asm.set_part_material(part, "body", "mdf").unwrap();
        let a = asm
            .add_instance(None, "a", part, Placement3::IDENTITY)
            .unwrap();
        let b = asm
            .add_instance(None, "b", part, Placement3::IDENTITY)
            .unwrap();
        asm.bind_material(b, "front", "walnut").unwrap();
        let _ = a;
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let list = flatten(&asm, &compiled);
        let front_of = |item: &RenderItem| {
            item.regions
                .iter()
                .find(|r| r.region == 0)
                .and_then(|r| r.material.clone())
        };
        let body_of = |item: &RenderItem| {
            item.regions
                .iter()
                .find(|r| r.region == 2)
                .and_then(|r| r.material.clone())
        };
        assert_eq!(front_of(&list.items[0]).as_deref(), Some("oak"));
        assert_eq!(front_of(&list.items[1]).as_deref(), Some("walnut"));
        assert_eq!(body_of(&list.items[0]).as_deref(), Some("mdf"));
        assert_eq!(body_of(&list.items[1]).as_deref(), Some("mdf"));
        // Rebinding is structure-only: zero new compilations, new resolution.
        asm.bind_material(b, "front", "ash").unwrap();
        let compiled = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let list = flatten(&asm, &compiled);
        assert_eq!(front_of(&list.items[1]).as_deref(), Some("ash"));
        assert_eq!(compiler.counters().parts_compiled, 1);
    }

    #[test]
    fn baked_parts_flatten_with_explicit_slots() {
        let recipe = slotted_recipe();
        let evaluation =
            exedra_constructive::evaluate::evaluate(&recipe, &EvalPolicy::default()).unwrap();
        let mesh = evaluation
            .bodies
            .into_iter()
            .next()
            .unwrap()
            .body
            .mesh
            .clone();
        let mut asm = Assembly::new();
        let part = asm.add_baked_part("baked", mesh, &["shell"]).unwrap();
        asm.set_default_slot(part, "shell").unwrap();
        asm.set_part_material(part, "shell", "steel").unwrap();
        asm.add_instance(None, "i", part, Placement3::IDENTITY)
            .unwrap();
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&asm, &CompilePolicy::default())
            .unwrap();
        let list = flatten(&asm, &compiled);
        assert_eq!(list.items.len(), 1);
        assert!(
            list.items[0]
                .regions
                .iter()
                .all(|r| r.material.as_deref() == Some("steel"))
        );
    }

    #[test]
    fn compose_is_affine_composition() {
        let t = Placement3::translate(1.0, 2.0, 3.0);
        let r = Placement3::rotate_z_then_translate(core::f64::consts::PI, 0.0, 0.0, 0.0);
        let world = compose(&r, &t);
        // Rz(180°) then nothing: translation flips in x/y.
        approx(world.rows[0][3], -1.0);
        approx(world.rows[1][3], -2.0);
        approx(world.rows[2][3], 3.0);
        let identity = compose(&Placement3::IDENTITY, &t);
        assert_eq!(identity, t);
    }
}
