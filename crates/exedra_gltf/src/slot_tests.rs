// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Regression coverage for authored slots across the complete export pipeline.

use super::*;
use exedra_assembly::{CompilePolicy, PartCompiler, flatten};
use exedra_constructive::ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};

#[test]
fn authored_slots_survive_overlapping_body_regions() {
    let mut builder = RecipeBuilder::new();
    let shell = builder.material_slot("shell");
    let back = builder.material_slot("back");
    let children = [shell, back].map(|slot| {
        builder
            .with_material(slot)
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box { size: [1.0; 3] },
                placement: Placement3::IDENTITY,
            })
            .unwrap()
    });
    let root = builder
        .add(NodeKind::Group {
            children: children.to_vec(),
        })
        .unwrap();
    let mut assembly = Assembly::new();
    let part = assembly
        .add_recipe_part("part", builder.finish(root).unwrap())
        .unwrap();
    assembly.set_part_material(part, "shell", "red").unwrap();
    assembly.set_part_material(part, "back", "blue").unwrap();
    assembly
        .add_instance(None, "a", part, Placement3::IDENTITY)
        .unwrap();
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    let list = flatten(&assembly, &compiled);
    assert_eq!(list.items.len(), 2);
    assert_eq!(
        list.items[0].regions[0].region,
        list.items[1].regions[0].region
    );
    for (item, expected) in list.items.iter().zip(["red", "blue"]) {
        assert!(
            item.regions
                .iter()
                .all(|region| region.material.as_deref() == Some(expected))
        );
    }
    let export = export_glb(&assembly, &compiled, &list).unwrap();
    let document = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(document.material_names(), ["red", "blue"]);
    assert_bound_exports(&assembly, &compiled, &[Some("red"), Some("blue")]);
    // An explicit fallback cannot replace either body's authored assignment.
    assembly.set_default_slot(part, "shell").unwrap();
    assembly.bind_region_slot(part, 0, "shell").unwrap();
    assert_bound_exports(&assembly, &compiled, &[Some("red"), Some("blue")]);
}

fn assert_bound_exports(assembly: &Assembly, compiled: &CompiledParts, expected: &[Option<&str>]) {
    let list = flatten(assembly, compiled);
    assert_eq!(list.items.len(), expected.len());
    for (item, expected) in list.items.iter().zip(expected) {
        assert!(
            item.regions
                .iter()
                .all(|r| r.material.as_deref() == *expected)
        );
    }
    let resolver = |_: &str| Some(json!({"pbrMetallicRoughness": {"metallicFactor": 0.0}}));
    let options = GltfExportOptions::default();
    for export in [
        export_glb(assembly, compiled, &list).unwrap(),
        export_glb_with_materials(assembly, compiled, &list, &resolver, options).unwrap(),
    ] {
        let document = GlbDocument::parse(&export.bytes).unwrap();
        let json = document.json();
        for (node, expected) in json["nodes"].as_array().unwrap().iter().zip(expected) {
            let mesh = usize::try_from(node["mesh"].as_u64().unwrap()).unwrap();
            for primitive in json["meshes"][mesh]["primitives"].as_array().unwrap() {
                match expected {
                    Some(expected) => {
                        let material = usize::try_from(
                            primitive["material"].as_u64().expect("assigned primitive"),
                        )
                        .unwrap();
                        assert_eq!(json["materials"][material]["name"], *expected);
                    }
                    None => assert!(primitive.get("material").is_none()),
                }
            }
        }
    }
    let preview = export_gltf(assembly, compiled, &list).unwrap();
    let resolved =
        export_gltf_with_materials(assembly, compiled, &list, &resolver, options).unwrap();
    assert_eq!(preview.stats.materials, resolved.stats.materials);
}

#[test]
fn inherited_slots_child_overrides_and_unassigned_siblings_reach_both_export_paths() {
    for single_slot in [true, false] {
        let mut builder = RecipeBuilder::new();
        let shell = builder.material_slot("shell");
        let back = if single_slot {
            shell
        } else {
            builder.material_slot("back")
        };
        let mut add_box = || {
            builder
                .add(NodeKind::Primitive {
                    spec: PrimitiveSpec::Box { size: [1.0; 3] },
                    placement: Placement3::IDENTITY,
                })
                .unwrap()
        };
        let first = add_box();
        let second = add_box();
        let unassigned = add_box();
        let group = builder
            .with_material(shell)
            .add(NodeKind::Group {
                children: vec![first],
            })
            .unwrap();
        let transform = builder
            .with_material(back)
            .add(NodeKind::Transform {
                child: second,
                xf: Placement3::translate(2.0, 0.0, 0.0),
            })
            .unwrap();
        // The group's shell assignment must not replace the nearer back assignment.
        let assigned = builder
            .with_material(shell)
            .add(NodeKind::Group {
                children: vec![group, transform],
            })
            .unwrap();
        let root = builder
            .add(NodeKind::Group {
                children: vec![assigned, unassigned],
            })
            .unwrap();
        let mut assembly = Assembly::new();
        let part = assembly
            .add_recipe_part("part", builder.finish(root).unwrap())
            .unwrap();
        assembly.set_part_material(part, "shell", "red").unwrap();
        if !single_slot {
            assembly.set_part_material(part, "back", "blue").unwrap();
        }
        let first_instance = assembly
            .add_instance(None, "a", part, Placement3::IDENTITY)
            .unwrap();
        let second_instance = assembly
            .add_instance(None, "b", part, Placement3::translate(4.0, 0.0, 0.0))
            .unwrap();
        assembly
            .bind_material(second_instance, "shell", "green")
            .unwrap();
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&assembly, &CompilePolicy::default())
            .unwrap();
        let back_first = if single_slot { "red" } else { "blue" };
        let back_second = if single_slot { "green" } else { "blue" };
        assert_bound_exports(
            &assembly,
            &compiled,
            &[
                Some("red"),
                Some(back_first),
                None,
                Some("green"),
                Some(back_second),
                None,
            ],
        );
        let counters = compiler.counters();
        assembly
            .bind_material(first_instance, "shell", "purple")
            .unwrap();
        let reused = compiler
            .compile_parts(&assembly, &CompilePolicy::default())
            .unwrap();
        assert!(std::rc::Rc::ptr_eq(
            compiled.part(part).unwrap(),
            reused.part(part).unwrap()
        ));
        assert_eq!(compiler.counters().parts_compiled, counters.parts_compiled);
        assert_eq!(
            compiler.counters().triangles_emitted,
            counters.triangles_emitted
        );
        assert_bound_exports(
            &assembly,
            &reused,
            &[
                Some("purple"),
                Some(if single_slot { "purple" } else { "blue" }),
                None,
                Some("green"),
                Some(back_second),
                None,
            ],
        );
    }
}
