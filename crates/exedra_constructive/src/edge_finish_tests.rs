// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::builders;
use crate::cache::EvalCache;
use crate::evaluate::{Severity, evaluate, evaluate_with_cache};
use crate::ir::{
    CapMode, CsgOp, NodeId, NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder, SlotId,
};
use crate::tessellate::{EvalPolicy, tessellate_extrude, tessellate_primitive};
use crate::text;
use alloc::{format, rc::Rc, vec};
use exedra_mesh::ChangeSetBuilder;
use exedra_mesh::op::set_face_region;

fn box_node(b: &mut RecipeBuilder, size: [f64; 3]) -> NodeId {
    b.add(NodeKind::Primitive {
        spec: PrimitiveSpec::Box { size },
        placement: Placement3::IDENTITY,
    })
    .unwrap()
}

fn rail_recipe(selection: EdgeSelection, policy: RoundPolicy) -> Recipe {
    let mut b = RecipeBuilder::new();
    let child = box_node(&mut b, [0.09, 0.2, 2.0]);
    let root = b
        .add(NodeKind::EdgeFinish {
            child,
            selection,
            policy,
        })
        .unwrap();
    b.finish(root).unwrap()
}

#[test]
fn finished_geometry_and_uv_diagnostics_replay_from_cache_without_capturing_defaults() {
    let mut b = RecipeBuilder::new();
    let red = b.material_slot("red");
    let blue = b.material_slot("blue");
    let child = box_node(&mut b, [0.09, 0.2, 2.0]);
    let finish = b
        .add(NodeKind::EdgeFinish {
            child,
            selection: EdgeSelection::SharpEdges,
            policy: RoundPolicy::fillet(0.006),
        })
        .unwrap();
    let a = b
        .with_material(red)
        .add(NodeKind::Transform {
            child: finish,
            xf: Placement3::IDENTITY,
        })
        .unwrap();
    let c = b
        .with_material(blue)
        .add(NodeKind::Transform {
            child: finish,
            xf: Placement3::IDENTITY,
        })
        .unwrap();
    let root = b
        .add(NodeKind::Group {
            children: vec![a, c],
        })
        .unwrap();
    let recipe = b.finish(root).unwrap();
    let policy = EvalPolicy::default();
    let mut cache = EvalCache::new();
    let cold = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    assert_eq!(cold.bodies.len(), 2);
    assert_eq!(cold.report.counters.edge_finish_passes, 1);
    assert_eq!(warm.report.counters.edge_finish_passes, 0);
    assert!(Rc::ptr_eq(&cold.bodies[0].body, &cold.bodies[1].body));
    assert!(Rc::ptr_eq(&cold.bodies[0].body, &warm.bodies[0].body));
    assert_eq!(cold.report.diagnostics, warm.report.diagnostics);
    assert_eq!(
        cold.report
            .diagnostics
            .iter()
            .filter(|d| d.code == "eval.edge_finish.uv_deferred")
            .count(),
        2
    );
    for result in [&cold, &warm] {
        assert!(
            result
                .report
                .diagnostics
                .iter()
                .all(|d| d.severity != Severity::Error)
        );
        for (placed, slot) in result.bodies.iter().zip([red, blue]) {
            assert!(
                placed
                    .body
                    .mesh
                    .faces()
                    .all(|f| placed.material_for_face(f) == Some(slot))
            );
        }
    }
    let pure = evaluate(&recipe, &policy).unwrap();
    assert_eq!(
        format!("{:?}", cold.bodies[0].body.mesh),
        format!("{:?}", pure.bodies[0].body.mesh)
    );
}

#[test]
fn face_slots_regions_and_features_survive_selected_finishing() {
    let mut body = tessellate_primitive(
        PrimitiveSpec::Box {
            size: [0.09, 0.2, 0.1],
        },
        &Placement3::IDENTITY,
        &EvalPolicy::default(),
    )
    .unwrap();
    for face in body.mesh.faces() {
        let Feature::PrimitiveRegion { region } = body.source_map.face_feature(face).unwrap()
        else {
            panic!("primitive")
        };
        body.face_materials.insert(face, SlotId(region));
    }
    let selection = EdgeSelection::RegionBoundaries(vec![[1, 3]]);
    let mut policy = RoundPolicy::chamfer(0.005);
    policy.region = Some(77);
    let before = format!("{:?}", body.mesh);
    let (finished, stats) = finish_edges(&body, &selection, &policy).unwrap();
    assert_eq!(stats.chains, 1);
    assert_eq!(format!("{:?}", body.mesh), before);
    finished.source_map.check(&finished.mesh).unwrap();
    let mut band = 0;
    for face in finished.mesh.faces() {
        let Feature::PrimitiveRegion { region } = finished.source_map.face_feature(face).unwrap()
        else {
            panic!("lost source feature")
        };
        assert_eq!(finished.face_materials[&face], SlotId(region));
        let output_region = finished
            .mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .unwrap()
            .get(face.as_id())
            .copied()
            .unwrap();
        if output_region == 77 {
            band += 1;
            assert_eq!(region, 1);
        } else {
            assert_eq!(output_region, region);
        }
    }
    assert_eq!(band, 1);
}

