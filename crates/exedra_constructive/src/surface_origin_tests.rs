// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    builders,
    cache::EvalCache,
    clearance::{BoundaryPolicy, PlanarPatch},
    evaluate::{Severity, evaluate, evaluate_with_cache},
    ir::{CapMode, CsgOp, NodeKind, Placement3, Plane3, Recipe, RecipeBuilder},
    tessellate::{EvalPolicy, Feature, TessellateError},
    workplane::{SurfaceSelector, WorkplaneAttachment, WorkplaneError, WorkplanePolicy},
};

fn attachment() -> WorkplaneAttachment {
    WorkplaneAttachment {
        surface: SurfaceSelector::SourceEndCap("panel".into()),
        anchor: [0.25, 0.25, 0.0],
        projection: [0.0, 0.0, 1.0],
        x_direction: [1.0, 0.0, 0.0],
    }
}

fn panel(slope: f64, cut: u8, duplicate: bool, attach: bool, reflect: bool) -> Recipe {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(builders::rect_from_corner(4.0, 3.0).unwrap());
    let source = b.source_ref("panel");
    let panel = b
        .with_source(source)
        .add(NodeKind::ExtrudeToPlane {
            profile,
            placement: Placement3::IDENTITY,
            plane: Plane3 {
                normal: [-slope, 0.0, 1.0],
                distance: 1.0,
            },
        })
        .unwrap();
    if cut == 3 {
        b.with_source(source)
            .add(NodeKind::PlanarFace {
                profile,
                placement: Placement3::IDENTITY,
            })
            .unwrap();
    }
    let mut root = panel;
    for (width, height, x, y, z, depth) in match cut {
        1 => alloc::vec![(6.0, 5.0, -1.0, -1.0, 0.5, 3.0)], // destroys end cap
        2 => alloc::vec![(0.5, 5.0, 1.5, -1.0, -1.0, 5.0)], // splits cap
        _ => alloc::vec![
            (0.5, 0.5, 1.0, 1.0, -1.0, 5.0),
            (0.5, 0.5, 2.5, 1.0, -1.0, 5.0)
        ],
    } {
        let hole = b.add_profile(builders::rect_from_corner(width, height).unwrap());
        if duplicate {
            b.with_source(source);
        }
        let hole = b
            .add(NodeKind::Extrude {
                profile: hole,
                placement: Placement3::translate(x, y, z),
                height: depth,
                caps: CapMode::Both,
            })
            .unwrap();
        root = b
            .add(NodeKind::Csg {
                op: CsgOp::Difference,
                operands: alloc::vec![root, hole],
            })
            .unwrap();
    }
    if attach {
        let profile = b.add_profile(builders::rect_from_corner(0.1, 0.1).unwrap());
        let child = b
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 0.1,
                caps: CapMode::Both,
            })
            .unwrap();
        root = b
            .add(NodeKind::OnWorkplane {
                support: root,
                child,
                attachment: attachment(),
            })
            .unwrap();
    }
    if reflect {
        root = b
            .add(NodeKind::Instance {
                of: root,
                placement: Placement3 {
                    rows: [
                        [-1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                },
            })
            .unwrap();
    }
    b.finish(root).unwrap()
}

#[test]
fn nested_drilling_retains_authored_cap_holes_and_warm_cache_evidence() {
    let mut cache = EvalCache::new();
    for slope in [0.0, 0.2, -0.1] {
        let recipe = panel(slope, 0, false, false, false);
        for warm in [false, true] {
            let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
            assert!(
                !result
                    .report
                    .diagnostics
                    .iter()
                    .any(|d| d.severity >= Severity::Error)
            );
            if warm {
                assert_eq!(result.report.counters.tessellations, 0);
            }
            let body = &result.bodies[0].body;
            let plane = attachment()
                .resolve(body, &WorkplanePolicy::default())
                .unwrap();
            assert!((plane.frame().rows[2][3] - (1.0 + 0.25 * slope)).abs() < 1e-6);
            for &face in plane.faces() {
                let origin = body.source_map.surface_origin(face).unwrap();
                assert_eq!(origin.source.as_deref(), Some("panel"));
                assert_eq!(origin.feature, Feature::CapEnd);
                assert_eq!(
                    body.source_map.face_feature(face),
                    Some(Feature::BooleanFace { operand: 0 })
                );
            }
            let patch =
                PlanarPatch::from_workplane(body, &plane, &BoundaryPolicy::default()).unwrap();
            assert_eq!(patch.loops().len(), 3);
        }
        let attached = panel(slope, 0, false, true, false);
        let result = evaluate_with_cache(&attached, &EvalPolicy::default(), &mut cache).unwrap();
        assert!(
            (result.report.attachments[0].frame.rows[2][3] - (1.0 + 0.25 * slope)).abs() < 1e-6
        );
        let text = crate::text::dump_recipe(&attached);
        assert_eq!(
            crate::text::parse_recipe(&text)
                .unwrap()
                .recipe_fingerprint(),
            attached.recipe_fingerprint()
        );
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string(&crate::interchange::to_dto(&attached)).unwrap();
            let restored =
                crate::interchange::from_dto(&serde_json::from_str(&json).unwrap()).unwrap();
            assert_eq!(restored.recipe_fingerprint(), attached.recipe_fingerprint());
        }
    }
}

