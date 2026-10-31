// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Implicit-field leaves: solids given as [`Expr`] fields, meshed by
//! restricted Delaunay refinement.
//!
//! A [`NodeKind::Implicit`](crate::ir::NodeKind::Implicit) leaf holds a
//! field in its local frame, the region meshed, and a placement. Evaluation
//! meshes the field with [`exedra_isosurface::delaunay_mesh`] at the
//! resolution [`ImplicitPolicy`] sets, keeping its creases sharp, labelling
//! faces with the regions of [`Tagged`](exedra_implicit::Tagged) parts, and
//! then places the mesh. The meshing report rides with the body, and an
//! incomplete mesh is reported as a warning.

use alloc::vec::Vec;

use exedra_implicit::{Aabb, Expr, Formula, ProfileExpr};
use exedra_isosurface::{DelaunayParams, DelaunayReport, DelaunayResult, delaunay_mesh};
use exedra_math::Placement3;
use exedra_mesh::attr;

use crate::tessellate::{Feature, TessellateError, TessellatedBody};

/// How implicit leaves are meshed.
///
/// Resolution is relative to each leaf's size, so one policy suits leaves
/// in any units.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ImplicitPolicy {
    /// Cells along the longest side of the region meshed; at least one.
    /// The field is sampled on cubes of that side. Faces are kept within a
    /// fiftieth of a cell of the surface and grow to six cells where it is
    /// flat; features thinner than a cell are found only where probing
    /// reaches them.
    pub cells: u32,
    /// The most vertices one leaf's mesh may hold before refinement stops.
    pub max_points: usize,
}

impl Default for ImplicitPolicy {
    fn default() -> Self {
        Self {
            cells: 32,
            max_points: 1 << 20,
        }
    }
}

/// The deepest an implicit leaf's field may nest, counting each
/// sub-expression or profile as one level and a blend's list of parts as
/// one more. Interchange readers nest the field inside the recipe's own
/// structure, and this keeps the whole within what they accept.
pub const MAX_IMPLICIT_NESTING: usize = 96;

/// How deeply `field` nests, counted as for [`MAX_IMPLICIT_NESTING`].
#[must_use]
pub fn nesting(field: &Expr) -> usize {
    let mut deepest = 0;
    let mut pending = alloc::vec![(field, 1_usize)];
    while let Some((field, depth)) = pending.pop() {
        deepest = deepest.max(depth);
        match field {
            Expr::Sphere(_)
            | Expr::Cuboid(_)
            | Expr::Cylinder(_)
            | Expr::Torus(_)
            | Expr::HalfSpace(_)
            | Expr::Capsule(_)
            | Expr::RoundCone(_) => {}
            Expr::Extrude(_) | Expr::Revolve(_) => deepest = deepest.max(depth + 1),
            Expr::Union(u) => pending.extend([(&u.left, depth + 1), (&u.right, depth + 1)]),
            Expr::Intersection(i) => {
                pending.extend([(&i.left, depth + 1), (&i.right, depth + 1)]);
            }
            Expr::Difference(d) => pending.extend([(&d.left, depth + 1), (&d.right, depth + 1)]),
            Expr::SmoothUnion(u) => {
                pending.extend([(u.left(), depth + 1), (u.right(), depth + 1)]);
            }
            Expr::Blend(b) => pending.extend(b.parts().iter().map(|part| (part, depth + 2))),
            Expr::Translate(t) => pending.push((t.field(), depth + 1)),
            Expr::Scale(s) => pending.push((s.field(), depth + 1)),
            Expr::Placed(p) => pending.push((p.field(), depth + 1)),
            Expr::Tagged(t) => pending.push((&t.field, depth + 1)),
        }
    }
    deepest
}

/// The region meshed for `field` and the side of its cells: `bounds` when
/// given, otherwise the box the field's claims put its interior in, grown
/// by two cells so the surface lies inside.
pub(crate) fn meshed_region(
    field: &Expr,
    bounds: Option<Aabb>,
    policy: &ImplicitPolicy,
) -> Option<(Aabb, f64)> {
    let cells = f64::from(policy.cells.max(1));
    let cell = |b: &Aabb| b.extent().into_iter().fold(0.0, f64::max) / cells;
    if let Some(bounds) = bounds {
        return Some((bounds, cell(&bounds)));
    }
    let support = *Formula::claims(field).support.bounds()?;
    let size = cell(&support);
    Some((support.grown(2.0 * size)?, size))
}