#[test]
fn finishing_a_boolean_preserves_cut_slots_and_per_occurrence_defaults() {
    let mut b = RecipeBuilder::new();
    let cut = b.material_slot("cut");
    let red = b.material_slot("red");
    let blue = b.material_slot("blue");
    let block = box_node(&mut b, [1.0, 1.0, 1.0]);
    // Imported cutter regions occupy their own namespace: the primitive
    // side region 1 would otherwise collide with the box's +X face.
    let mut cutter_mesh = tessellate_primitive(
        PrimitiveSpec::Cylinder {
            radius: 0.15,
            height: 0.4,
            segments: 16,
        },
        &Placement3::translate(0.7, 0.7, 0.8),
        &EvalPolicy::default(),
    )
    .unwrap()
    .mesh;
    let regions: Vec<_> = cutter_mesh
        .faces()
        .map(|face| {
            (
                face,
                *cutter_mesh
                    .attrs()
                    .dense(attr::FACE_REGION)
                    .unwrap()
                    .get(face.as_id())
                    .unwrap()
                    + 100,
            )
        })
        .collect();
    let mut edit = cutter_mesh.edit_with(ChangeSetBuilder::new());
    for (face, region) in regions {
        set_face_region(&mut edit, face, region).unwrap();
    }
    let _ = edit.finish();
    let import = b.add_import(cutter_mesh).unwrap();
    let cutter = b
        .with_material(cut)
        .add(NodeKind::MeshImport {
            import,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let difference = b
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![block, cutter],
        })
        .unwrap();
    let finish = b
        .add(NodeKind::EdgeFinish {
            child: difference,
            selection: EdgeSelection::RegionBoundaries(vec![[5, 101]]),
            policy: RoundPolicy::chamfer(0.05),
        })
        .unwrap();
    let mut children = Vec::new();
    for slot in [red, blue] {
        children.push(
            b.with_material(slot)
                .add(NodeKind::Transform {
                    child: finish,
                    xf: Placement3::IDENTITY,
                })
                .unwrap(),
        );
    }
    let root = b.add(NodeKind::Group { children }).unwrap();
    let recipe = b.finish(root).unwrap();
    let mut cache = EvalCache::new();
    for pass in 0..2 {
        let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
        assert_eq!(result.bodies.len(), 2, "{:?}", result.report.diagnostics);
        assert_eq!(
            result.report.counters.edge_finish_passes,
            u32::from(pass == 0)
        );
        assert!(Rc::ptr_eq(&result.bodies[0].body, &result.bodies[1].body));
        for (placed, default) in result.bodies.iter().zip([red, blue]) {
            let mut cuts = 0;
            let mut surfaces = 0;
            for face in placed.body.mesh.faces() {
                match placed.body.source_map.face_feature(face).unwrap() {
                    Feature::BooleanFace { operand: 0 } => {
                        assert_eq!(placed.material_for_face(face), Some(default));
                        surfaces += 1;
                    }
                    Feature::BooleanFace { operand: 1 } => {
                        assert_eq!(placed.material_for_face(face), Some(cut));
                        cuts += 1;
                    }
                    other => panic!("lost Boolean ownership: {other:?}"),
                }
            }
            assert!(cuts > 0 && surfaces > 0);
        }
    }
}

