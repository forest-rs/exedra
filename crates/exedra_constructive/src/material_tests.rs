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
fn boolean_uniform_slots_survive_and_mixed_slots_are_explicit_refusals() {
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
                if mixed {
                    assert!(result.bodies.is_empty());
                    assert_eq!(
                        result.report.fidelity_of(root),
                        Some(Fidelity::EnvelopeOnly)
                    );
                    assert!(
                        result
                            .report
                            .diagnostics
                            .iter()
                            .any(|d| d.code == "eval.csg.material_slots_unsupported")
                    );
                } else {
                    assert_eq!(result.bodies.len(), 1);
                    assert_eq!(result.bodies[0].material, Some(a));
                    assert_eq!(result.report.fidelity_of(root), Some(Fidelity::Exact));
                }
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
fn mixed_assigned_and_unassigned_boolean_operand_groups_are_refused() {
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
    assert!(result.bodies.is_empty());
    assert!(
        result
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "eval.csg.material_slots_unsupported")
    );
}

#[test]
fn repeated_partial_instances_do_not_hide_material_refusals_from_booleans() {
    let mut builder = RecipeBuilder::new();
    let a = builder.material_slot("a");
    let b = builder.material_slot("b");
    let first = cube(&mut builder, Some(a), 0.0);
    let second = cube(&mut builder, Some(b), 0.5);
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
                .filter(|d| d.code == "eval.csg.material_slots_unsupported")
                .count(),
            2
        );
    }
}