/// Meshes `field` within `bounds` (or its claimed support) and places the
/// mesh by `placement`.
pub(crate) fn tessellate(
    field: &Expr,
    bounds: Option<Aabb>,
    placement: &Placement3,
    policy: &ImplicitPolicy,
) -> Result<TessellatedBody, TessellateError> {
    let (region, cell) =
        meshed_region(field, bounds, policy).ok_or(TessellateError::ImplicitUnbounded)?;
    let mut params = DelaunayParams::new(region, cell);
    params.max_points = policy.max_points;
    params.regions = true;
    let mut result = delaunay_mesh(field, &params).map_err(TessellateError::Implicit)?;
    place(&mut result, placement)?;
    let mesh = result.mesh().map_err(TessellateError::Build)?;
    let regions = mesh.attrs().dense(attr::FACE_REGION);
    let face_region = |face: exedra_mesh::FaceId| {
        regions
            .and_then(|layer| layer.get(face.as_id()).copied())
            .unwrap_or(0)
    };
    let face_features: Vec<Feature> = mesh
        .faces()
        .map(|face| Feature::ImplicitRegion {
            region: face_region(face),
        })
        .collect();
    // A vertex takes the region of the first face around it.
    let mut vertex_features =
        alloc::vec![Feature::ImplicitRegion { region: 0 }; mesh.vertices().count()];
    let vertex_slots: Vec<exedra_mesh::VertexId> = mesh.vertices().collect();
    let mut seen = alloc::vec![false; vertex_slots.len()];
    for face in mesh.faces() {
        let region = face_region(face);
        for corner in mesh.face_loop(face) {
            if let Some(vertex) = mesh.from_vertex(corner)
                && let Ok(slot) = vertex_slots.binary_search(&vertex)
                && !seen[slot]
            {
                seen[slot] = true;
                vertex_features[slot] = Feature::ImplicitRegion { region };
            }
        }
    }
    let source_map = crate::source_map::SourceMap::new(&mesh, face_features, vertex_features);
    Ok(TessellatedBody {
        mesh,
        source_map,
        face_materials: alloc::collections::BTreeMap::new(),
        refinement: None,
        sweep_checks: None,
        path_sampling: None,
        loft_sampling: None,
        implicit: Some(result.report),
    })
}

/// Places the mesher's `f64` positions before the mesh narrows them, so a
/// field far from the origin placed near it keeps its precision; a
/// reflection reverses each triangle to keep it facing out.
fn place(result: &mut DelaunayResult, placement: &Placement3) -> Result<(), TessellateError> {
    for p in &mut result.positions {
        *p = placement.transform_point(*p);
        if !p.iter().all(|c| c.is_finite()) {
            return Err(TessellateError::NonFiniteGeometry);
        }
    }
    if placement.linear_determinant() < 0.0 {
        for t in &mut result.triangles {
            t.swap(1, 2);
        }
    }
    // Every triangle must keep its area once narrowed for the mesh.
    let narrowed = |i: u32| exedra_math::narrow(result.positions[i as usize]).map(f64::from);
    for t in &result.triangles {
        let [a, b, c] = t.map(narrowed);
        let area = exedra_math::cross(exedra_math::sub(b, a), exedra_math::sub(c, a));
        if area == [0.0; 3] || !area.iter().all(|v| v.is_finite()) {
            return Err(TessellateError::CollapsedGeometry);
        }
    }
    Ok(())
}

/// The diagnostic message for an incomplete implicit mesh.
pub(crate) fn incomplete_message(report: &DelaunayReport) -> alloc::string::String {
    let mut reasons = Vec::new();
    if report.hit_point_limit {
        reasons.push(alloc::string::String::from("the point budget ran out"));
    }
    if report.hit_crease_limit {
        reasons.push(alloc::string::String::from(
            "the crease sample budget ran out",
        ));
    }
    if report.clipped {
        reasons.push(alloc::string::String::from(
            "the surface crosses the meshed bounds",
        ));
    }
    if report.disk_failures > 0 {
        reasons.push(alloc::format!(
            "{} vertex neighborhood(s) are not disks",
            report.disk_failures
        ));
    }
    if report.broken_creases > 0 {
        reasons.push(alloc::format!(
            "{} crease segment(s) are cut across",
            report.broken_creases
        ));
    }
    if report.pending_balls > 0 {
        reasons.push(alloc::format!(
            "{} crease protection(s) could not shrink",
            report.pending_balls
        ));
    }
    if report.isolated_vertices > 0 {
        reasons.push(alloc::format!(
            "{} piece(s) of surface were lost",
            report.isolated_vertices
        ));
    }
    alloc::format!("implicit mesh is incomplete: {}", reasons.join("; "))
}

