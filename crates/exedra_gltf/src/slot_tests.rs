// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Regression coverage for authored slots across the complete export pipeline.

use super::*;
use exedra_assembly::{CompilePolicy, PartCompiler, flatten};
use exedra_constructive::ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};
use std::sync::Arc;

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
    let export = export_glb(&assembly, &compiled).unwrap();
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
        export_glb(assembly, compiled).unwrap(),
        export_glb_with_materials(assembly, compiled, &resolver, options).unwrap(),
    ] {
        let document = GlbDocument::parse(&export.bytes).unwrap();
        let json = document.json();
        let drawable_nodes: Vec<_> = json["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node.get("mesh").is_some())
            .collect();
        assert_eq!(drawable_nodes.len(), expected.len());
        for (node, expected) in drawable_nodes.into_iter().zip(expected) {
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
    let preview = export_gltf(assembly, compiled).unwrap();
    let resolved = export_gltf_with_materials(assembly, compiled, &resolver, options).unwrap();
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
        assert!(Arc::ptr_eq(
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

#[test]
fn recessed_boolean_exports_face_materials_and_reuses_geometry_when_rebound() {
    use exedra_constructive::ir::{CsgOp, Plane3};
    for mirrored in [false, true] {
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
        let recipe = builder.finish(root).unwrap();
        let recipe = if mirrored {
            recipe
                .mirrored(Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 0.0,
                })
                .unwrap()
        } else {
            recipe
        };
        let mut assembly = Assembly::new();
        let part = assembly.add_recipe_part("recess", recipe).unwrap();
        assembly.set_part_material(part, "front", "oak").unwrap();
        assembly.set_part_material(part, "cutter", "inset").unwrap();
        assembly.set_default_slot(part, "front").unwrap();
        assembly.bind_region_slot(part, 0, "front").unwrap();
        let instance = assembly
            .add_instance(None, "panel", part, Placement3::IDENTITY)
            .unwrap();
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&assembly, &CompilePolicy::default())
            .unwrap();
        assert_eq!(compiled.part(part).unwrap().bodies.len(), 1);
        let ranges = &compiled.part(part).unwrap().bodies[0].regions;
        assert!(
            ranges
                .windows(2)
                .all(|r| (r[0].region, r[0].material_slot) < (r[1].region, r[1].material_slot))
        );
        assert!(
            ranges
                .windows(2)
                .any(|r| r[0].region == r[1].region && r[0].material_slot != r[1].material_slot)
        );
        let export = export_glb(&assembly, &compiled).unwrap();
        assert_eq!(
            export.bytes,
            export_glb(&assembly, &compiled).unwrap().bytes
        );
        let document = GlbDocument::parse(&export.bytes).unwrap();
        assert_eq!(document.material_names(), ["oak", "inset"]);
        check_recess_material_geometry(&document, mirrored);

        assembly.bind_material(instance, "cutter", "brass").unwrap();
        let reused = compiler
            .compile_parts(&assembly, &CompilePolicy::default())
            .unwrap();
        assert!(Arc::ptr_eq(
            compiled.part(part).unwrap(),
            reused.part(part).unwrap()
        ));
        let export = export_glb(&assembly, &reused).unwrap();
        assert_eq!(
            GlbDocument::parse(&export.bytes).unwrap().material_names(),
            ["oak", "brass"]
        );
        assert_eq!(compiler.counters().parts_compiled, 1);
    }
}

fn check_recess_material_geometry(document: &GlbDocument, mirrored: bool) {
    use super::normal_tests::{attribute, read_floats, read_indices};
    let mut floor_center = false;
    let mut volume = 0.0;
    let sign = if mirrored { -1.0 } else { 1.0 };
    for primitive in document.json()["meshes"][0]["primitives"]
        .as_array()
        .unwrap()
    {
        let material = usize::try_from(primitive["material"].as_u64().unwrap()).unwrap();
        let name = document.json()["materials"][material]["name"]
            .as_str()
            .unwrap();
        for triangle in read_indices(document, primitive).chunks_exact(3) {
            let points: Vec<_> = triangle
                .iter()
                .map(|index| {
                    read_floats::<3>(document, attribute(document, primitive, "POSITION"), *index)
                        .map(f64::from)
                })
                .collect();
            // Signed volume uses the emitted winding, including reflections.
            let [a, b, c] = [points[0], points[1], points[2]];
            volume += (a[0] * (b[1] * c[2] - b[2] * c[1])
                + a[1] * (b[2] * c[0] - b[0] * c[2])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0;
            let local: Vec<_> = points.iter().map(|p| [p[0] * sign, p[1], p[2]]).collect();
            let cut = local
                .iter()
                .all(|p| (1.0..=3.0).contains(&p[0]) && (1.0..=3.0).contains(&p[1]) && p[2] >= 1.0);
            assert_eq!(name, if cut { "inset" } else { "oak" });
            let sides: Vec<_> = (0..3)
                .map(|i| {
                    let a = local[i];
                    let b = local[(i + 1) % 3];
                    (b[0] - a[0]) * (2.0 - a[1]) - (b[1] - a[1]) * (2.0 - a[0])
                })
                .collect();
            let covers_center = sides.iter().all(|v| *v >= 0.0) || sides.iter().all(|v| *v <= 0.0);
            if local.iter().all(|p| p[2] == 2.0) {
                assert!(!covers_center, "the recess opening must remain empty");
            }
            if local.iter().all(|p| p[2] == 1.0) && covers_center {
                floor_center = true;
                assert_eq!(
                    name, "inset",
                    "the exposed cutter cap owns the recess floor"
                );
                for index in triangle {
                    assert_eq!(
                        read_floats::<3>(
                            document,
                            attribute(document, primitive, "NORMAL"),
                            *index
                        ),
                        [0.0, 0.0, 1.0]
                    );
                }
            }
        }
    }
    assert!(floor_center);
    assert!((volume - 28.0).abs() < 1e-6, "recess volume={volume}");
}