#[test]
fn removed_split_and_duplicate_targets_fail_without_fallback() {
    let mut cache = EvalCache::new();
    for (cut, duplicate, expected) in [
        (1, false, WorkplaneError::EmptySelection),
        (2, false, WorkplaneError::AmbiguousSelection),
        (0, true, WorkplaneError::AmbiguousSelection),
    ] {
        evaluate_with_cache(
            &panel(0.0, 0, false, false, false),
            &EvalPolicy::default(),
            &mut cache,
        )
        .unwrap();
        for _ in 0..2 {
            let result = evaluate_with_cache(
                &panel(0.0, cut, duplicate, false, false),
                &EvalPolicy::default(),
                &mut cache,
            )
            .unwrap();
            assert_eq!(
                attachment()
                    .resolve(&result.bodies[0].body, &WorkplanePolicy::default())
                    .unwrap_err()
                    .kind,
                expected
            );
            let error = evaluate_with_cache(
                &panel(0.0, cut, duplicate, true, false),
                &EvalPolicy::default(),
                &mut cache,
            )
            .unwrap_err();
            assert!(
                matches!(error.error, TessellateError::Attachment(failure) if failure.kind == expected)
            );
        }
    }
}

#[test]
fn reflected_boolean_retains_authored_cap_and_outward_frame() {
    let result = evaluate(&panel(0.2, 0, false, false, true), &EvalPolicy::default()).unwrap();
    let body = &result.bodies[0].body;
    let attachment = WorkplaneAttachment {
        anchor: [-0.25, 0.25, 0.0],
        ..attachment()
    };
    let plane = attachment
        .resolve(body, &WorkplanePolicy::default())
        .unwrap();
    assert!((plane.frame().rows[2][3] - 1.05).abs() < 1e-6);
    assert!(plane.frame().rows[2][2] > 0.9);
    assert_eq!(
        PlanarPatch::from_workplane(body, &plane, &BoundaryPolicy::default())
            .unwrap()
            .loops()
            .len(),
        3
    );
}

#[test]
fn union_intersection_and_difference_keep_source_roles_separate() {
    for op in [CsgOp::Union, CsgOp::Intersection, CsgOp::Difference] {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect_from_corner(4.0, 3.0).unwrap());
        let label = b.source_ref("panel");
        let panel = b
            .with_source(label)
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let profile = b.add_profile(builders::rect_from_corner(1.0, 1.0).unwrap());
        let label = b.source_ref("tool");
        let tool = b
            .with_source(label)
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::translate(1.0, 1.0, 0.5),
                height: 2.0,
                caps: CapMode::Both,
            })
            .unwrap();
        let root = b
            .add(NodeKind::Csg {
                op,
                operands: alloc::vec![panel, tool],
            })
            .unwrap();
        let result = evaluate(&b.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
        assert_eq!(result.bodies.len(), 1);
        let body = &result.bodies[0].body;
        let plane = attachment()
            .resolve(body, &WorkplanePolicy::default())
            .unwrap();
        assert!((plane.frame().rows[2][3] - 1.0).abs() < 1e-6);
        let patch = PlanarPatch::from_workplane(body, &plane, &BoundaryPolicy::default()).unwrap();
        assert_eq!(
            patch.loops().len(),
            if op == CsgOp::Intersection { 1 } else { 2 }
        );
        if op == CsgOp::Difference {
            let bottom = WorkplaneAttachment {
                surface: SurfaceSelector::SourceStartCap("tool".into()),
                ..attachment()
            }
            .resolve(body, &WorkplanePolicy::default())
            .unwrap();
            assert!((bottom.frame().rows[2][3] - 0.5).abs() < 1e-6);
            assert!(
                bottom.frame().rows[2][2] > 0.99,
                "subtracted start cap reverses normal but keeps ancestry"
            );
        }
    }
}

