// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::evaluate::{Fidelity, evaluate};
use crate::ir::{
    CapMode, CsgOp, FramePolicy, NodeKind, Path3, Placement3, PrimitiveSpec, Recipe, RecipeBuilder,
};
use crate::profile::{Loop2, Profile2, Seg2};
use crate::tessellate::EvalPolicy;
use alloc::{format, vec, vec::Vec};

fn recipe(w: f64, h: f64, bar: f64, distinct: bool, stage: u8) -> Recipe {
    let mut b = RecipeBuilder::new();
    let depth = if distinct { 0.054 } else { 0.018 };
    let p = b.add_profile(
        Profile2::simple(
            Loop2::new(vec![
                Seg2::line((0.0, -bar / 2.0)),
                Seg2::line((depth, -bar / 2.0)),
                Seg2::line((depth, bar / 2.0)),
                Seg2::line((0.0, bar / 2.0)),
            ])
            .unwrap(),
        )
        .unwrap(),
    );
    let mut bars = Vec::new();
    for (points, z) in [
        (
            vec![[-w, -h, 0.0], [2.0 * w, 2.0 * h, 0.0]],
            if distinct { -0.018 } else { 0.0 },
        ),
        (
            vec![[-w, 2.0 * h, 0.0], [2.0 * w, -h, 0.0]],
            if distinct { -0.009 } else { 0.0 },
        ),
    ] {
        let child = b
            .add(NodeKind::Sweep {
                profile: p,
                path: Path3::Polyline {
                    points,
                    frame: FramePolicy::RotationMinimizing,
                },
                caps: CapMode::Both,
            })
            .unwrap();
        bars.push(
            b.add(NodeKind::Transform {
                child,
                xf: Placement3::translate(0.0, 0.0, z),
            })
            .unwrap(),
        );
    }
    let union = b
        .add(NodeKind::Csg {
            op: CsgOp::Union,
            operands: bars,
        })
        .unwrap();
    let root = if stage == 0 {
        union
    } else {
        let (x, y) = if stage == 3 { (0.05, 0.03) } else { (0.0, 0.0) };
        let panel = b
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box {
                    size: [w, h, if stage == 1 { 0.018 } else { 0.004 }],
                },
                placement: Placement3::translate(x, y, if stage == 1 { 0.0 } else { 0.007 }),
            })
            .unwrap();
        let cutter = b
            .add(NodeKind::Transform {
                child: union,
                xf: Placement3::translate(x, y, 0.0),
            })
            .unwrap();
        b.add(NodeKind::Csg {
            op: if stage == 1 {
                CsgOp::Intersection
            } else {
                CsgOp::Difference
            },
            operands: if stage == 1 {
                vec![cutter, panel]
            } else {
                vec![panel, cutter]
            },
        })
        .unwrap()
    };
    b.finish(root).unwrap()
}

// Four triangles remain outside the two infinite strips. Their bases and
// heights follow directly from the perpendicular bar width; each has the
// same area, even when the opening is not square.
fn expected(w: f64, h: f64, bar: f64, distinct: bool, stage: u8) -> (usize, f64) {
    let length = libm::sqrt(w * w + h * h);
    let infill_area = ((w * h - bar * length) * (w * h - bar * length)) / (w * h);
    match stage {
        0 => {
            let depth = if distinct { 0.054 } else { 0.018 };
            let overlap_depth = if distinct { 0.045 } else { 0.018 };
            let overlap_area = bar * bar * length * length / (2.0 * w * h);
            (1, 6.0 * length * bar * depth - overlap_area * overlap_depth)
        }
        1 => (1, (w * h - infill_area) * 0.018),
        _ => (4, infill_area * 0.004),
    }
}

