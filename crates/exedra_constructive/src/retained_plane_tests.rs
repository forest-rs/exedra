// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    builders,
    cache::{EvalCache, policy_fingerprint},
    evaluate::{evaluate, evaluate_with_cache},
    ir::{CapMode, NodeKind, Placement3, Plane3, PlaneSide, Recipe, RecipeBuilder, SlotId},
    section::{CutCap, SectionError},
    tessellate::{EvalPolicy, Feature, TessellateError},
    text,
};
use alloc::vec::Vec;

fn recipe(side: PlaneSide, distance: f64, parent: Placement3) -> Recipe {
    let mut b = RecipeBuilder::new();
    let wall = b.material_slot("wall");
    let cap = b.material_slot("cut");
    let profile = b.add_profile(builders::rect(2.0, 3.0).unwrap());
    let child = b
        .with_material(wall)
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 4.0,
            caps: CapMode::Both,
        })
        .unwrap();
    let child = b
        .add(NodeKind::Transform {
            child,
            xf: Placement3::translate(0.0, 0.0, 10.0),
        })
        .unwrap();
    let cut = b
        .add(NodeKind::PlaneCut {
            child,
            plane: Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance,
            },
            side,
            cap: CutCap {
                region: 100,
                material: Some(cap),
            },
        })
        .unwrap();
    let root = b
        .add(NodeKind::Transform {
            child: cut,
            xf: parent,
        })
        .unwrap();
    b.finish(root).unwrap()
}

fn z_bounds(evaluation: &crate::evaluate::Evaluation) -> [f32; 2] {
    let mesh = &evaluation.bodies[0].body.mesh;
    mesh.vertices()
        .map(|v| mesh.vertex_position(v).unwrap()[2])
        .fold([f32::INFINITY, f32::NEG_INFINITY], |[lo, hi], z| {
            [lo.min(z), hi.max(z)]
        })
}

#[test]
fn cuts_respect_nested_transforms_side_materials_and_cache() {
    let mut cache = EvalCache::new();
    for (side, expected) in [
        (PlaneSide::Negative, [110.0, 111.75]),
        (PlaneSide::Positive, [111.75, 114.0]),
    ] {
        let recipe = recipe(side, 11.75, Placement3::translate(0.0, 0.0, 100.0));
        let policy = EvalPolicy::default();
        let cold = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
        let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
        assert_eq!(z_bounds(&cold), expected);
        assert_eq!(z_bounds(&warm), expected);
        assert_eq!(warm.report.counters.tessellations, 0);
        assert_eq!(warm.report.counters.cache_hits, 2);
        let placed = &warm.bodies[0];
        placed.body.source_map.check(&placed.body.mesh).unwrap();
        assert!(placed.body.mesh.validate_deep().is_empty());
        let mut caps = 0;
        for face in placed.body.mesh.faces() {
            if placed.body.source_map.face_feature(face) == Some(Feature::PlaneCutCap) {
                caps += 1;
                assert_eq!(
                    placed
                        .body
                        .mesh
                        .attrs()
                        .dense(exedra_mesh::attr::FACE_REGION)
                        .unwrap()
                        .get(face.into()),
                    Some(&100)
                );
                assert_eq!(placed.material_for_face(face), Some(SlotId(1)));
            } else {
                assert_eq!(placed.material_for_face(face), Some(SlotId(0)));
            }
        }
        assert!(caps > 0);
        let text = text::dump_recipe(&recipe);
        assert_eq!(text::dump_recipe(&text::parse_recipe(&text).unwrap()), text);
        #[cfg(feature = "serde")]
        {
            let dto = crate::interchange::to_dto(&recipe);
            let encoded = serde_json::to_string(&dto).unwrap();
            let decoded =
                crate::interchange::from_dto(&serde_json::from_str(&encoded).unwrap()).unwrap();
            assert_eq!(decoded.recipe_fingerprint(), recipe.recipe_fingerprint());
        }
    }
}

