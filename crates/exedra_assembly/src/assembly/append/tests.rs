// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{CompilePolicy, InstancePath, PartCompiler, assembly_fingerprint, flatten};
use alloc::rc::Rc;
use exedra_constructive::ir::{NodeKind, PrimitiveSpec, RecipeBuilder};
use exedra_mesh::Mesh;

fn recipe() -> Recipe {
    sized_recipe(1.0)
}

fn sized_recipe(size: f64) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let surface = builder.material_slot("surface");
    builder.material_slot("edge");
    let root = builder
        .with_material(surface)
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: [size; 3] },
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    builder.finish(root).unwrap()
}

fn populated() -> Assembly {
    let mut assembly = Assembly::new();
    let part = assembly.add_recipe_part("existing", recipe()).unwrap();
    assembly
        .add_instance(None, "existing", part, Placement3::IDENTITY)
        .unwrap();
    assembly
}

#[test]
fn copies_geometry_bindings_metadata_and_hierarchy_without_losing_sharing() {
    let mut source = Assembly::new();
    let unused = source.add_recipe_part("unused", recipe()).unwrap();
    let frame = source.add_recipe_part("frame", recipe()).unwrap();
    let baked = source
        .add_baked_part("insert", Mesh::new(), &["edge", "surface"])
        .unwrap();
    source.set_default_slot(frame, "surface").unwrap();
    source.bind_region_slot(frame, 7, "edge").unwrap();
    source
        .set_part_material(frame, "surface", "caller:wood")
        .unwrap();
    source
        .set_part_material(baked, "edge", "caller:metal")
        .unwrap();
    let root = source
        .add_instance(None, "unit", frame, Placement3::translate(2.0, 0.0, 0.0))
        .unwrap();
    let child = source
        .add_instance(
            Some(root),
            "insert",
            baked,
            Placement3::translate(0.0, 3.0, 0.0),
        )
        .unwrap();
    let other = source
        .add_instance(None, "second", frame, Placement3::IDENTITY)
        .unwrap();
    let second_child = source
        .add_instance(Some(other), "insert", baked, Placement3::IDENTITY)
        .unwrap();
    source.bind_material(child, "edge", "caller:brass").unwrap();
    source.set_metadata(child, "role", "fastener").unwrap();
    let source_before = assembly_fingerprint(&source);
    let placement = Placement3 {
        rows: [
            [-1.0, 0.5, 0.0, 10.0],
            [0.0, 2.0, 0.0, 20.0],
            [0.0, 0.0, 1.0, 30.0],
        ],
    };
    let mut destination = populated();
    let map = destination.append(&source, "west", placement).unwrap();

    assert_eq!(destination.parts().len(), 3);
    assert_eq!(destination.instances().len(), 5);
    assert_eq!(map.part(unused), None);
    assert_eq!(map.part(PartId(u32::MAX)), None);
    assert_eq!(map.instance(InstanceId(u32::MAX)), None);
    assert_eq!(assembly_fingerprint(&source), source_before);
    for id in [frame, baked] {
        let original = source.part(id).unwrap();
        let copied = destination.part(map.part(id).unwrap()).unwrap();
        assert_eq!(copied.slots(), original.slots());
        assert_eq!(copied.region_slots(), original.region_slots());
        assert_eq!(copied.default_slot(), original.default_slot());
        assert_eq!(copied.default_materials(), original.default_materials());
    }
    assert!(matches!(
        destination.part(map.part(frame).unwrap()).unwrap().source(),
        PartSource::Recipe(_)
    ));
    assert!(matches!(
        destination.part(map.part(baked).unwrap()).unwrap().source(),
        PartSource::Baked(_)
    ));
    let copied = destination.instance(map.instance(child).unwrap()).unwrap();
    assert_eq!(copied.part(), map.part(baked).unwrap());
    assert_eq!(copied.parent(), map.instance(root));
    assert_eq!(copied.key(), "insert");
    assert_eq!(
        copied.placement(),
        source.instance(child).unwrap().placement()
    );
    assert_eq!(
        copied.bindings(),
        source.instance(child).unwrap().bindings()
    );
    assert_eq!(
        copied.metadata(),
        source.instance(child).unwrap().metadata()
    );
    assert_eq!(
        destination
            .instance(map.instance(second_child).unwrap())
            .unwrap()
            .part(),
        copied.part()
    );
    assert_eq!(
        destination
            .instance(map.instance(root).unwrap())
            .unwrap()
            .children(),
        &[map.instance(child).unwrap()]
    );
    assert_eq!(
        destination.resolve_path(&InstancePath::from_segments(&["west-unit", "insert"])),
        map.instance(child)
    );
    assert_eq!(
        destination.resolve_path(&InstancePath::from_segments(&["west-second", "insert"])),
        map.instance(second_child)
    );
    assert_eq!(
        destination.resolved_material(map.instance(root).unwrap(), SlotIndex(0)),
        Some("caller:wood")
    );
    assert_eq!(
        destination.resolved_material(map.instance(child).unwrap(), SlotIndex(0)),
        Some("caller:brass")
    );

    // Verify the composed placement through the actual rendering seam.
    let mut compiler = PartCompiler::new();
    let compiled = compiler
        .compile_parts(&destination, &CompilePolicy::default())
        .unwrap();
    let rendered = flatten(&destination, &compiled);
    let item = rendered
        .items
        .iter()
        .find(|item| Some(item.instance) == map.instance(root))
        .unwrap();
    assert_eq!(
        item.world.rows,
        [
            [-1.0, 0.5, 0.0, 8.0],
            [0.0, 2.0, 0.0, 20.0],
            [0.0, 0.0, 1.0, 30.0]
        ]
    );
    let count = compiler.counters().parts_compiled;
    let generation = destination.content_generation();
    destination
        .bind_material(map.instance(root).unwrap(), "surface", "caller:paint")
        .unwrap();
    compiler
        .compile_parts(&destination, &CompilePolicy::default())
        .unwrap();
    assert_eq!(compiler.counters().parts_compiled, count);
    assert_eq!(destination.content_generation(), generation);
    assert_eq!(
        source.resolved_material(root, SlotIndex(0)),
        Some("caller:wood")
    );
}