fn component_geometry(mesh: &exedra_mesh::Mesh) -> Vec<(f64, [f64; 3], [f64; 3])> {
    use exedra_math::{cross, dot, sub};
    use exedra_mesh::{FaceId, FaceTriangulation};
    assert!(mesh.validate_deep().is_empty());
    let mut visited = Vec::new();
    let mut components = Vec::new();
    for seed in mesh.faces() {
        if visited.contains(&seed) {
            continue;
        }
        visited.push(seed);
        let mut pending = vec![seed];
        let mut volume = 0.0;
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        while let Some(face) = pending.pop() {
            for he in mesh.face_loop(face) {
                let neighbor = mesh.face(mesh.twin(he).unwrap()).unwrap();
                assert_ne!(neighbor, FaceId::OUTSIDE, "open component");
                if !visited.contains(&neighbor) {
                    visited.push(neighbor);
                    pending.push(neighbor);
                }
                let p = mesh
                    .vertex_position(mesh.to_vertex(he).unwrap())
                    .unwrap()
                    .map(f64::from);
                for i in 0..3 {
                    min[i] = min[i].min(p[i]);
                    max[i] = max[i].max(p[i]);
                }
            }
            for corners in mesh.face_triangles(face, FaceTriangulation::Robust) {
                let p = corners.map(|c| {
                    mesh.vertex_position(mesh.to_vertex(c).unwrap())
                        .unwrap()
                        .map(f64::from)
                });
                assert_ne!(cross(sub(p[1], p[0]), sub(p[2], p[0])), [0.0; 3]);
                volume += dot(p[0], cross(p[1], p[2])) / 6.0;
            }
        }
        assert!(volume > 0.0, "component must face outward");
        components.push((volume, min, max));
    }
    components
}

#[test]
fn diagonal_catalog_boolean_matrix() {
    for (width, height, bar) in [
        (480.0, 480.0, 24.0),
        (480.0, 780.0, 24.0),
        (780.0, 380.0, 24.0),
        (120.0, 830.0, 16.0),
        (480.0, 780.0, 48.0),
    ] {
        for derived in [false, true] {
            let (w, h): (f64, f64) = if derived {
                (
                    (width + 120.0) / 1000.0 - 0.05 - 0.07,
                    (height + 120.0) / 1000.0 - 0.03 - 0.09,
                )
            } else {
                (width / 1000.0, height / 1000.0)
            };
            let bar = bar / 1000.0;
            // A common translation is rounded into f32 before the second
            // Boolean. Compare geometry within eight storage ULPs at the
            // opening scale, independently of the Boolean's exact predicates.
            let position_tolerance = 8.0 * f64::from(f32::EPSILON) * w.max(h);
            for distinct in [false, true] {
                let mut unshifted = Vec::new();
                for stage in 0..4 {
                    let context = format!(
                        "{w}x{h}/{bar} derived={derived} distinct={distinct} stage={stage}"
                    );
                    let recipe = recipe(w, h, bar, distinct, stage);
                    let out = evaluate(&recipe, &EvalPolicy::default()).unwrap();
                    assert_eq!(
                        out.report.fidelity_of(recipe.root()),
                        Some(Fidelity::Exact),
                        "{context}: {:?}",
                        out.report.diagnostics
                    );
                    assert_eq!(out.bodies.len(), 1);
                    let components = component_geometry(&out.bodies[0].body.mesh);
                    let (count, volume) = expected(w, h, bar, distinct, stage);
                    assert_eq!(components.len(), count, "{context}");
                    for &(actual, _, _) in &components {
                        let per_component = volume / f64::from(u32::try_from(count).unwrap());
                        assert!(
                            (actual - per_component).abs() < per_component * 1e-5,
                            "{context}: volume={actual}, expected={per_component}"
                        );
                    }
                    if stage == 2 {
                        unshifted = components.clone();
                    }
                    if stage == 3 {
                        for before in &unshifted {
                            let after = components
                                .iter()
                                .find(|after| {
                                    [0.05, 0.03, 0.0].into_iter().enumerate().all(|(i, shift)| {
                                        (before.1[i] + shift - after.1[i]).abs() < position_tolerance
                                    })
                                })
                                .unwrap_or_else(|| {
                                    panic!("{context}: missing translated component {before:?}; actual={components:?}")
                                });
                            assert!((before.0 - after.0).abs() < before.0 * 1e-5, "{context}");
                            for (i, shift) in [0.05, 0.03, 0.0].into_iter().enumerate() {
                                assert!(
                                    (before.1[i] + shift - after.1[i]).abs() < position_tolerance,
                                    "{context}"
                                );
                                assert!(
                                    (before.2[i] + shift - after.2[i]).abs() < position_tolerance,
                                    "{context}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