#[test]
fn outer_reflection_and_scale_apply_after_local_finishing() {
    let mut b = RecipeBuilder::new();
    let child = box_node(&mut b, [0.09, 0.2, 2.0]);
    let finish = b
        .add(NodeKind::EdgeFinish {
            child,
            selection: EdgeSelection::SharpEdges,
            policy: RoundPolicy::fillet(0.006),
        })
        .unwrap();
    let xf = Placement3 {
        rows: [
            [-2.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 2.0],
            [0.0, 0.0, 1.0, 3.0],
        ],
    };
    let placed = b.add(NodeKind::Transform { child: finish, xf }).unwrap();
    let root = b
        .add(NodeKind::Group {
            children: vec![finish, placed],
        })
        .unwrap();
    let eval = evaluate(&b.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
    assert_eq!(eval.bodies.len(), 2, "{:?}", eval.report.diagnostics);
    let local = &eval.bodies[0].body.mesh;
    let world = &eval.bodies[1].body.mesh;
    assert!(world.validate_deep().is_empty());
    for (a, c) in local.vertices().zip(world.vertices()) {
        let p = local.vertex_position(a).unwrap().map(f64::from);
        let q = world.vertex_position(c).unwrap().map(f64::from);
        for (row, actual) in xf.rows.iter().zip(q) {
            let expected = row[0] * p[0] + row[1] * p[1] + row[2] * p[2] + row[3];
            assert!((actual - expected).abs() < 3e-7);
        }
    }
}

#[test]
fn target_order_is_canonical_and_every_policy_field_affects_the_fingerprint() {
    let policy = RoundPolicy::chamfer(0.005);
    let a = rail_recipe(
        EdgeSelection::RegionBoundaries(vec![[1, 3], [2, 4]]),
        policy,
    );
    let b = rail_recipe(
        EdgeSelection::RegionBoundaries(vec![[4, 2], [3, 1], [1, 3]]),
        policy,
    );
    assert_eq!(a.recipe_fingerprint(), b.recipe_fingerprint());
    let baseline = rail_recipe(EdgeSelection::SharpEdges, policy).recipe_fingerprint();
    let variants = [
        RoundPolicy {
            kind: RoundKind::Fillet { radius: 0.005 },
            ..policy
        },
        RoundPolicy {
            kind: RoundKind::Chamfer { setback: 0.006 },
            ..policy
        },
        RoundPolicy {
            segments: Some(4),
            ..policy
        },
        RoundPolicy {
            chord_tolerance: 0.0001,
            ..policy
        },
        RoundPolicy {
            sharpness_threshold: 0.75,
            ..policy
        },
        RoundPolicy {
            region: Some(7),
            ..policy
        },
        RoundPolicy {
            max_planar_deviation: 0.0001,
            ..policy
        },
        RoundPolicy {
            max_tangent_turn: 0.3,
            ..policy
        },
    ];
    for variant in variants {
        assert_ne!(
            rail_recipe(EdgeSelection::SharpEdges, variant).recipe_fingerprint(),
            baseline
        );
    }
    assert_ne!(a.recipe_fingerprint(), baseline);
    let dump = text::dump_recipe(&a);
    let restored = text::parse_recipe(&dump).unwrap();
    assert_eq!(restored.recipe_fingerprint(), a.recipe_fingerprint());
    #[cfg(feature = "serde")]
    {
        use crate::interchange::{from_dto, to_dto};
        let json = serde_json::to_string(&to_dto(&a)).unwrap();
        let restored = from_dto(&serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(restored.recipe_fingerprint(), a.recipe_fingerprint());
    }
}

#[test]
fn unsupported_geometry_is_explicit_and_does_not_emit_a_partial_finish() {
    let body = tessellate_extrude(
        &builders::l_profile(2.0, 2.0, 1.0, 1.0).unwrap(),
        &Placement3::IDENTITY,
        1.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    let before = format!("{:?}", body.mesh);
    assert!(matches!(
        finish_edges(&body, &EdgeSelection::SharpEdges, &RoundPolicy::fillet(0.1)),
        Err(EdgeFinishError::Round(RoundError::ConcaveEdge { .. }))
    ));
    assert_eq!(format!("{:?}", body.mesh), before);
    let missing = rail_recipe(
        EdgeSelection::RegionBoundaries(vec![[99, 100]]),
        RoundPolicy::fillet(0.006),
    );
    let result = evaluate(&missing, &EvalPolicy::default()).unwrap();
    assert!(result.bodies.is_empty());
    assert!(
        result
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "eval.edge_finish.empty_selection")
    );
    let too_wide = rail_recipe(EdgeSelection::SharpEdges, RoundPolicy::fillet(0.046));
    let result = evaluate(&too_wide, &EvalPolicy::default()).unwrap();
    assert!(result.bodies.is_empty());
    assert!(
        result
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "eval.edge_finish.clearance_exceeded")
    );
}

#[test]
fn recessed_panel_region_collisions_are_refused_as_ambiguous() {
    let mut b = RecipeBuilder::new();
    let panel = box_node(&mut b, [1.0, 1.0, 0.1]);
    let cutter = b
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box {
                size: [0.8, 0.8, 0.04],
            },
            placement: Placement3::translate(0.1, 0.1, 0.08),
        })
        .unwrap();
    let csg = b
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![panel, cutter],
        })
        .unwrap();
    let finish = b
        .add(NodeKind::EdgeFinish {
            child: csg,
            selection: EdgeSelection::RegionBoundaries(vec![[1, 5]]),
            policy: RoundPolicy::fillet(0.005),
        })
        .unwrap();
    let result = evaluate(&b.finish(finish).unwrap(), &EvalPolicy::default()).unwrap();
    assert!(result.bodies.is_empty());
    assert!(
        result
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "eval.edge_finish.ambiguous_selection"),
        "{:?}",
        result.report.diagnostics
    );
}