#[test]
fn selection_prunes_subtrees_and_preserves_source_order() {
    let mut source = populated();
    let root = source.roots()[0];
    let part = source.instances()[0].part();
    let omitted = source
        .add_instance(Some(root), "omit", part, Placement3::IDENTITY)
        .unwrap();
    let descendant = source
        .add_instance(Some(omitted), "never-visit", part, Placement3::IDENTITY)
        .unwrap();
    let retained = source
        .add_instance(
            Some(root),
            "keep",
            part,
            Placement3::translate(1.0, 2.0, 3.0),
        )
        .unwrap();
    let mut visited = Vec::new();
    let mut destination = Assembly::new();
    let map = destination
        .append_selected(
            &source,
            "copy",
            Placement3::translate(10.0, 0.0, 0.0),
            |id, _| {
                visited.push(id);
                id != omitted
            },
        )
        .unwrap();
    assert_eq!(visited, [root, omitted, retained]);
    assert_eq!(map.instance(omitted), None);
    assert_eq!(map.instance(descendant), None);
    assert_eq!(destination.parts().len(), 1);
    assert_eq!(destination.instances().len(), 2);
    let compiled = PartCompiler::new()
        .compile_parts(&destination, &CompilePolicy::default())
        .unwrap();
    let rendered = flatten(&destination, &compiled);
    assert_eq!(
        rendered.items[1].world,
        Placement3::translate(11.0, 2.0, 3.0)
    );
}

#[test]
fn late_errors_leave_every_destination_record_unchanged() {
    let mut source = populated();
    source
        .add_instance(None, "last", PartId(0), Placement3::IDENTITY)
        .unwrap();
    let mut destination = populated();
    destination
        .add_instance(None, "copy-last", PartId(0), Placement3::IDENTITY)
        .unwrap();
    let before = assembly_fingerprint(&destination);
    let generation = destination.content_generation();
    assert_eq!(
        destination.append(&source, "copy", Placement3::IDENTITY),
        Err(AssemblyError::DuplicateChildKey {
            parent: None,
            key: "copy-last".into()
        })
    );
    assert_eq!(assembly_fingerprint(&destination), before);
    assert_eq!(destination.content_generation(), generation);
    assert_eq!(destination.part_by_key("copy-existing"), None);
    assert_eq!(destination.parts().len(), 1);
    assert_eq!(destination.instances().len(), 2);

    let mut destination = populated();
    let before = assembly_fingerprint(&destination);
    // The first root is staged successfully; only the second composition overflows.
    source.instances[1].placement = Placement3::translate(f64::MAX, 0.0, 0.0);
    assert_eq!(
        destination.append(&source, "copy", Placement3::translate(f64::MAX, 0.0, 0.0)),
        Err(AssemblyError::NonFinitePlacement)
    );
    assert_eq!(assembly_fingerprint(&destination), before);
    assert_eq!(destination.content_generation(), 1);
    assert_eq!(destination.part_by_key("copy-existing"), None);
}