#[test]
fn disjoint_cuts_are_empty_and_contacts_and_budgets_are_typed_failures() {
    let policy = EvalPolicy::default();
    assert!(
        evaluate(
            &recipe(PlaneSide::Negative, 9.0, Placement3::IDENTITY),
            &policy
        )
        .unwrap()
        .bodies
        .is_empty()
    );
    assert!(matches!(
        evaluate(
            &recipe(PlaneSide::Negative, 10.0, Placement3::IDENTITY),
            &policy
        )
        .unwrap_err()
        .error,
        TessellateError::PlaneCut(SectionError::AmbiguousContact)
    ));
    let mut small = policy;
    small.section.max_triangles = 1;
    assert!(matches!(
        evaluate(
            &recipe(PlaneSide::Negative, 11.75, Placement3::IDENTITY),
            &small
        )
        .unwrap_err()
        .error,
        TessellateError::PlaneCut(SectionError::BudgetExceeded)
    ));
    let base = policy_fingerprint(&policy);
    for policy in [
        small,
        EvalPolicy {
            section: crate::section::SectionPolicy {
                distance_tolerance: 1e-5,
                ..policy.section
            },
            ..policy
        },
    ] {
        assert_ne!(base, policy_fingerprint(&policy));
    }
}

#[test]
fn retained_plane_extrusion_round_trips_and_rebinds_target_with_holes() {
    let make = |distance| {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::ring(2.0, 0.75).unwrap());
        let root = b
            .add(NodeKind::ExtrudeToPlane {
                profile,
                placement: Placement3::translate(0.0, 0.0, 2.0),
                plane: Plane3 {
                    normal: [0.1, -0.15, 1.0],
                    distance,
                },
            })
            .unwrap();
        b.finish(root).unwrap()
    };
    let mut cache = EvalCache::new();
    let a = make(5.0);
    let b = make(6.0);
    assert_ne!(a.recipe_fingerprint(), b.recipe_fingerprint());
    for recipe in [&a, &b] {
        let cold = evaluate_with_cache(recipe, &EvalPolicy::default(), &mut cache).unwrap();
        let warm = evaluate_with_cache(recipe, &EvalPolicy::default(), &mut cache).unwrap();
        assert_eq!(warm.report.counters.tessellations, 0);
        assert_eq!(warm.report.counters.cache_hits, 1);
        let positions = |e: &crate::evaluate::Evaluation| {
            e.bodies[0]
                .body
                .mesh
                .vertices()
                .map(|v| *e.bodies[0].body.mesh.vertex_position(v).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(positions(&cold), positions(&warm));
        let section = crate::section::section_body(
            &warm.bodies[0].body,
            Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance: 3.3,
            },
            &EvalPolicy::default().section,
        )
        .unwrap();
        assert_eq!(section.regions[0].holes.len(), 1);
        assert_eq!(
            text::parse_recipe(&text::dump_recipe(recipe))
                .unwrap()
                .recipe_fingerprint(),
            recipe.recipe_fingerprint()
        );
        #[cfg(feature = "serde")]
        assert_eq!(
            crate::interchange::from_dto(&crate::interchange::to_dto(recipe))
                .unwrap()
                .recipe_fingerprint(),
            recipe.recipe_fingerprint()
        );
    }
    assert!(
        matches!(evaluate(&make(1.0), &EvalPolicy::default()).unwrap_err().error,
        TessellateError::ExtrudeToPlane(error) if *error == crate::extrude::ExtrudeToPlaneError::NotForward)
    );
}

