// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{CompilePolicy, PartCompiler, assembly_fingerprint, flatten};
use exedra_constructive::ir::{NodeKind, PrimitiveSpec, RecipeBuilder};

fn cube(assembly: &mut Assembly) -> PartId {
    let mut builder = RecipeBuilder::new();
    let root = builder
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: [1.0; 3] },
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    assembly
        .add_recipe_part("cube", builder.finish(root).unwrap())
        .unwrap()
}

#[test]
fn frames_carry_nested_poses_and_metadata_without_registering_geometry() {
    let mut assembly = Assembly::new();
    let root = assembly
        .add_frame(None, "root", Placement3::translate(10.0, 0.0, 0.0))
        .unwrap();
    let pivot = assembly
        .add_frame(
            Some(root),
            "pivot",
            Placement3 {
                rows: [
                    [0.0, -1.0, 0.0, 1.0],
                    [1.0, 0.0, 0.0, 2.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
            },
        )
        .unwrap();
    assembly
        .set_metadata(pivot, "occurrence", "caller:pivot")
        .unwrap();
    assert_eq!(assembly.content_generation(), 0);
    assert_eq!(assembly.parts().len(), 0);
    assert_eq!(assembly.instance(pivot).unwrap().part(), None);
    assert_eq!(assembly.resolved_material(pivot, SlotIndex(0)), None);
    assert_eq!(
        assembly.bind_material(pivot, "surface", "paint"),
        Err(AssemblyError::NoPart(pivot))
    );

    let part = cube(&mut assembly);
    let arm = assembly
        .add_instance(
            Some(pivot),
            "arm",
            part,
            Placement3::translate(3.0, 0.0, 0.0),
        )
        .unwrap();
    let marker = assembly
        .add_instance(
            Some(arm),
            "marker",
            part,
            Placement3::translate(0.0, 2.0, 0.0),
        )
        .unwrap();
    assembly
        .add_frame(None, "empty", Placement3::IDENTITY)
        .unwrap();
    let mut compiler = PartCompiler::new();
    let compiled = compiler
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    assert_eq!(compiler.counters().parts_compiled, 1);
    let draw = flatten(&assembly, &compiled);
    assert_eq!(draw.items.len(), 2);
    assert_eq!(draw.items[0].instance, arm);
    assert_eq!(draw.items[1].instance, marker);
    assert_eq!(
        draw.items[0].world.rows,
        [
            [0.0, -1.0, 0.0, 11.0],
            [1.0, 0.0, 0.0, 5.0],
            [0.0, 0.0, 1.0, 0.0]
        ]
    );
    assert_eq!(draw.items[1].world.rows[0][3], 9.0);
    assert_eq!(
        assembly.path_of(marker).unwrap().to_string(),
        "root/pivot/arm/marker"
    );
}

#[test]
fn append_keeps_empty_frames_and_attaches_source_roots_to_the_destination_parent() {
    let mut source = Assembly::new();
    let root = source
        .add_frame(None, "module", Placement3::translate(1.0, 0.0, 0.0))
        .unwrap();
    let empty = source
        .add_frame(Some(root), "empty", Placement3::IDENTITY)
        .unwrap();
    let part = cube(&mut source);
    let child = source
        .add_instance(
            Some(root),
            "cube",
            part,
            Placement3::translate(0.0, 2.0, 0.0),
        )
        .unwrap();
    let mut destination = Assembly::new();
    let parent = destination
        .add_frame(None, "site", Placement3::translate(10.0, 0.0, 0.0))
        .unwrap();
    let map = destination
        .append(
            Some(parent),
            &source,
            "west",
            Placement3::translate(0.0, 0.0, 3.0),
        )
        .unwrap();
    assert_eq!(destination.roots(), &[parent]);
    assert_eq!(
        destination.instance(parent).unwrap().children(),
        &[map.instance(root).unwrap()]
    );
    assert_eq!(
        destination
            .instance(map.instance(root).unwrap())
            .unwrap()
            .part(),
        None
    );
    assert_eq!(
        destination
            .instance(map.instance(empty).unwrap())
            .unwrap()
            .part(),
        None
    );
    assert_eq!(
        destination
            .instance(map.instance(child).unwrap())
            .unwrap()
            .part(),
        map.part(part)
    );
    let compiled = PartCompiler::new()
        .compile_parts(&destination, &CompilePolicy::default())
        .unwrap();
    let draw = flatten(&destination, &compiled);
    assert_eq!(draw.items[0].world, Placement3::translate(11.0, 2.0, 3.0));
    let before = assembly_fingerprint(&destination);
    assert!(matches!(
        destination.append(
            Some(InstanceId(u32::MAX)),
            &source,
            "east",
            Placement3::IDENTITY
        ),
        Err(AssemblyError::UnknownInstance(_))
    ));
    assert_eq!(assembly_fingerprint(&destination), before);

    let mut frames = Assembly::new();
    let map = frames
        .append_selected(
            None,
            &source,
            "frames",
            Placement3::IDENTITY,
            |_, instance| instance.part().is_none(),
        )
        .unwrap();
    assert_eq!(frames.parts().len(), 0);
    assert_eq!(frames.instances().len(), 2);
    assert_eq!(map.part(part), None);
    assert_eq!(map.instance(child), None);
    assert_eq!(frames.content_generation(), 0);
}

#[cfg(feature = "serde")]
#[test]
fn frames_round_trip_without_accepting_material_bindings() {
    use crate::interchange::{AssemblyInterchangeError, SlotMaterialDto, from_dto, to_dto};
    let mut assembly = Assembly::new();
    let frame = assembly
        .add_frame(None, "frame", Placement3::IDENTITY)
        .unwrap();
    assembly
        .set_metadata(frame, "occurrence", "external:key")
        .unwrap();
    let part = cube(&mut assembly);
    assembly
        .add_instance(Some(frame), "solid", part, Placement3::IDENTITY)
        .unwrap();
    let mut dto = to_dto(&assembly);
    let json = serde_json::to_string(&dto).unwrap();
    let rebuilt = from_dto(&serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(
        assembly_fingerprint(&assembly),
        assembly_fingerprint(&rebuilt)
    );
    dto.instances[0].bindings.push(SlotMaterialDto {
        slot: "surface".into(),
        material: "paint".into(),
    });
    assert!(matches!(
        from_dto(&dto),
        Err(AssemblyInterchangeError::Assembly(AssemblyError::NoPart(_)))
    ));
}
