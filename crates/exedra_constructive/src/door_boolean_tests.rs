// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    builders,
    evaluate::{Fidelity, evaluate},
    ir::{CapMode, CsgOp, NodeKind, Placement3, Recipe, RecipeBuilder, SlotId},
    tessellate::EvalPolicy,
};
use alloc::vec;
use exedra_math::{cross, dot, sub};
use exedra_mesh::{FaceId, FaceTriangulation};

fn recessed_panel(scale: f64, holes: bool, chained: bool) -> Recipe {
    let mut b = RecipeBuilder::new();
    let finish = b.material_slot("finish");
    let profile =
        b.add_profile(builders::rounded_rect(0.6 * scale, 0.8 * scale, 0.02 * scale).unwrap());
    let panel = b
        .add(NodeKind::Extrude {
            profile,
            height: 0.02 * scale,
            caps: CapMode::Both,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let profile =
        b.add_profile(builders::rounded_rect(0.5 * scale, 0.7 * scale, 0.012 * scale).unwrap());
    let pocket = b
        .add(NodeKind::Extrude {
            profile,
            height: 0.012 * scale,
            caps: CapMode::Both,
            placement: Placement3::translate(0.05 * scale, 0.05 * scale, 0.012 * scale),
        })
        .unwrap();
    let mut operands = vec![panel, pocket];
    if holes {
        for x in [0.025, 0.575] {
            let profile = b.add_profile(builders::circle(0.0025 * scale).unwrap());
            operands.push(
                b.add(NodeKind::Extrude {
                    profile,
                    height: 0.04 * scale,
                    caps: CapMode::Both,
                    placement: Placement3::translate(x * scale, 0.4 * scale, -0.01 * scale),
                })
                .unwrap(),
            );
        }
    }
    let root = if chained {
        operands[1..].iter().fold(panel, |left, &right| {
            b.with_material(finish)
                .add(NodeKind::Csg {
                    op: CsgOp::Difference,
                    operands: vec![left, right],
                })
                .unwrap()
        })
    } else {
        b.with_material(finish)
            .add(NodeKind::Csg {
                op: CsgOp::Difference,
                operands,
            })
            .unwrap()
    };
    b.finish(root).unwrap()
}

#[test]
fn rounded_recesses_and_drilled_holes_preserve_solid_geometry() {
    for scale in [1.0, 1000.0] {
        for holes in [false, true] {
            for chained in [false, true] {
                let recipe = recessed_panel(scale, holes, chained);
                let mut policy = EvalPolicy::default();
                policy.discretize.chord_tolerance = 0.00002 * scale;
                let result = evaluate(&recipe, &policy).unwrap();
                assert_eq!(
                    result.report.fidelity_of(recipe.root()),
                    Some(Fidelity::Exact),
                    "scale={scale} holes={holes} chained={chained}: {:?}",
                    result.report.diagnostics
                );
                assert_eq!(result.bodies.len(), 1);
                assert_eq!(result.bodies[0].material, Some(SlotId(0)));
                let body = &result.bodies[0].body;
                let mesh = &body.mesh;
                assert!(mesh.validate_deep().is_empty());
                body.source_map.check(mesh).unwrap();
                let mut volume = 0.0;
                for face in mesh.faces() {
                    assert!(
                        mesh.face_loop(face)
                            .all(|he| mesh.face(mesh.twin(he).unwrap()) != Some(FaceId::OUTSIDE)),
                        "open face: scale={scale} holes={holes} chained={chained}"
                    );
                    let (triangles, fallback) =
                        mesh.face_triangles_counted(face, FaceTriangulation::Robust);
                    assert!(
                        !fallback,
                        "triangulation fallback: scale={scale} holes={holes} chained={chained} face={face:?}"
                    );
                    for t in triangles {
                        let [a, b, c] = t.map(|he| {
                            mesh.vertex_position(mesh.to_vertex(he).unwrap())
                                .unwrap()
                                .map(f64::from)
                        });
                        assert_ne!(
                            cross(sub(b, a), sub(c, a)),
                            [0.0; 3],
                            "degenerate face: scale={scale} holes={holes} chained={chained} face={face:?}"
                        );
                        volume += dot(a, cross(b, c)) / 6.0;
                    }
                }
                let pi = core::f64::consts::PI;
                let mut expected = (0.6 * 0.8 - (4.0 - pi) * 0.02_f64.powi(2)) * 0.02
                    - (0.5 * 0.7 - (4.0 - pi) * 0.012_f64.powi(2)) * 0.008;
                if holes {
                    expected -= 2.0 * pi * 0.0025_f64.powi(2) * 0.02;
                }
                assert!(
                    (volume / scale.powi(3) - expected).abs() < expected * 1e-5,
                    "volume={volume} expected={expected} scale={scale} holes={holes} chained={chained}"
                );
            }
        }
    }
}