#[test]
fn grouped_bodies_keep_defaults_distinct_and_refused_children_do_not_leak() {
    use crate::ir::{NodeId, RecipeError};
    let mut b = RecipeBuilder::new();
    let red = b.material_slot("red");
    let blue = b.material_slot("blue");
    let profile = b.add_profile(builders::rect(2.0, 3.0).unwrap());
    let a = b
        .with_material(red)
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 4.0,
            caps: CapMode::Both,
        })
        .unwrap();
    let other = b
        .with_material(blue)
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::translate(4.0, 0.0, 0.0),
            height: 4.0,
            caps: CapMode::Both,
        })
        .unwrap();
    let group = b
        .add(NodeKind::Group {
            children: alloc::vec![a, other],
        })
        .unwrap();
    let plane = Plane3 {
        normal: [0.0, 0.0, 1.0],
        distance: 1.75,
    };
    assert!(matches!(
        b.add(NodeKind::PlaneCut {
            child: group,
            plane,
            side: PlaneSide::Negative,
            cap: CutCap {
                region: 100,
                material: Some(SlotId(99))
            }
        }),
        Err(RecipeError::UnknownSlot { slot: 99 })
    ));
    let root = b
        .add(NodeKind::PlaneCut {
            child: group,
            plane,
            side: PlaneSide::Negative,
            cap: CutCap {
                region: 100,
                material: None,
            },
        })
        .unwrap();
    let evaluation = evaluate(&b.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
    assert_eq!(evaluation.bodies.len(), 2);
    for (body, slot) in evaluation.bodies.into_iter().zip([red, blue]) {
        assert!(body.body.mesh.boundary_loops().unwrap().is_empty());
        assert!(
            body.body
                .mesh
                .faces()
                .all(|face| body.material_for_face(face) == Some(slot))
        );
    }
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(builders::rect(2.0, 3.0).unwrap());
    let open = b
        .add(NodeKind::PlanarFace {
            profile,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let refused = b
        .add(NodeKind::Stretch {
            child: open,
            plane: Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 0.75,
            },
            length: 1.0,
        })
        .unwrap();
    let root = b
        .add(NodeKind::PlaneCut {
            child: refused,
            plane,
            side: PlaneSide::Negative,
            cap: CutCap {
                region: 100,
                material: None,
            },
        })
        .unwrap();
    let failure = evaluate(&b.finish(root).unwrap(), &EvalPolicy::default()).unwrap_err();
    assert_eq!(failure.node, NodeId(root.0));
    assert_eq!(failure.error, TessellateError::IncompletePlaneCut);
}

#[test]
fn reflected_scaled_parents_place_completed_extrusion_and_cut() {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(builders::rect(2.0, 3.0).unwrap());
    let leaf = b
        .add(NodeKind::ExtrudeToPlane {
            profile,
            placement: Placement3::IDENTITY,
            plane: Plane3 {
                normal: [0.2, -0.1, 1.0],
                distance: 4.0,
            },
        })
        .unwrap();
    let cut = b
        .add(NodeKind::PlaneCut {
            child: leaf,
            plane: Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance: 1.75,
            },
            side: PlaneSide::Positive,
            cap: CutCap {
                region: 100,
                material: None,
            },
        })
        .unwrap();
    let root = b
        .add(NodeKind::Transform {
            child: cut,
            xf: Placement3 {
                rows: [
                    [-2.0, 0.0, 0.0, 10.0],
                    [0.0, 3.0, 0.0, 0.0],
                    [0.0, 0.0, 4.0, 100.0],
                ],
            },
        })
        .unwrap();
    let evaluation = evaluate(&b.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
    assert_eq!(z_bounds(&evaluation)[0], 107.0);
    let body = &evaluation.bodies[0].body;
    assert!(body.mesh.validate_deep().is_empty());
    assert!(body.mesh.boundary_loops().unwrap().is_empty());
    let mut volume = 0.0;
    let mut triangles = Vec::new();
    for face in body.mesh.faces() {
        assert!(!body.mesh.face_triangles_into(
            face,
            exedra_mesh::FaceTriangulation::Robust,
            &mut triangles
        ));
        for triangle in &triangles {
            let p = triangle.map(|h| {
                body.mesh
                    .vertex_position(body.mesh.to_vertex(h).unwrap())
                    .unwrap()
                    .map(f64::from)
            });
            volume += exedra_math::dot(p[0], exedra_math::cross(p[1], p[2])) / 6.0;
        }
    }
    assert!(volume > 0.0, "reflections must retain outward winding");
}
