// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    builders,
    cache::{EvalCache, policy_fingerprint},
    evaluate::{evaluate, evaluate_with_cache},
    ir::{CapMode, NodeKind, Placement3, Plane3, Recipe, RecipeBuilder},
    tessellate::{EvalPolicy, TessellateError},
    text,
    workplane::{SurfaceSelector, WorkplaneAttachment, WorkplaneError},
};

fn intent() -> WorkplaneAttachment {
    WorkplaneAttachment {
        surface: SurfaceSelector::EndCap,
        anchor: [0.5, 0.5, 0.0],
        projection: [0.0, 0.0, 1.0],
        x_direction: [1.0, 0.0, 0.0],
    }
}
fn attached(
    height: f64,
    slope: f64,
    attachment: WorkplaneAttachment,
    parent: Placement3,
) -> Recipe {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(builders::rect(4.0, 3.0).unwrap());
    let support = b
        .add(NodeKind::ExtrudeToPlane {
            profile,
            placement: Placement3::IDENTITY,
            plane: Plane3 {
                normal: [-slope, 0.0, 1.0],
                distance: height,
            },
        })
        .unwrap();
    let peg = b.add_profile(builders::rect(0.2, 0.2).unwrap());
    let source = b.source_ref("mount");
    let child = b
        .with_source(source)
        .add(NodeKind::Extrude {
            profile: peg,
            placement: Placement3::IDENTITY,
            height: 0.3,
            caps: CapMode::Both,
        })
        .unwrap();
    let attached = b
        .add(NodeKind::OnWorkplane {
            support,
            child,
            attachment,
        })
        .unwrap();
    let root = b
        .add(NodeKind::Transform {
            child: attached,
            xf: parent,
        })
        .unwrap();
    b.finish(root).unwrap()
}

#[test]
fn retained_attachment_follows_new_planes_preserves_roll_and_round_trips() {
    let policy = EvalPolicy::default();
    let mut cache = EvalCache::new();
    let mut previous = None;
    for (height, slope) in [(1.0, 0.0), (1.5, 0.2), (1.2, -0.15)] {
        let recipe = attached(height, slope, intent(), Placement3::IDENTITY);
        let cold = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
        assert_eq!(
            cold.bodies.len(),
            1,
            "support must not be emitted by attachment"
        );
        let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
        assert_eq!(warm.report.counters.tessellations, 0);
        assert_eq!(warm.report.counters.cache_hits, 2);
        assert_eq!(warm.report.attachments, cold.report.attachments);
        let resolution = &warm.report.attachments[0];
        let frame = resolution.frame;
        assert!((frame.rows[2][3] - (height + 0.5 * slope)).abs() < 1e-6);
        assert!((frame.rows[0][3] - 0.5).abs() < 1e-12);
        assert!((frame.rows[1][3] - 0.5).abs() < 1e-12);
        let x = frame.rows.map(|row| row[0]);
        let normal = frame.rows.map(|row| row[2]);
        assert!(exedra_math::dot(x, [1.0, 0.0, slope]) > 0.99);
        assert!(exedra_math::dot(normal, [-slope, 0.0, 1.0]) > 0.99);
        assert!(resolution.selected_faces > 0);
        assert!(
            warm.bodies[0]
                .body
                .mesh
                .boundary_loops()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            text::parse_recipe(&text::dump_recipe(&recipe))
                .unwrap()
                .recipe_fingerprint(),
            recipe.recipe_fingerprint()
        );
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string(&crate::interchange::to_dto(&recipe)).unwrap();
            let parsed =
                crate::interchange::from_dto(&serde_json::from_str(&json).unwrap()).unwrap();
            assert_eq!(parsed.recipe_fingerprint(), recipe.recipe_fingerprint());
        }
        if let Some(old) = previous {
            assert_ne!(old, recipe.recipe_fingerprint());
        }
        previous = Some(recipe.recipe_fingerprint());
    }
}