/// Appends a canonical encoding of `field` and `bounds` for fingerprints:
/// a tag per variant, then its parameters as exact bits, children in order.
pub(crate) fn encode(out: &mut Vec<u8>, field: &Expr, bounds: Option<Aabb>) {
    match bounds {
        None => out.push(0),
        Some(b) => {
            out.push(1);
            put_all(out, &b.min);
            put_all(out, &b.max);
        }
    }
    encode_expr(out, field);
}

fn put(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&v.to_bits().to_le_bytes());
}

fn put_all(out: &mut Vec<u8>, values: &[f64]) {
    for &v in values {
        put(out, v);
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per expression variant, each a short encoding"
)]
fn encode_expr(out: &mut Vec<u8>, field: &Expr) {
    match field {
        Expr::Sphere(s) => {
            out.push(0);
            put_all(out, &s.center());
            put(out, s.radius());
        }
        Expr::Cuboid(c) => {
            out.push(1);
            put_all(out, &c.center());
            put_all(out, &c.half_extents());
        }
        Expr::Cylinder(c) => {
            out.push(2);
            put_all(out, &c.center());
            put_all(out, &c.axis());
            put(out, c.radius());
            put(out, c.half_height());
        }
        Expr::Torus(t) => {
            out.push(3);
            put_all(out, &t.center());
            put_all(out, &t.axis());
            put(out, t.major_radius());
            put(out, t.minor_radius());
        }
        Expr::HalfSpace(h) => {
            out.push(4);
            put_all(out, &h.point());
            put_all(out, &h.normal());
        }
        Expr::Capsule(c) => {
            out.push(5);
            put_all(out, &c.a());
            put_all(out, &c.b());
            put(out, c.radius());
        }
        Expr::RoundCone(c) => {
            out.push(6);
            put_all(out, &c.a());
            put_all(out, &c.b());
            put(out, c.radius_a());
            put(out, c.radius_b());
        }
        Expr::Extrude(e) => {
            out.push(7);
            encode_profile(out, e.profile());
            put(out, e.half_height());
        }
        Expr::Revolve(r) => {
            out.push(8);
            encode_profile(out, r.profile());
        }
        Expr::Union(u) => {
            out.push(9);
            encode_expr(out, &u.left);
            encode_expr(out, &u.right);
        }
        Expr::Intersection(i) => {
            out.push(10);
            encode_expr(out, &i.left);
            encode_expr(out, &i.right);
        }
        Expr::Difference(d) => {
            out.push(11);
            encode_expr(out, &d.left);
            encode_expr(out, &d.right);
        }
        Expr::SmoothUnion(s) => {
            out.push(12);
            encode_expr(out, s.left());
            encode_expr(out, s.right());
            put(out, s.radius());
        }
        Expr::Blend(b) => {
            out.push(13);
            out.extend_from_slice(&(b.parts().len() as u64).to_le_bytes());
            for part in b.parts() {
                encode_expr(out, part);
            }
            put(out, b.radius());
        }
        Expr::Translate(t) => {
            out.push(14);
            encode_expr(out, t.field());
            put_all(out, &t.offset());
        }
        Expr::Scale(s) => {
            out.push(15);
            encode_expr(out, s.field());
            put(out, s.factor());
        }
        Expr::Placed(p) => {
            out.push(16);
            encode_expr(out, p.field());
            for row in p.placement().rows {
                put_all(out, &row);
            }
        }
        Expr::Tagged(t) => {
            out.push(17);
            encode_expr(out, &t.field);
            out.extend_from_slice(&t.region.to_le_bytes());
        }
    }
}

