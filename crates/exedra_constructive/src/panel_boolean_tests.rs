// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::builders;
use crate::evaluate::{Fidelity, evaluate};
use crate::ir::{CapMode, CsgOp, NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder};
use crate::tessellate::EvalPolicy;
use alloc::{format, vec, vec::Vec};

#[derive(Clone, Copy, Debug)]
enum Front {
    Square,
    Chamfer,
    Rounded,
}

fn shelf_difference(rotated: bool, order: &[usize], chained: bool, front: Front) -> Recipe {
    let mut b = RecipeBuilder::new();
    let slot = b.material_slot("finish");
    let (profile, height, placement) = match front {
        Front::Square => (
            builders::rect(0.6, 0.56).unwrap(),
            0.018,
            Placement3::translate(0.0, 0.0, 0.351),
        ),
        Front::Chamfer | Front::Rounded => {
            use crate::profile::{Loop2, Profile2, Seg2};
            let corner = match front {
                Front::Rounded => Seg2::arc((0.0, 0.014), libm::tan(core::f64::consts::FRAC_PI_8)),
                _ => Seg2::line((0.0, 0.014)),
            };
            let profile = Profile2::simple(
                Loop2::new(vec![
                    Seg2::line((0.56, 0.0)),
                    Seg2::line((0.56, 0.018)),
                    Seg2::line((0.004, 0.018)),
                    corner,
                    Seg2::line((0.0, 0.0)),
                ])
                .unwrap(),
            )
            .unwrap();
            (
                profile,
                0.6,
                Placement3::from_axes(
                    [0.0, 1.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 0.0, 0.351],
                ),
            )
        }
    };
    let profile = b.add_profile(profile);
    let shelf = b
        .add(NodeKind::Extrude {
            profile,
            height,
            caps: CapMode::Both,
            placement,
        })
        .unwrap();
    let walls = if rotated {
        [
            ([0.56, 0.018, 0.684], [0.6, 0.0, 0.018], 90),
            ([0.6, 0.008, 0.684], [0.6, 0.56, 0.018], 180),
            ([0.56, 0.018, 0.684], [0.0, 0.56, 0.018], -90),
        ]
    } else {
        [
            ([0.018, 0.56, 0.684], [0.582, 0.0, 0.018], 0),
            ([0.6, 0.008, 0.684], [0.0, 0.552, 0.018], 0),
            ([0.018, 0.56, 0.684], [0.0, 0.0, 0.018], 0),
        ]
    };
    let mut operands = vec![shelf];
    for &index in order {
        let (size, at, degrees) = if index < 3 {
            walls[index]
        } else {
            [
                ([0.018, 0.56, 0.684], [0.6, 0.0, 0.018], 0),
                ([0.6, 0.008, 0.684], [0.0, 0.56, 0.018], 0),
                ([0.018, 0.56, 0.684], [-0.018, 0.0, 0.018], 0),
            ][index - 3]
        };
        operands.push(
            b.add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box { size },
                placement: {
                    let (s, c) = match degrees {
                        90 => (1.0, 0.0),
                        180 => (0.0, -1.0),
                        -90 => (-1.0, 0.0),
                        _ => (0.0, 1.0),
                    };
                    Placement3::from_axes([c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0], at)
                },
            })
            .unwrap(),
        );
    }
    let root = if chained {
        operands[1..].iter().fold(shelf, |left, &right| {
            b.with_material(slot)
                .add(NodeKind::Csg {
                    op: CsgOp::Difference,
                    operands: vec![left, right],
                })
                .unwrap()
        })
    } else {
        b.with_material(slot)
            .add(NodeKind::Csg {
                op: CsgOp::Difference,
                operands,
            })
            .unwrap()
    };
    b.finish(root).unwrap()
}