#[test]
fn projection_and_missing_surfaces_are_explicit_failures_without_old_frame_fallback() {
    let mut cache = EvalCache::new();
    let policy = EvalPolicy::default();
    evaluate_with_cache(
        &attached(1.0, 0.0, intent(), Placement3::IDENTITY),
        &policy,
        &mut cache,
    )
    .unwrap();
    for (attachment, expected) in [
        (
            WorkplaneAttachment {
                surface: SurfaceSelector::Region(999),
                ..intent()
            },
            WorkplaneError::EmptySelection,
        ),
        (
            WorkplaneAttachment {
                projection: [1.0, 0.0, 0.0],
                ..intent()
            },
            WorkplaneError::ParallelProjection,
        ),
        (
            WorkplaneAttachment {
                x_direction: [0.0, 0.0, 1.0],
                ..intent()
            },
            WorkplaneError::ParallelAxis,
        ),
    ] {
        let failure = evaluate_with_cache(
            &attached(1.0, 0.0, attachment, Placement3::IDENTITY),
            &policy,
            &mut cache,
        )
        .unwrap_err();
        assert_eq!(failure.error, TessellateError::Attachment(expected));
    }
    let changed = EvalPolicy {
        workplane: crate::workplane::WorkplanePolicy {
            max_faces: 1,
            ..policy.workplane
        },
        ..policy
    };
    assert_ne!(policy_fingerprint(&policy), policy_fingerprint(&changed));
    assert_eq!(
        evaluate(
            &attached(1.0, 0.2, intent(), Placement3::IDENTITY),
            &changed
        )
        .unwrap_err()
        .error,
        TessellateError::Attachment(WorkplaneError::BudgetExceeded)
    );
}

#[test]
fn ancestor_affine_placement_applies_after_local_resolution() {
    let parent = Placement3 {
        rows: [
            [-2.0, 0.0, 0.0, 10.0],
            [0.0, 3.0, 0.0, 0.0],
            [0.0, 0.0, 4.0, 100.0],
        ],
    };
    let evaluation = evaluate(
        &attached(1.0, 0.0, intent(), parent),
        &EvalPolicy::default(),
    )
    .unwrap();
    assert_eq!(evaluation.report.attachments[0].frame.rows[2][3], 1.0);
    let mesh = &evaluation.bodies[0].body.mesh;
    let min_z = mesh
        .vertices()
        .map(|v| mesh.vertex_position(v).unwrap()[2])
        .fold(f32::INFINITY, f32::min);
    assert_eq!(min_z, 104.0);
    assert!(mesh.validate_deep().is_empty());
    let mut triangles = alloc::vec::Vec::new();
    let mut volume = 0.0;
    for face in mesh.faces() {
        assert!(!mesh.face_triangles_into(
            face,
            exedra_mesh::FaceTriangulation::Robust,
            &mut triangles
        ));
        for triangle in &triangles {
            let p = triangle.map(|h| {
                mesh.vertex_position(mesh.to_vertex(h).unwrap())
                    .unwrap()
                    .map(f64::from)
            });
            volume += exedra_math::dot(p[0], exedra_math::cross(p[1], p[2])) / 6.0;
        }
    }
    assert!(volume > 0.0);
}

#[test]
fn multiple_support_bodies_are_ambiguous() {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(builders::rect(1.0, 1.0).unwrap());
    let a = b
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 1.0,
            caps: CapMode::Both,
        })
        .unwrap();
    let second = b
        .add(NodeKind::Instance {
            of: a,
            placement: Placement3::translate(2.0, 0.0, 0.0),
        })
        .unwrap();
    let support = b
        .add(NodeKind::Group {
            children: alloc::vec![a, second],
        })
        .unwrap();
    let root = b
        .add(NodeKind::OnWorkplane {
            support,
            child: a,
            attachment: intent(),
        })
        .unwrap();
    assert_eq!(
        evaluate(&b.finish(root).unwrap(), &EvalPolicy::default())
            .unwrap_err()
            .error,
        TessellateError::AttachmentSupportCount { actual: 2 }
    );
}