#[test]
fn semantic_selection_survives_compaction_and_different_tessellation() {
    for segments in [16, 32] {
        let body = tessellate_primitive(
            PrimitiveSpec::Cylinder {
                radius: 0.1,
                height: 0.2,
                segments,
            },
            &Placement3::IDENTITY,
            &EvalPolicy::default(),
        )
        .unwrap();
        let selection = EdgeSelection::RegionBoundaries(vec![[1, 2]]);
        let policy = RoundPolicy::chamfer(0.003);
        let (finished, stats) = finish_edges(&body, &selection, &policy).unwrap();
        assert_eq!(stats.closed_chains, 1);
        assert_eq!(stats.strip_faces, segments);
        // Rounding deleted source faces and vertices. Compact that result,
        // then resolve a different rim by semantic region, never by edge ID.
        let (mesh, remap) = finished.mesh.compact();
        let faces: BTreeMap<_, _> = finished
            .mesh
            .faces()
            .map(|f| {
                (
                    remap.face(f).unwrap(),
                    finished.source_map.face_feature(f).unwrap(),
                )
            })
            .collect();
        let vertices: BTreeMap<_, _> = finished
            .mesh
            .vertices()
            .map(|v| {
                (
                    remap.vertex(v).unwrap(),
                    finished.source_map.vertex_feature(v).unwrap(),
                )
            })
            .collect();
        let source_map = SourceMap::new(
            &mesh,
            mesh.faces().map(|f| faces[&f]).collect(),
            mesh.vertices().map(|v| vertices[&v]).collect(),
        );
        let compacted = TessellatedBody {
            mesh,
            source_map,
            face_materials: BTreeMap::new(),
            refinement: None,
        };
        let bottom = EdgeSelection::RegionBoundaries(vec![[1, 3]]);
        let (a, stats_a) = finish_edges(&finished, &bottom, &policy).unwrap();
        let (b, stats_b) = finish_edges(&compacted, &bottom, &policy).unwrap();
        assert_eq!(stats_a, stats_b);
        let positions = |body: &TessellatedBody| {
            let mut p: Vec<_> = body
                .mesh
                .vertices()
                .map(|v| body.mesh.vertex_position(v).unwrap().map(f32::to_bits))
                .collect();
            p.sort_unstable();
            p
        };
        assert_eq!(positions(&a), positions(&b));
        assert!(a.mesh.validate_deep().is_empty());
        assert!(b.mesh.validate_deep().is_empty());
    }
}

#[test]
fn incomplete_children_remain_refused_when_nested_or_cached() {
    let mut b = RecipeBuilder::new();
    let child = box_node(&mut b, [0.09, 0.2, 2.0]);
    let bad = b
        .add(NodeKind::EdgeFinish {
            child,
            selection: EdgeSelection::SharpEdges,
            policy: RoundPolicy::fillet(0.046),
        })
        .unwrap();
    let group = b
        .add(NodeKind::Group {
            children: vec![child, bad],
        })
        .unwrap();
    let root = b
        .add(NodeKind::EdgeFinish {
            child: group,
            selection: EdgeSelection::SharpEdges,
            policy: RoundPolicy::fillet(0.006),
        })
        .unwrap();
    let recipe = b.finish(root).unwrap();
    let mut cache = EvalCache::new();
    for _ in 0..2 {
        let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
        assert!(result.bodies.is_empty());
        assert_eq!(result.report.counters.edge_finish_refusals, 2);
        assert!(
            result
                .report
                .diagnostics
                .iter()
                .any(|d| d.code == "eval.edge_finish.incomplete_child")
        );
    }
}
