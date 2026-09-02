// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The renderer seam: flattening an assembly into a [`RenderList`].
//!
//! [`flatten`] walks the instance tree depth-first in insertion order,
//! composes f64 world placements down the tree, and resolves every
//! region's material key through the binding chain (instance binding wins
//! over part default). The result is a flat, deterministic list that
//! renderers and exporters consume; it carries no geometry of its own —
//! items reference compiled bodies in the [`CompiledParts`] set. Each item
//! also records exact world-space bounds derived from the emitted positions,
//! so placed geometry budgets do not need to reconstruct the render seam.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use exedra_constructive::evaluate::Aabb3;
use exedra_constructive::ir::Placement3;

use crate::assembly::{Assembly, InstanceId, InstancePath, PartId};
use crate::compile::CompiledParts;

/// One region of a rendered body with its resolved material key.
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
    /// The part whose compiled entry holds this body's buffers.
    pub part: PartId,
    /// Index into the compiled part's body list.
    pub body: u32,
    /// Axis-aligned bounds of this body's emitted positions after `world`.
    ///
    /// [`flatten`] transforms every emitted position rather than transforming
    /// the part-local AABB, so rotations and general affine placements do not
    /// make this an overestimate. An empty body has no bounds.
    pub world_bounds: Option<Aabb3>,
    /// Per-region index ranges with resolved material keys, ascending by
    /// region and covering the whole index buffer.
    pub regions: Vec<ResolvedRegion>,
}

/// A flat, deterministic list of drawables.
#[derive(Clone, Debug, Default)]
pub struct RenderList {
    /// Items in depth-first insertion order.
    pub items: Vec<RenderItem>,
}

impl RenderList {
    /// Total placed triangle count, including every instance occurrence.
    ///
    /// Unlike [`crate::CompiledPart::triangle_count`], this count follows the
    /// flattened drawables: placing one compiled part twice counts its
    /// triangles twice.
    #[must_use]
    pub fn triangle_count(&self) -> u64 {
        self.items
            .iter()
            .flat_map(|item| &item.regions)
            .map(|region| u64::from(region.count) / 3)
            .sum()
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

/// Flattens `assembly` against its compiled parts.
///
/// Instances whose part has no compiled entry are skipped (this only
/// happens when `compiled` came from a different assembly state); with a
/// matching [`CompiledParts`] every instance contributes one item per
/// compiled body.
#[must_use]
pub fn flatten(assembly: &Assembly, compiled: &CompiledParts) -> RenderList {
    let mut items = Vec::new();
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
        if let (Some(def), Some(entry)) = (assembly.part(inst.part()), compiled.part(inst.part())) {
            let path = assembly
                .path_of(id)
                .unwrap_or_else(|| InstancePath(Vec::new()));
            for (body_index, body) in entry.bodies.iter().enumerate() {
                let world_bounds = placed_bounds(&body.tri.positions, &world);
                let regions = body
                    .regions
                    .iter()
                    .map(|range| ResolvedRegion {
                        region: range.region,
                        start: range.start,
                        count: range.count,
                        material: def
                            .region_slot(range.region)
                            .and_then(|slot| assembly.resolved_material(id, slot))
                            .map(ToString::to_string),
                    })
                    .collect();
                items.push(RenderItem {
                    path: path.clone(),
                    instance: id,
                    world,
                    part: inst.part(),
                    body: crate::len_u32(body_index),
                    world_bounds,
                    regions,
                });
            }
        }
        for &child in inst.children().iter().rev() {
            if let Some(c) = assembly.instance(child) {
                stack.push((child, compose(&world, c.placement())));
            }
        }
    }
    RenderList { items }
}

fn placed_bounds(positions: &[[f32; 3]], world: &Placement3) -> Option<Aabb3> {
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
    use crate::compile::PartCompiler;
    use exedra_constructive::builders;
    use exedra_constructive::ir::{CapMode, NodeKind, Recipe, RecipeBuilder};
    use exedra_constructive::tessellate::EvalPolicy;

    fn slotted_recipe() -> Recipe {
        let mut b = RecipeBuilder::new();
        let front = b.material_slot("front");
        let body = b.material_slot("body");
        let _ = (front, body);
        let profile = b.add_profile(builders::rect(40.0, 20.0).unwrap());
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
            .compile_parts(&asm, &EvalPolicy::default())
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
            .compile_parts(&asm, &EvalPolicy::default())
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
    fn placed_bounds_transform_emitted_positions_not_local_aabb_corners() {
        // A rotated non-box triangle occupies only part of its transformed
        // local AABB. Computing from emitted vertices keeps the public bound
        // exact instead of returning that larger box.
        let positions = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let world =
            Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_4, 3.0, -2.0, 5.0);
        let bounds = placed_bounds(&positions, &world).expect("three emitted positions");
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
            .compile_parts(&asm, &EvalPolicy::default())
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
            .compile_parts(&asm, &EvalPolicy::default())
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
            .compile_parts(&asm, &EvalPolicy::default())
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
            .compile_parts(&asm, &EvalPolicy::default())
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
