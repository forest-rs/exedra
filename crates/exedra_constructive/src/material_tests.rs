// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;

use crate::cache::EvalCache;
use crate::evaluate::{Fidelity, evaluate, evaluate_with_cache};
use crate::ir::{
    CsgOp, NodeId, NodeKind, Placement3, Plane3, PrimitiveSpec, RecipeBuilder, SlotId,
};
use crate::tessellate::EvalPolicy;

fn cube(builder: &mut RecipeBuilder, material: Option<SlotId>, x: f64) -> NodeId {
    if let Some(slot) = material {
        builder.with_material(slot);
    }
    builder
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: [1.0; 3] },
            placement: Placement3::translate(x, 0.0, 0.0),
        })
        .unwrap()
}

#[test]
fn recessed_panel_accepts_distinct_shell_and_cutter_slots() {
    let mut builder = RecipeBuilder::new();
    let front = builder.material_slot("front");
    let cutter = builder.material_slot("cutter");
    let shell = builder
        .with_material(front)
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box {
                size: [4.0, 4.0, 2.0],
            },
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let recess = builder
        .with_material(cutter)
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: [2.0; 3] },
            placement: Placement3::translate(1.0, 1.0, 1.0),
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![shell, recess],
        })
        .unwrap();
    let result = evaluate(&builder.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
    assert_eq!(result.bodies.len(), 1, "{:?}", result.report.diagnostics);
    assert_eq!(result.report.fidelity_of(root), Some(Fidelity::Exact));
}