fn encode_profile(out: &mut Vec<u8>, profile: &ProfileExpr) {
    match profile {
        ProfileExpr::Circle(c) => {
            out.push(0);
            put_all(out, &c.center());
            put(out, c.radius());
        }
        ProfileExpr::Rectangle(r) => {
            out.push(1);
            put_all(out, &r.center());
            put_all(out, &r.half_extents());
        }
        ProfileExpr::HalfPlane(h) => {
            out.push(2);
            put_all(out, &h.point());
            put_all(out, &h.normal());
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use exedra_implicit::{Aabb, Cuboid, Cylinder, Difference, Expr, HalfSpace, Tagged};
    use exedra_math::Placement3;
    use exedra_mesh::attr;

    use super::ImplicitPolicy;
    use crate::cache::EvalCache;
    use crate::evaluate::{evaluate, evaluate_with_cache};
    use crate::ir::{NodeKind, RecipeBuilder, RecipeError};
    use crate::tessellate::{EvalPolicy, Feature};

    fn drilled() -> Expr {
        Expr::from(Difference::new(
            Expr::from(Tagged::new(
                Expr::from(Cuboid::new([0.0; 3], [0.6; 3]).unwrap()),
                10,
            )),
            Expr::from(Tagged::new(
                Expr::from(Cylinder::new([0.0; 3], [0.0, 0.0, 1.0], 0.3, 1.0).unwrap()),
                20,
            )),
        ))
    }

    fn policy() -> EvalPolicy {
        EvalPolicy {
            implicit: ImplicitPolicy {
                cells: 12,
                ..ImplicitPolicy::default()
            },
            ..EvalPolicy::default()
        }
    }

    #[test]
    fn implicit_leaves_mesh_sharp_tagged_and_placed() {
        let mut b = RecipeBuilder::new();
        let leaf = b
            .add_node(NodeKind::Implicit {
                field: drilled(),
                bounds: None,
                placement: Placement3::translate(5.0, 0.0, 0.0),
            })
            .unwrap();
        let recipe = b.finish(leaf).unwrap();
        let mut cache = EvalCache::new();
        let cold = evaluate_with_cache(&recipe, &policy(), &mut cache).unwrap();
        let report = cold.report.implicit_meshes[0];
        assert_eq!(report.0, leaf);
        assert!(report.1.is_complete(), "{:?}", report.1);
        assert!(
            cold.report.diagnostics.is_empty(),
            "{:?}",
            cold.report.diagnostics
        );

        let body = &cold.bodies[0].body;
        let mesh = &body.mesh;
        assert!(mesh.validate_deep().is_empty());
        // Placed: every vertex lies around x = 5.
        assert!(mesh.vertices().all(|v| {
            let p = mesh.vertex_position(v).unwrap();
            (p[0] - 5.0).abs() <= 0.61
        }));
        // Faces carry the parts' regions, in the mesh and as provenance.
        let regions = mesh.attrs().dense(attr::FACE_REGION).unwrap();
        let mut seen: Vec<u32> = mesh
            .faces()
            .map(|f| {
                let region = *regions.get(f.as_id()).unwrap();
                assert_eq!(
                    body.source_map.face_feature(f),
                    Some(Feature::ImplicitRegion { region })
                );
                region
            })
            .collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen, [10, 20]);
        // Box edges and bore rims are marked sharp.
        let sharp = mesh
            .half_edges()
            .filter(|&e| mesh.edge_sharpness(e).is_some_and(|s| s > 0.0))
            .count();
        assert!(sharp > 0);

        // A cache hit replays the meshing report.
        let warm = evaluate_with_cache(&recipe, &policy(), &mut cache).unwrap();
        assert_eq!(warm.report.counters.cache_hits, 1);
        assert_eq!(warm.report.implicit_meshes, cold.report.implicit_meshes);
    }

    #[test]
    fn incomplete_meshes_are_warned_of() {
        let mut b = RecipeBuilder::new();
        // Bounds that cut the box open.
        let leaf = b
            .add_node(NodeKind::Implicit {
                field: drilled(),
                bounds: Some(Aabb::new([-0.5; 3], [0.5; 3]).unwrap()),
                placement: Placement3::IDENTITY,
            })
            .unwrap();
        let recipe = b.finish(leaf).unwrap();
        let result = evaluate(&recipe, &policy()).unwrap();
        assert!(result.report.implicit_meshes[0].1.clipped);
        let warning = &result.report.diagnostics[0];
        assert_eq!(warning.code, "eval.implicit.incomplete");
        assert!(warning.message.contains("bounds"), "{}", warning.message);
    }

    #[test]
    fn fields_with_no_bounds_need_them_given() {
        let floor = Expr::from(HalfSpace::new([0.0; 3], [0.0, 0.0, 1.0]).unwrap());
        let mut b = RecipeBuilder::new();
        let refused = b.add_node(NodeKind::Implicit {
            field: floor.clone(),
            bounds: None,
            placement: Placement3::IDENTITY,
        });
        assert_eq!(
            refused.unwrap_err(),
            RecipeError::InvalidParameter {
                what: "implicit bounds"
            }
        );
        assert!(
            b.add_node(NodeKind::Implicit {
                field: floor,
                bounds: Some(Aabb::new([-1.0; 3], [1.0; 3]).unwrap()),
                placement: Placement3::IDENTITY,
            })
            .is_ok()
        );
    }

    #[test]
    fn fingerprints_follow_the_field() {
        let leaf = |field: Expr| {
            let mut b = RecipeBuilder::new();
            let node = b
                .add_node(NodeKind::Implicit {
                    field,
                    bounds: None,
                    placement: Placement3::IDENTITY,
                })
                .unwrap();
            b.finish(node).unwrap().recipe_fingerprint()
        };
        let a = leaf(drilled());
        assert_eq!(a, leaf(drilled()));
        let retagged = Expr::from(Tagged::new(drilled(), 4));
        assert_ne!(a, leaf(retagged));
    }

    #[test]
    fn placement_keeps_the_meshers_precision() {
        // A ball far from the origin, placed at it.
        let far = Expr::from(exedra_implicit::Sphere::new([1e9; 3], 1.0).unwrap());
        let mut b = RecipeBuilder::new();
        let leaf = b
            .add_node(NodeKind::Implicit {
                field: far,
                bounds: None,
                placement: Placement3::translate(-1e9, -1e9, -1e9),
            })
            .unwrap();
        let result = evaluate(&b.finish(leaf).unwrap(), &policy()).unwrap();
        let mesh = &result.bodies[0].body.mesh;
        assert!(mesh.validate_deep().is_empty());
        for v in mesh.vertices() {
            let p = mesh.vertex_position(v).unwrap().map(f64::from);
            let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!((r - 1.0).abs() < 0.02, "{p:?}");
        }
    }

    #[test]
    fn mirrored_leaves_face_outward() {
        let mut b = RecipeBuilder::new();
        let mirror =
            Placement3::from_axes([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]);
        let leaf = b
            .add_node(NodeKind::Implicit {
                field: drilled(),
                bounds: None,
                placement: mirror,
            })
            .unwrap();
        let result = evaluate(&b.finish(leaf).unwrap(), &policy()).unwrap();
        let mesh = &result.bodies[0].body.mesh;
        assert!(mesh.validate_deep().is_empty());
        // Outward faces enclose a positive volume.
        let mut volume = 0.0;
        for face in mesh.faces() {
            let p: Vec<[f64; 3]> = mesh
                .face_loop(face)
                .filter_map(|he| mesh.to_vertex(he))
                .filter_map(|v| mesh.vertex_position(v))
                .map(|p| p.map(f64::from))
                .collect();
            for i in 1..p.len() - 1 {
                volume += exedra_math::dot(p[0], exedra_math::cross(p[i], p[i + 1])) / 6.0;
            }
        }
        // The box less the bore: 1.2³ - π 0.3² 1.2.
        let expected = 1.2_f64.powi(3) - core::f64::consts::PI * 0.09 * 1.2;
        assert!((volume - expected).abs() < 0.05 * expected, "{volume}");
    }

    #[test]
    fn bounds_and_nesting_are_checked_when_added() {
        let ball = || Expr::from(exedra_implicit::Sphere::new([0.0; 3], 1.0).unwrap());
        let add = |field: Expr, bounds: Option<Aabb>| {
            RecipeBuilder::new().add_node(NodeKind::Implicit {
                field,
                bounds,
                placement: Placement3::IDENTITY,
            })
        };
        for (min, max) in [
            ([1.0; 3], [-1.0; 3]),
            ([f64::NAN, 0.0, 0.0], [1.0; 3]),
            ([0.0; 3], [f64::INFINITY, 1.0, 1.0]),
        ] {
            assert_eq!(
                add(ball(), Some(Aabb { min, max })).unwrap_err(),
                RecipeError::InvalidParameter {
                    what: "implicit bounds"
                }
            );
        }
        let chain =
            |depth: usize| (1..depth).fold(ball(), |field, _| Expr::from(Tagged::new(field, 1)));
        assert_eq!(
            super::nesting(&chain(super::MAX_IMPLICIT_NESTING)),
            super::MAX_IMPLICIT_NESTING
        );
        assert!(add(chain(super::MAX_IMPLICIT_NESTING), None).is_ok());
        assert_eq!(
            add(chain(super::MAX_IMPLICIT_NESTING + 1), None).unwrap_err(),
            RecipeError::InvalidParameter {
                what: "implicit nesting"
            }
        );
    }
}