#[test]
fn coplanar_panel_differences_are_exact() {
    let mut failures = Vec::new();
    for rotated in [false, true] {
        for order in [
            &[0][..],
            &[1],
            &[2],
            &[0, 1, 2],
            &[0, 2, 1],
            &[1, 0, 2],
            &[1, 2, 0],
            &[2, 0, 1],
            &[2, 1, 0],
            &[3],
            &[4],
            &[5],
            &[3, 4, 5],
            &[0, 4, 2],
        ] {
            for chained in [false, true] {
                let recipe = shelf_difference(rotated, order, chained, Front::Square);
                let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
                if result.report.fidelity_of(recipe.root()) != Some(Fidelity::Exact)
                    || result.bodies.len() != 1
                {
                    failures.push(format!(
                        "rotated={rotated} order={order:?} chained={chained}: {:?}",
                        result.report.diagnostics
                    ));
                } else {
                    let mesh = &result.bodies[0].body.mesh;
                    assert!(
                        result.bodies[0]
                            .body
                            .mesh
                            .faces()
                            .all(|face| result.bodies[0].material_for_face(face)
                                == Some(crate::ir::SlotId(0)))
                    );
                    assert!(mesh.validate_deep().is_empty());
                    assert!(
                        mesh.faces().all(|face| mesh
                            .face_loop(face)
                            .all(|he| mesh.face(mesh.twin(he).unwrap())
                                != Some(exedra_mesh::FaceId::OUTSIDE))),
                        "open square rotated={rotated} order={order:?} chained={chained}"
                    );
                    let min = [if order.contains(&2) { 0.018 } else { 0.0 }, 0.0, 0.351];
                    let max = [
                        if order.contains(&0) { 0.582 } else { 0.6 },
                        if order.contains(&1) { 0.552 } else { 0.56 },
                        0.369,
                    ];
                    let (tri, _) = mesh.to_trimesh(&exedra_mesh::ExtractParams {
                        face_triangulation: exedra_mesh::FaceTriangulation::Robust,
                        ..Default::default()
                    });
                    let mut actual_min = [f64::INFINITY; 3];
                    let mut actual_max = [f64::NEG_INFINITY; 3];
                    for vertex in mesh.vertices() {
                        let p = mesh.vertex_position(vertex).unwrap();
                        for axis in 0..3 {
                            actual_min[axis] = actual_min[axis].min(f64::from(p[axis]));
                            actual_max[axis] = actual_max[axis].max(f64::from(p[axis]));
                        }
                    }
                    for axis in 0..3 {
                        assert!(
                            (actual_min[axis] - min[axis]).abs() < 1e-7,
                            "{actual_min:?}"
                        );
                        assert!(
                            (actual_max[axis] - max[axis]).abs() < 1e-7,
                            "{actual_max:?}"
                        );
                    }
                    let mut volume = 0.0;
                    for indices in tri.indices.chunks_exact(3) {
                        let p = indices
                            .iter()
                            .map(|&i| tri.positions[i as usize].map(f64::from))
                            .collect::<Vec<_>>();
                        let normal = exedra_math::cross(
                            exedra_math::sub(p[1], p[0]),
                            exedra_math::sub(p[2], p[0]),
                        );
                        assert_ne!(
                            normal, [0.0; 3],
                            "rotated={rotated} order={order:?} chained={chained}: triangle={p:?}"
                        );
                        volume += exedra_math::dot(p[0], exedra_math::cross(p[1], p[2])) / 6.0;
                    }
                    let expected = (max[0] - min[0]) * (max[1] - min[1]) * (max[2] - min[2]);
                    assert!(
                        (volume - expected).abs() < expected * 1e-5,
                        "rotated={rotated} order={order:?} chained={chained}: volume={volume}, expected={expected}"
                    );
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn shaped_fronts_survive_panel_cuts() {
    for front in [Front::Chamfer, Front::Rounded] {
        for tolerance in [0.0001, 0.00001] {
            for chained in [false, true] {
                let recipe = shelf_difference(false, &[0, 1, 2], chained, front);
                let mut policy = EvalPolicy::default();
                policy.discretize.chord_tolerance = tolerance;
                let result = evaluate(&recipe, &policy).unwrap();
                assert_eq!(
                    result.report.fidelity_of(recipe.root()),
                    Some(Fidelity::Exact),
                    "{front:?} tolerance={tolerance} chained={chained}: {:?}",
                    result.report.diagnostics
                );
                assert_eq!(result.bodies.len(), 1);
                assert!(
                    result.bodies[0]
                        .body
                        .mesh
                        .faces()
                        .all(|face| result.bodies[0].material_for_face(face)
                            == Some(crate::ir::SlotId(0)))
                );
                let mesh = &result.bodies[0].body.mesh;
                assert!(mesh.validate_deep().is_empty());
                assert!(
                    mesh.faces().all(|face| mesh
                        .face_loop(face)
                        .all(|he| mesh.face(mesh.twin(he).unwrap())
                            != Some(exedra_mesh::FaceId::OUTSIDE))),
                    "open mesh: {front:?} tolerance={tolerance} chained={chained}"
                );
                let (tri, _) = mesh.to_trimesh(&exedra_mesh::ExtractParams {
                    face_triangulation: exedra_mesh::FaceTriangulation::Robust,
                    ..Default::default()
                });
                let mut bounds = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                for vertex in mesh.vertices() {
                    let p = mesh.vertex_position(vertex).unwrap().map(f64::from);
                    for (axis, value) in p.into_iter().enumerate() {
                        bounds.0[axis] = bounds.0[axis].min(value);
                        bounds.1[axis] = bounds.1[axis].max(value);
                    }
                }
                for (actual, expected) in bounds
                    .0
                    .into_iter()
                    .chain(bounds.1)
                    .zip([0.018, 0.0, 0.351, 0.582, 0.552, 0.369])
                {
                    assert!(
                        (actual - expected).abs() < 1e-7,
                        "{front:?}: bounds={bounds:?}"
                    );
                }
                let mut volume = 0.0;
                for indices in tri.indices.chunks_exact(3) {
                    let [a, b, c] = [indices[0], indices[1], indices[2]]
                        .map(|i| tri.positions[i as usize].map(f64::from));
                    assert_ne!(
                        exedra_math::cross(exedra_math::sub(b, a), exedra_math::sub(c, a)),
                        [0.0; 3]
                    );
                    volume += exedra_math::dot(a, exedra_math::cross(b, c)) / 6.0;
                }
                let removed_area = 0.004_f64.powi(2)
                    * match front {
                        Front::Chamfer => 0.5,
                        Front::Rounded => 1.0 - core::f64::consts::FRAC_PI_4,
                        Front::Square => unreachable!(),
                    };
                let expected = 0.564 * (0.552 * 0.018 - removed_area);
                let allowed_error = expected * 1e-5 + 0.564 * 0.008 * tolerance;
                assert!(
                    (volume - expected).abs() < allowed_error,
                    "{front:?} tolerance={tolerance} chained={chained}: volume={volume}, expected={expected}"
                );
            }
        }
    }
}