#[test]
fn inherited_slots_are_occurrence_local_and_cache_independent() {
    let mut builder = RecipeBuilder::new();
    let a = builder.material_slot("a");
    let b = builder.material_slot("b");
    let shared = cube(&mut builder, None, 0.0);
    let explicit = cube(&mut builder, Some(b), 2.0);
    let group = builder
        .with_material(a)
        .add(NodeKind::Group {
            children: vec![shared, explicit],
        })
        .unwrap();
    let transform = builder
        .with_material(b)
        .add(NodeKind::Transform {
            child: shared,
            xf: Placement3::IDENTITY,
        })
        .unwrap();
    let mirror = builder
        .with_material(a)
        .add(NodeKind::Mirror {
            child: shared,
            plane: Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 0.0,
            },
        })
        .unwrap();
    let first_instance = builder
        .with_material(a)
        .add(NodeKind::Instance {
            of: shared,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let second_instance = builder
        .with_material(b)
        .add(NodeKind::Instance {
            of: shared,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let explicit_instance = builder
        .with_material(a)
        .add(NodeKind::Instance {
            of: explicit,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Group {
            children: vec![
                group,
                transform,
                mirror,
                first_instance,
                second_instance,
                explicit_instance,
                shared,
            ],
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let expected = [
        Some(a),
        Some(b),
        Some(b),
        Some(a),
        Some(a),
        Some(b),
        Some(b),
        None,
    ];
    let policy = EvalPolicy::default();
    let pure = evaluate(&recipe, &policy).unwrap();
    let mut cache = EvalCache::new();
    let cold = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    for result in [&pure, &cold, &warm] {
        assert_eq!(
            result
                .bodies
                .iter()
                .map(|body| body.material)
                .collect::<Vec<_>>(),
            expected
        );
    }
    assert_eq!(warm.report.counters.tessellations, 0);
    assert!(
        Rc::ptr_eq(&cold.bodies[0].body, &cold.bodies[2].body),
        "inherited slot changes do not change cached geometry"
    );
}

#[test]
fn exact_and_mesh_stretches_preserve_effective_child_slots() {
    let mut builder = RecipeBuilder::new();
    let a = builder.material_slot("a");
    let b = builder.material_slot("b");
    let inherited = cube(&mut builder, None, 0.0);
    let explicit = cube(&mut builder, Some(b), 0.0);
    let transform = builder
        .with_material(b)
        .add(NodeKind::Transform {
            child: inherited,
            xf: Placement3::IDENTITY,
        })
        .unwrap();
    let group = builder
        .add(NodeKind::Group {
            children: vec![inherited, explicit],
        })
        .unwrap();
    let plane = Plane3 {
        normal: [1.0, 0.0, 0.0],
        distance: 0.5,
    };
    let mut stretches = Vec::new();
    for child in [inherited, explicit, transform, group] {
        stretches.push(
            builder
                .with_material(a)
                .add(NodeKind::Stretch {
                    child,
                    plane,
                    length: 0.2,
                })
                .unwrap(),
        );
    }
    let root = builder
        .add(NodeKind::Group {
            children: stretches,
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let mut cache = EvalCache::new();
    for _ in 0..2 {
        let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
        assert_eq!(
            result
                .bodies
                .iter()
                .map(|body| body.material)
                .collect::<Vec<_>>(),
            [Some(a), Some(b), Some(b), Some(a), Some(b)]
        );
        assert_eq!(result.report.counters.stretch_exact, 3);
        assert_eq!(result.report.counters.stretch_mesh, 1);
    }
}

#[test]
fn boolean_faces_preserve_uniform_and_mixed_slots() {
    for op in [CsgOp::Union, CsgOp::Difference, CsgOp::Intersection] {
        for mixed in [false, true] {
            let mut builder = RecipeBuilder::new();
            let a = builder.material_slot("a");
            let b = builder.material_slot("b");
            let first = cube(&mut builder, Some(a), 0.0);
            let second = cube(&mut builder, mixed.then_some(b), 0.5);
            let root = builder
                .with_material(a)
                .add(NodeKind::Csg {
                    op,
                    operands: vec![first, second],
                })
                .unwrap();
            let recipe = builder.finish(root).unwrap();
            let mut cache = EvalCache::new();
            for _ in 0..2 {
                let result =
                    evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
                assert_eq!(result.bodies.len(), 1, "{:?}", result.report.diagnostics);
                assert_eq!(result.report.fidelity_of(root), Some(Fidelity::Exact));
                let placed = &result.bodies[0];
                let mut owners = alloc::collections::BTreeSet::new();
                for face in placed.body.mesh.faces() {
                    let Some(crate::tessellate::Feature::BooleanFace { operand }) =
                        placed.body.source_map.face_feature(face)
                    else {
                        panic!("face owner")
                    };
                    owners.insert(operand);
                    let expected = if mixed && operand == 1 { b } else { a };
                    assert_eq!(placed.material_for_face(face), Some(expected));
                }
                assert_eq!(owners.len(), 2);
            }
        }
    }
}

#[test]
fn imported_bodies_keep_their_slot_through_mirror_and_mesh_stretch() {
    let mut source = RecipeBuilder::new();
    let root = cube(&mut source, None, 0.0);
    let evaluated = evaluate(&source.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
    let mut builder = RecipeBuilder::new();
    let a = builder.material_slot("a");
    let b = builder.material_slot("b");
    let import = builder
        .add_import(evaluated.bodies[0].body.mesh.clone())
        .unwrap();
    let imported = builder
        .with_material(b)
        .add(NodeKind::MeshImport {
            import,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let mirrored = builder
        .with_material(a)
        .add(NodeKind::Mirror {
            child: imported,
            plane: Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 0.0,
            },
        })
        .unwrap();
    let stretched = builder
        .with_material(a)
        .add(NodeKind::Stretch {
            child: imported,
            plane: Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 0.5,
            },
            length: 0.2,
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Group {
            children: vec![imported, mirrored, stretched],
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let mut cache = EvalCache::new();
    for _ in 0..2 {
        let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
        assert_eq!(
            result
                .bodies
                .iter()
                .map(|body| body.material)
                .collect::<Vec<_>>(),
            [Some(b); 3]
        );
        assert_eq!(result.report.counters.stretch_mesh, 1);
    }
}

#[test]
fn boolean_operand_groups_preserve_assigned_and_unassigned_faces() {
    let mut builder = RecipeBuilder::new();
    let a = builder.material_slot("a");
    let assigned = cube(&mut builder, Some(a), 0.0);
    let unassigned = cube(&mut builder, None, 2.0);
    let group = builder
        .add(NodeKind::Group {
            children: vec![assigned, unassigned],
        })
        .unwrap();
    let cutter = cube(&mut builder, Some(a), 0.5);
    let root = builder
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![group, cutter],
        })
        .unwrap();
    let result = evaluate(&builder.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
    assert_eq!(result.bodies.len(), 1, "{:?}", result.report.diagnostics);
    assert_eq!(result.report.fidelity_of(root), Some(Fidelity::Exact));
    let placed = &result.bodies[0];
    let mut assigned = 0;
    let mut unassigned = 0;
    for face in placed.body.mesh.faces() {
        let separate = placed.body.mesh.face_loop(face).all(|corner| {
            let vertex = placed.body.mesh.to_vertex(corner).unwrap();
            placed.body.mesh.vertex_position(vertex).unwrap()[0] >= 2.0
        });
        assert_eq!(placed.material_for_face(face), (!separate).then_some(a));
        if separate {
            unassigned += 1;
        } else {
            assigned += 1;
        }
    }
    assert!(assigned > 0 && unassigned > 0);
}

#[test]
fn repeated_partial_instances_do_not_hide_topology_refusals_from_booleans() {
    let mut builder = RecipeBuilder::new();
    let a = builder.material_slot("a");
    let b = builder.material_slot("b");
    let first = cube(&mut builder, Some(a), 0.0);
    let second = builder
        .with_material(b)
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: [1.0; 3] },
            placement: Placement3::translate(1.0, 1.0, 0.0),
        })
        .unwrap();
    let refused = builder
        .add(NodeKind::Csg {
            op: CsgOp::Union,
            operands: vec![first, second],
        })
        .unwrap();
    let partial = builder
        .add(NodeKind::Group {
            children: vec![first, refused],
        })
        .unwrap();
    let instance = builder
        .add(NodeKind::Instance {
            of: partial,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let consuming = builder
        .add(NodeKind::Csg {
            op: CsgOp::Union,
            operands: vec![instance, first],
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Group {
            children: vec![consuming, consuming],
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let mut cache = EvalCache::new();
    for _ in 0..2 {
        let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
        assert!(result.bodies.is_empty());
        assert_eq!(
            result
                .report
                .diagnostics
                .iter()
                .filter(|d| d.node == Some(refused) && d.code == "eval.csg.unsupported")
                .count(),
            2
        );
    }
}

fn assert_operand_slots(placed: &crate::evaluate::PlacedBody, slots: &[Option<SlotId>]) {
    let mut seen = alloc::collections::BTreeSet::new();
    assert!(placed.body.mesh.validate_deep().is_empty());
    for face in placed.body.mesh.faces() {
        let Some(crate::tessellate::Feature::BooleanFace { operand }) =
            placed.body.source_map.face_feature(face)
        else {
            panic!("Boolean surface has an operand owner");
        };
        seen.insert(operand);
        assert_eq!(placed.material_for_face(face), slots[usize::from(operand)]);
    }
    assert_eq!(
        seen.len(),
        slots.len(),
        "every expected operand has a surface"
    );
}

#[test]
fn boolean_cached_face_slots_do_not_capture_inherited_defaults() {
    let mut builder = RecipeBuilder::new();
    let cutter_slot = builder.material_slot("cutter");
    let red = builder.material_slot("red");
    let blue = builder.material_slot("blue");
    let shell = cube(&mut builder, None, 0.0);
    let cutter = cube(&mut builder, Some(cutter_slot), 0.5);
    let shared = builder
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![shell, cutter],
        })
        .unwrap();
    let children = [red, blue].map(|slot| {
        builder
            .with_material(slot)
            .add(NodeKind::Transform {
                child: shared,
                xf: Placement3::IDENTITY,
            })
            .unwrap()
    });
    let root = builder
        .add(NodeKind::Group {
            children: children.to_vec(),
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let mut cache = EvalCache::new();
    let pure = evaluate(&recipe, &EvalPolicy::default()).unwrap();
    let cold = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
    let warm = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
    for result in [&pure, &cold, &warm] {
        assert_eq!(result.bodies.len(), 2);
        for (placed, default) in result.bodies.iter().zip([red, blue]) {
            assert_operand_slots(placed, &[Some(default), Some(cutter_slot)]);
        }
    }
    assert!(Rc::ptr_eq(&cold.bodies[0].body, &cold.bodies[1].body));
    assert_eq!(warm.report.counters.tessellations, 0);
}

#[test]
fn nested_boolean_preserves_distinct_slots_within_one_operand() {
    let mut builder = RecipeBuilder::new();
    let a = builder.material_slot("a");
    let b = builder.material_slot("b");
    let c = builder.material_slot("c");
    let first = cube(&mut builder, Some(a), 0.0);
    let second = cube(&mut builder, Some(b), 0.5);
    let union = builder
        .add(NodeKind::Csg {
            op: CsgOp::Union,
            operands: vec![first, second],
        })
        .unwrap();
    let cutter = builder
        .with_material(c)
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box {
                size: [0.5, 0.5, 1.0],
            },
            placement: Placement3::translate(0.75, 0.25, 0.5),
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![union, cutter],
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let mut cache = EvalCache::new();
    for _ in 0..2 {
        let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
        assert_eq!(result.bodies.len(), 1, "{:?}", result.report.diagnostics);
        let placed = &result.bodies[0];
        let mut seen = alloc::collections::BTreeSet::new();
        for face in placed.body.mesh.faces() {
            let Some(crate::tessellate::Feature::BooleanFace { operand }) =
                placed.body.source_map.face_feature(face)
            else {
                panic!("face owner")
            };
            let points: Vec<_> = placed
                .body
                .mesh
                .face_loop(face)
                .map(|corner| {
                    *placed
                        .body
                        .mesh
                        .vertex_position(placed.body.mesh.to_vertex(corner).unwrap())
                        .unwrap()
                })
                .collect();
            let expected = if operand == 1 {
                c
            } else if points.iter().all(|p| p[0] <= 1.0) {
                a
            } else {
                b
            };
            assert_eq!(placed.material_for_face(face), Some(expected));
            seen.insert(expected.0);
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), [a.0, b.0, c.0]);
    }
}

#[test]
fn coincident_faces_and_cutters_use_operand_order_without_merging_slots() {
    for op in [CsgOp::Union, CsgOp::Intersection, CsgOp::Difference] {
        for reverse in [false, true] {
            let mut builder = RecipeBuilder::new();
            let a = builder.material_slot("a");
            let b = builder.material_slot("b");
            assert_eq!(builder.material_slot("a"), a);
            let first = cube(&mut builder, Some(a), 0.5);
            let second = cube(&mut builder, Some(b), 0.5);
            let mut operands = if reverse {
                vec![second, first]
            } else {
                vec![first, second]
            };
            let default = builder.material_slot("shell");
            if op == CsgOp::Difference {
                let shell = cube(&mut builder, Some(default), 0.0);
                operands.insert(0, shell);
            }
            let root = builder.add(NodeKind::Csg { op, operands }).unwrap();
            let recipe = builder.finish(root).unwrap();
            assert_eq!(recipe.slots(), &["a", "b", "shell"]);
            let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
            assert_eq!(result.bodies.len(), 1, "{:?}", result.report.diagnostics);
            let winner = if reverse { b } else { a };
            let placed = &result.bodies[0];
            if op == CsgOp::Difference {
                assert_operand_slots(placed, &[Some(default), Some(winner)]);
            } else {
                assert_operand_slots(placed, &[Some(winner)]);
            }
        }
    }
}

#[test]
fn mixed_boolean_slots_survive_mirrors_instances_and_mesh_stretch() {
    let mut builder = RecipeBuilder::new();
    let a = builder.material_slot("shell");
    let b = builder.material_slot("cut");
    let shell = cube(&mut builder, Some(a), 0.0);
    let cutter = cube(&mut builder, Some(b), 0.5);
    let difference = builder
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![shell, cutter],
        })
        .unwrap();
    let mirror = builder
        .add(NodeKind::Mirror {
            child: difference,
            plane: Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 0.0,
            },
        })
        .unwrap();
    let instance = builder
        .add(NodeKind::Instance {
            of: difference,
            placement: Placement3 {
                rows: [
                    [-2.0, 0.0, 0.0, 0.0],
                    [0.0, 3.0, 0.0, 0.0],
                    [0.0, 0.0, 4.0, 0.0],
                ],
            },
        })
        .unwrap();
    let mut children = vec![mirror, instance];
    let mut expected_bounds = vec![
        ([-0.5, 0.0, 0.0], [0.0, 1.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 3.0, 4.0]),
    ];
    for length in [0.125, -0.125] {
        for distance in [-1.0, 0.25, 2.0] {
            children.push(
                builder
                    .add(NodeKind::Stretch {
                        child: difference,
                        plane: Plane3 {
                            normal: [1.0, 0.0, 0.0],
                            distance,
                        },
                        length,
                    })
                    .unwrap(),
            );
            let min = if distance < 0.0 { length } else { 0.0 };
            let max = if distance < 0.5 { 0.5 + length } else { 0.5 };
            expected_bounds.push(([min, 0.0, 0.0], [max, 1.0, 1.0]));
        }
    }
    let root = builder.add(NodeKind::Group { children }).unwrap();
    let recipe = builder.finish(root).unwrap();
    let mut cache = EvalCache::new();
    for _ in 0..2 {
        let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
        assert_eq!(
            result.bodies.len(),
            expected_bounds.len(),
            "{:?}",
            result.report.diagnostics
        );
        for (placed, (min, max)) in result.bodies.iter().zip(&expected_bounds) {
            assert_operand_slots(placed, &[Some(a), Some(b)]);
            for axis in 0..3 {
                let coordinates: Vec<_> = placed
                    .body
                    .mesh
                    .vertices()
                    .map(|vertex| {
                        f64::from(placed.body.mesh.vertex_position(vertex).unwrap()[axis])
                    })
                    .collect();
                assert_eq!(
                    coordinates.iter().copied().fold(f64::INFINITY, f64::min),
                    min[axis]
                );
                assert_eq!(
                    coordinates
                        .iter()
                        .copied()
                        .fold(f64::NEG_INFINITY, f64::max),
                    max[axis]
                );
            }
        }
    }
}