#[test]
fn qualified_selectors_round_trip_without_source_table_indices() {
    for source in ["", "panel with spaces", "cabinet/左", "escaped\\label"] {
        for surface in [
            SurfaceSelector::SourceStartCap(source.into()),
            SurfaceSelector::SourceEndCap(source.into()),
        ] {
            let mut b = RecipeBuilder::new();
            let p = b.add_profile(builders::rect_from_corner(1.0, 1.0).unwrap());
            let support = b
                .add(NodeKind::Extrude {
                    profile: p,
                    placement: Placement3::IDENTITY,
                    height: 1.0,
                    caps: CapMode::Both,
                })
                .unwrap();
            let root = b
                .add(NodeKind::OnWorkplane {
                    support,
                    child: support,
                    attachment: WorkplaneAttachment {
                        surface,
                        ..attachment()
                    },
                })
                .unwrap();
            let recipe = b.finish(root).unwrap();
            let text = crate::text::dump_recipe(&recipe);
            assert_eq!(
                crate::text::parse_recipe(&text)
                    .unwrap()
                    .recipe_fingerprint(),
                recipe.recipe_fingerprint()
            );
        }
    }
}

#[test]
fn unused_duplicate_labels_do_not_change_cached_attachment_semantics() {
    let plain = panel(0.0, 0, false, false, false);
    let unused = panel(0.0, 3, false, false, false);
    assert_eq!(plain.recipe_fingerprint(), unused.recipe_fingerprint());
    let mut cache = EvalCache::new();
    for recipe in [&plain, &unused, &plain] {
        let result = evaluate_with_cache(recipe, &EvalPolicy::default(), &mut cache).unwrap();
        attachment()
            .resolve(&result.bodies[0].body, &WorkplanePolicy::default())
            .unwrap();
    }
}

#[test]
fn naming_context_changes_recipe_identity_without_invalidating_subtree_geometry() {
    let make = |duplicate| {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect_from_corner(2.0, 3.0).unwrap());
        let label = b.source_ref("panel");
        let kind = NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 1.0,
            caps: CapMode::Both,
        };
        let first = b.with_source(label).add(kind.clone()).unwrap();
        let second = if duplicate {
            b.with_source(label).add(kind).unwrap()
        } else {
            first
        };
        let root = b
            .add(NodeKind::Group {
                children: alloc::vec![first, second],
            })
            .unwrap();
        b.finish(root).unwrap()
    };
    let shared = make(false);
    let duplicate = make(true);
    assert_eq!(
        shared.fingerprint(shared.root()),
        duplicate.fingerprint(duplicate.root())
    );
    assert_ne!(shared.recipe_fingerprint(), duplicate.recipe_fingerprint());
    let mut cache = EvalCache::new();
    evaluate_with_cache(&shared, &EvalPolicy::default(), &mut cache).unwrap();
    let result = evaluate_with_cache(&duplicate, &EvalPolicy::default(), &mut cache).unwrap();
    assert_eq!(result.report.counters.tessellations, 0);
    assert_eq!(result.report.counters.cache_hits, 2);
    assert_eq!(
        attachment()
            .resolve(&result.bodies[0].body, &WorkplanePolicy::default())
            .unwrap_err()
            .kind,
        WorkplaneError::AmbiguousSelection
    );
    let text = crate::text::dump_recipe(&duplicate);
    assert_eq!(
        crate::text::parse_recipe(&text)
            .unwrap()
            .recipe_fingerprint(),
        duplicate.recipe_fingerprint()
    );
    #[cfg(feature = "serde")]
    {
        let json = serde_json::to_string(&crate::interchange::to_dto(&duplicate)).unwrap();
        let restored = crate::interchange::from_dto(&serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(
            restored.recipe_fingerprint(),
            duplicate.recipe_fingerprint()
        );
    }
}