#[test]
fn disconnected_terminal_features_are_rejected_without_picking_the_first_patch() {
    let mut builder = exedra_mesh::MeshBuilder::new();
    for point in [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [2.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [2.0, 1.0, 0.0],
    ] {
        builder.push_vertex(point);
    }
    builder.add_face(&[0, 1, 2]).unwrap();
    builder.add_face(&[3, 4, 5]).unwrap();
    let mut body =
        crate::tessellate::TessellatedBody::from_imported_mesh(builder.build().unwrap().mesh);
    body.source_map = crate::source_map::SourceMap::new(
        &body.mesh,
        alloc::vec![crate::tessellate::Feature::CapEnd;2],
        alloc::vec![crate::tessellate::Feature::Imported;6],
    );
    assert_eq!(
        intent()
            .resolve(&body, &crate::workplane::WorkplanePolicy::default())
            .unwrap_err(),
        WorkplaneError::AmbiguousSelection
    );
}

#[test]
fn removed_terminal_cap_is_missing_even_when_a_new_cut_reuses_its_region() {
    use crate::{ir::PlaneSide, section::CutCap};
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(builders::rect(2.0, 2.0).unwrap());
    let child = b
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 1.0,
            caps: CapMode::Both,
        })
        .unwrap();
    let cut = b
        .add(NodeKind::PlaneCut {
            child,
            plane: Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance: 0.6,
            },
            side: PlaneSide::Negative,
            cap: CutCap {
                region: crate::tessellate::REGION_CAP_END,
                material: None,
            },
        })
        .unwrap();
    let body = evaluate(&b.finish(cut).unwrap(), &EvalPolicy::default()).unwrap();
    assert_eq!(
        intent()
            .resolve(
                &body.bodies[0].body,
                &crate::workplane::WorkplanePolicy::default()
            )
            .unwrap_err(),
        WorkplaneError::EmptySelection
    );
    let explicit = WorkplaneAttachment {
        surface: SurfaceSelector::Region(crate::tessellate::REGION_CAP_END),
        ..intent()
    };
    assert!(
        explicit
            .resolve(
                &body.bodies[0].body,
                &crate::workplane::WorkplanePolicy::default()
            )
            .is_ok()
    );
}

#[test]
fn boolean_surfaces_retain_cap_meaning_and_immediate_operand_regions() {
    use crate::{edge_finish::OperandRegion, ir::CsgOp};
    let mut b = RecipeBuilder::new();
    let panel = b.add_profile(builders::rect(2.0, 2.0).unwrap());
    let panel = b
        .add(NodeKind::Extrude {
            profile: panel,
            placement: Placement3::IDENTITY,
            height: 1.0,
            caps: CapMode::Both,
        })
        .unwrap();
    let hole = b.add_profile(builders::rect(0.5, 0.5).unwrap());
    let hole = b
        .add(NodeKind::Extrude {
            profile: hole,
            placement: Placement3::translate(0.75, 0.75, -0.5),
            height: 2.0,
            caps: CapMode::Both,
        })
        .unwrap();
    let root = b
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: alloc::vec![panel, hole],
        })
        .unwrap();
    let result = evaluate(&b.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
    assert_eq!(result.bodies.len(), 1);
    let body = &result.bodies[0].body;
    let semantic = intent()
        .resolve(body, &crate::workplane::WorkplanePolicy::default())
        .unwrap();
    let explicit = WorkplaneAttachment {
        surface: SurfaceSelector::OperandRegion(OperandRegion {
            operand: 0,
            region: crate::tessellate::REGION_CAP_END,
        }),
        ..intent()
    };
    let frame = explicit
        .resolve(body, &crate::workplane::WorkplanePolicy::default())
        .unwrap();
    assert_eq!(semantic.faces(), frame.faces());
    let patch = crate::clearance::PlanarPatch::from_workplane(
        body,
        &frame,
        &crate::clearance::BoundaryPolicy::default(),
    )
    .unwrap();
    assert_eq!(patch.loops().len(), 2);
}

#[test]
fn incomplete_support_cannot_attach_to_a_surviving_body_even_when_cached() {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(builders::rect(2.0, 3.0).unwrap());
    let solid = b
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 1.0,
            caps: CapMode::Both,
        })
        .unwrap();
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
    let support = b
        .add(NodeKind::Group {
            children: alloc::vec![solid, refused],
        })
        .unwrap();
    let root = b
        .add(NodeKind::OnWorkplane {
            support,
            child: solid,
            attachment: intent(),
        })
        .unwrap();
    let recipe = b.finish(root).unwrap();
    let mut cache = EvalCache::new();
    for _ in 0..2 {
        let failure = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap_err();
        assert_eq!(failure.node, root);
        assert_eq!(failure.error, TessellateError::IncompleteAttachmentSupport);
    }
}