#[test]
fn validates_prefix_placement_and_part_collisions() {
    let source = populated();
    let mut destination = populated();
    for prefix in ["", "a/b"] {
        assert_eq!(
            destination.append(&source, prefix, Placement3::IDENTITY),
            Err(AssemblyError::InvalidKey(prefix.into()))
        );
    }
    for value in [f64::NAN, f64::INFINITY] {
        assert_eq!(
            destination.append(&source, "copy", Placement3::translate(value, 0.0, 0.0)),
            Err(AssemblyError::NonFinitePlacement)
        );
    }
    destination
        .add_recipe_part("copy-existing", recipe())
        .unwrap();
    let before = assembly_fingerprint(&destination);
    assert_eq!(
        destination.append(&source, "copy", Placement3::IDENTITY),
        Err(AssemblyError::DuplicatePartKey("copy-existing".into()))
    );
    assert_eq!(assembly_fingerprint(&destination), before);
}

#[test]
fn empty_selection_is_a_noop_and_capacity_is_checked_without_overflow() {
    let mut destination = populated();
    let before = assembly_fingerprint(&destination);
    let generation = destination.content_generation();
    destination
        .append(&Assembly::new(), "empty", Placement3::IDENTITY)
        .unwrap();
    let map = destination
        .append_selected(&populated(), "empty", Placement3::IDENTITY, |_, _| false)
        .unwrap();
    assert_eq!(map.part(PartId(0)), None);
    assert_eq!(map.instance(InstanceId(0)), None);
    assert_eq!(assembly_fingerprint(&destination), before);
    assert_eq!(destination.content_generation(), generation);
    assert_eq!(check_capacity(0, u32::MAX as usize), Ok(()));
    assert_eq!(
        check_capacity(u32::MAX as usize, 1),
        Err(AssemblyError::CapacityExceeded)
    );
    assert_eq!(
        check_capacity(usize::MAX, 1),
        Err(AssemblyError::CapacityExceeded)
    );
}

#[test]
fn rebuilt_snapshots_reuse_compiled_geometry_despite_new_poses_and_local_handles() {
    let mut source = populated();
    let mut compiler = PartCompiler::new();
    let policy = CompilePolicy::default();
    let mut first = populated();
    let first_map = first.append(&source, "copy", Placement3::IDENTITY).unwrap();
    let first_compiled = compiler.compile_parts(&first, &policy).unwrap();
    let old_part = first_map.part(PartId(0)).unwrap();
    let old_geometry = first_compiled.part(old_part).unwrap();
    assert_eq!(compiler.counters().parts_compiled, 1);
    let triangles = compiler.counters().triangles_emitted;

    // Rebuild structure with different part IDs, a new pose and new bindings.
    let mut second = Assembly::new();
    let second_map = second
        .append(&source, "copy", Placement3::translate(5.0, 2.0, 1.0))
        .unwrap();
    second
        .bind_material(
            second_map.instance(InstanceId(0)).unwrap(),
            "surface",
            "caller:paint",
        )
        .unwrap();
    let new_part = second_map.part(PartId(0)).unwrap();
    assert_ne!(old_part, new_part);
    let second_compiled = compiler.compile_parts(&second, &policy).unwrap();
    assert!(
        Rc::ptr_eq(old_geometry, second_compiled.part(new_part).unwrap()),
        "poses and local handle assignment do not identify geometry"
    );
    assert!(
        core::ptr::eq(
            first_compiled.report(old_part).unwrap(),
            second_compiled.report(new_part).unwrap()
        ),
        "cache hits retain surface evidence"
    );
    assert_eq!(compiler.counters().parts_compiled, 1);
    assert_eq!(compiler.counters().triangles_emitted, triangles);

    source
        .replace_part_source(PartId(0), PartSource::Recipe(sized_recipe(2.0)))
        .unwrap();
    let mut resized = second.clone();
    let resized_map = resized
        .append(&source, "resized", Placement3::IDENTITY)
        .unwrap();
    let resized_compiled = compiler.compile_parts(&resized, &policy).unwrap();
    assert_eq!(compiler.counters().parts_compiled, 2);
    assert!(
        Rc::ptr_eq(old_geometry, resized_compiled.part(new_part).unwrap()),
        "only the changed content compiles"
    );
    assert!(
        !Rc::ptr_eq(
            old_geometry,
            resized_compiled
                .part(resized_map.part(PartId(0)).unwrap())
                .unwrap()
        ),
        "changed dimensions must not reuse old geometry"
    );
    let mut changed_policy = policy;
    changed_policy.evaluation.discretize.chord_tolerance *= 2.0;
    compiler.compile_parts(&second, &changed_policy).unwrap();
    assert_eq!(compiler.counters().parts_compiled, 3);

    drop(compiler);
    assert_eq!(
        flatten(&first, &first_compiled).items[1].world,
        Placement3::IDENTITY
    );
    assert_eq!(
        flatten(&second, &second_compiled).items[0].world,
        Placement3::translate(5.0, 2.0, 1.0)
    );
    assert_eq!(
        old_geometry.triangle_count(),
        12,
        "old snapshots own their compiled buffers after the cache is released"
    );
}
