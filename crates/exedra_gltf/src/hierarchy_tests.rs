// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use exedra_assembly::{CompilePolicy, PartCompiler, compose, flatten};
use exedra_constructive::ir::{CsgOp, NodeKind, PrimitiveSpec, Recipe, RecipeBuilder};
use exedra_mesh::Mesh;
use std::{collections::BTreeSet, f64::consts::FRAC_PI_2, sync::Arc};

fn boxes(size: f64, count: usize) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let slot = builder.material_slot("surface");
    let children = (0..count)
        .map(|i| {
            builder
                .with_material(slot)
                .add(NodeKind::Primitive {
                    spec: PrimitiveSpec::Box {
                        size: [size, 0.1, size],
                    },
                    placement: Placement3::translate(0.0, i as f64 * 0.2, 0.0),
                })
                .unwrap()
        })
        .collect();
    let root = builder.add(NodeKind::Group { children }).unwrap();
    builder.finish(root).unwrap()
}

fn articulated_assembly(angle: f64, size: f64, material: &str) -> Assembly {
    let mut assembly = Assembly::new();
    let arm_recipe = boxes(size, 2);
    let arm_part = assembly.add_recipe_part("arm", arm_recipe.clone()).unwrap();
    let shared_part = assembly.add_recipe_part("spare-arm", arm_recipe).unwrap();
    let marker_part = assembly.add_recipe_part("marker", boxes(0.1, 1)).unwrap();
    let empty_part = assembly
        .add_baked_part("empty-result", Mesh::new(), &[])
        .unwrap();
    assembly
        .set_part_material(arm_part, "surface", material)
        .unwrap();
    assembly
        .set_part_material(shared_part, "surface", "finish-b")
        .unwrap();
    let root = assembly
        .add_frame(None, "root", Placement3::translate(10.0, 20.0, 30.0))
        .unwrap();
    assembly
        .set_metadata(root, "caller.instance", "external:root")
        .unwrap();
    let arm = assembly
        .add_instance(
            Some(root),
            "arm",
            arm_part,
            Placement3::rotate_z_then_translate(angle, 1.0, 2.0, 0.0),
        )
        .unwrap();
    assembly
        .set_metadata(arm, "caller.instance", "external:arm")
        .unwrap();
    assembly
        .add_instance(
            Some(arm),
            "marker",
            marker_part,
            Placement3::translate(0.8, -0.05, 0.8),
        )
        .unwrap();
    assembly
        .add_frame(
            Some(arm),
            "attachment",
            Placement3::translate(0.0, 0.0, 1.0),
        )
        .unwrap();
    assembly
        .add_instance(Some(arm), "empty", empty_part, Placement3::IDENTITY)
        .unwrap();
    assembly
        .add_instance(
            None,
            "spare",
            shared_part,
            Placement3::translate(-5.0, 0.0, 0.0),
        )
        .unwrap();
    assembly
}

fn index(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn logical_node<'a>(json: &'a Value, path: &str) -> (usize, &'a Value) {
    json["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .find(|(_, node)| node["extras"]["instancePath"].as_str() == Some(path))
        .unwrap()
}

/// Independently follow glTF parent/local matrices and decode each drawable's
/// POSITION data once, even when several material primitives share it.
fn world_geometry(document: &GlbDocument) -> (Vec<[f64; 3]>, Vec<(usize, Placement3)>) {
    let json = document.json();
    let mut stack: Vec<_> = json["scenes"][index(&json["scene"])]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| (index(id), Placement3::IDENTITY))
        .collect();
    let mut points = Vec::new();
    let mut placements = Vec::new();
    while let Some((id, parent)) = stack.pop() {
        let node = &json["nodes"][id];
        let local = node
            .get("matrix")
            .map_or(Placement3::IDENTITY, |matrix| Placement3 {
                rows: std::array::from_fn(|r| {
                    std::array::from_fn(|c| matrix[c * 4 + r].as_f64().unwrap())
                }),
            });
        let world = compose(&parent, &local);
        placements.push((id, world));
        if let Some(children) = node.get("children") {
            stack.extend(
                children
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|child| (index(child), world)),
            );
        }
        let Some(mesh) = node.get("mesh") else {
            continue;
        };
        let positions: BTreeSet<_> = json["meshes"][index(mesh)]["primitives"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| index(&p["attributes"]["POSITION"]))
            .collect();
        for accessor in positions {
            let accessor = &json["accessors"][accessor];
            assert_eq!(accessor["componentType"], 5126);
            let view = &json["bufferViews"][index(&accessor["bufferView"])];
            let base = index(&view["byteOffset"]) + accessor.get("byteOffset").map_or(0, index);
            let stride = view.get("byteStride").map_or(12, index);
            for vertex in 0..index(&accessor["count"]) {
                let p: [f64; 3] = std::array::from_fn(|axis| {
                    let offset = base + vertex * stride + axis * 4;
                    f64::from(f32::from_le_bytes(
                        document.bin()[offset..offset + 4].try_into().unwrap(),
                    ))
                });
                points.push(
                    world
                        .rows
                        .map(|r| r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + r[3]),
                );
            }
        }
    }
    (points, placements)
}

fn sort_points(points: &mut [[f64; 3]]) {
    points.sort_by(|a, b| {
        a[0].total_cmp(&b[0])
            .then(a[1].total_cmp(&b[1]))
            .then(a[2].total_cmp(&b[2]))
    });
}

#[test]
fn retained_hierarchy_matches_evaluated_vertices_through_intermediate_poses() {
    let mut compiler = PartCompiler::new();
    let policy = CompilePolicy::default();
    let mut previous = None;
    let mut original_buffer = None;
    for angle in [0.0, 0.35, FRAC_PI_2] {
        let assembly = articulated_assembly(angle, 1.0, "finish-a");
        let compiled = compiler.compile_parts(&assembly, &policy).unwrap();
        assert_eq!(compiler.counters().parts_compiled, 3);
        assert_eq!(compiler.counters().triangles_emitted, 36);
        let arm_part = compiled.part(PartId(0)).unwrap();
        assert!(
            Arc::ptr_eq(arm_part, compiled.part(PartId(1)).unwrap()),
            "equal registrations share compiled ownership"
        );
        if let Some(previous) = &previous {
            assert!(
                Arc::ptr_eq(previous, arm_part),
                "pose changes reuse immutable geometry"
            );
        }
        previous = Some(Arc::clone(arm_part));
        let draw = flatten(&assembly, &compiled);
        for coordinates in [GltfCoordinates::Preserve, GltfCoordinates::ZUpToYUp] {
            let options = GltfExportOptions { coordinates };
            let export = export_glb_with_options(&assembly, &compiled, options).unwrap();
            assert_eq!(
                export.bytes,
                export_glb_with_options(&assembly, &compiled, options)
                    .unwrap()
                    .bytes
            );
            let document = GlbDocument::parse(&export.bytes).unwrap();
            let json = document.json();
            let paths: Vec<_> = json["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|node| node["extras"]["instancePath"].as_str())
                .collect();
            assert_eq!(paths.len(), assembly.instances().len());
            assert_eq!(
                paths.iter().copied().collect::<BTreeSet<_>>().len(),
                paths.len(),
                "each logical occurrence has exactly one identity"
            );
            let (root_id, root) = logical_node(json, "root");
            let (arm_id, arm) = logical_node(json, "root/arm");
            let (marker_id, _) = logical_node(json, "root/arm/marker");
            assert!(
                root["children"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(arm_id))
            );
            assert!(
                arm["children"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(marker_id))
            );
            assert!(root.get("mesh").is_none());
            assert!(
                arm.get("mesh").is_none(),
                "multi-body geometry attaches below the occurrence"
            );
            assert_eq!(arm["extras"]["caller.instance"], "external:arm");
            assert!(
                logical_node(json, "root/arm/attachment").1["extras"]
                    .get("partKey")
                    .is_none()
            );
            assert_eq!(
                logical_node(json, "root/arm/empty").1["extras"]["partKey"],
                "empty-result"
            );
            assert_eq!(
                export.stats.buffer_bytes,
                36 * 4 * 3
                    + compiled.parts()[0]
                        .bodies
                        .iter()
                        .chain(&compiled.parts()[2].bodies)
                        .map(|body| body.tri.positions.len() as u64 * 32)
                        .sum::<u64>()
            );
            if let Some(buffer) = &original_buffer {
                assert_eq!(document.bin(), buffer);
            }
            original_buffer = Some(document.bin().to_vec());

            let (mut actual, placements) = world_geometry(&document);
            let mut expected: Vec<_> = draw
                .items
                .iter()
                .flat_map(|item| {
                    compiled.part(item.part).unwrap().bodies[item.body as usize]
                        .tri
                        .positions
                        .iter()
                        .map(move |p| {
                            let p = p.map(f64::from);
                            let world = item
                                .world
                                .rows
                                .map(|r| r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + r[3]);
                            match coordinates {
                                GltfCoordinates::Preserve => world,
                                GltfCoordinates::ZUpToYUp => [world[0], world[2], -world[1]],
                            }
                        })
                })
                .collect();
            sort_points(&mut actual);
            sort_points(&mut expected);
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(expected) {
                for axis in 0..3 {
                    assert!(
                        (actual[axis] - expected[axis]).abs() < 1e-9,
                        "exported position {actual:?} differs from evaluated {expected:?}"
                    );
                }
            }
            if coordinates == GltfCoordinates::Preserve {
                let root_world = placements.iter().find(|(id, _)| *id == root_id).unwrap().1;
                let arm_world = placements.iter().find(|(id, _)| *id == arm_id).unwrap().1;
                assert_eq!(root_world.rows.map(|r| r[3]), [10.0, 20.0, 30.0]);
                assert_eq!(arm_world.rows.map(|r| r[3]), [11.0, 22.0, 30.0]);
            }
        }
    }
}

#[test]
fn material_pose_and_cache_release_keep_old_exports_usable_but_resized_sources_are_rejected() {
    let mut compiler = PartCompiler::new();
    let policy = CompilePolicy::default();
    let original = articulated_assembly(0.0, 1.0, "finish-a");
    let compiled = compiler.compile_parts(&original, &policy).unwrap();
    let old_export = export_glb(&original, &compiled).unwrap();
    let changed = articulated_assembly(0.6, 1.0, "finish-b");
    let changed_export = export_glb(&changed, &compiled).unwrap();
    assert_ne!(old_export.bytes, changed_export.bytes);
    assert_eq!(
        GlbDocument::parse(&old_export.bytes).unwrap().bin(),
        GlbDocument::parse(&changed_export.bytes).unwrap().bin()
    );
    assert_eq!(
        changed_export.stats.meshes, 3,
        "equal material bindings share both arm meshes and the marker"
    );
    assert!(matches!(
        export_glb(&articulated_assembly(0.0, 2.0, "finish-a"), &compiled),
        Err(GltfError::CompilationMismatch(
            CompilationMismatch::PartContent { .. }
        ))
    ));
    let empty = compiler.compile_parts(&Assembly::new(), &policy).unwrap();
    assert!(matches!(
        export_glb(&original, &empty),
        Err(GltfError::CompilationMismatch(
            CompilationMismatch::PartCount { .. }
        ))
    ));
    compiler.clear_cache();
    assert_eq!(
        old_export.bytes,
        export_glb(&original, &compiled).unwrap().bytes
    );
    assert_eq!(compiler.counters().parts_compiled, 3);
}

#[test]
fn error_level_partial_geometry_cannot_be_hidden_by_frame_nodes_or_cache_hits() {
    let mut builder = RecipeBuilder::new();
    let a = builder
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: [1.0; 3] },
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let b = builder
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: [1.0; 3] },
            placement: Placement3::translate(1.0, 1.0, 0.0),
        })
        .unwrap();
    let refused = builder
        .add(NodeKind::Csg {
            op: CsgOp::Union,
            operands: vec![a, b],
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Group {
            children: vec![a, refused],
        })
        .unwrap();
    let mut assembly = Assembly::new();
    let frame = assembly
        .add_frame(None, "assembly", Placement3::IDENTITY)
        .unwrap();
    let part = assembly
        .add_recipe_part("partial", builder.finish(root).unwrap())
        .unwrap();
    assembly
        .add_instance(Some(frame), "part", part, Placement3::IDENTITY)
        .unwrap();
    let mut compiler = PartCompiler::new();
    for _ in 0..2 {
        let compiled = compiler
            .compile_parts(&assembly, &CompilePolicy::default())
            .unwrap();
        assert!(!compiled.report(part).unwrap().clean_at(Severity::Error));
        let Err(GltfError::IncompleteGeometry { part: 0, report }) =
            export_glb(&assembly, &compiled)
        else {
            panic!("partial geometry must carry its report out of export");
        };
        assert_eq!(&*report, compiled.report(part).unwrap());
        drop(compiled);
        assert!(!report.diagnostics.is_empty());
    }
    assert_eq!(compiler.counters().parts_compiled, 1);
}

#[test]
fn frame_only_hierarchy_exports_without_geometry_records() {
    let mut assembly = Assembly::new();
    let root = assembly
        .add_frame(None, "assembly", Placement3::IDENTITY)
        .unwrap();
    assembly
        .add_frame(
            Some(root),
            "attachment",
            Placement3::translate(1.0, 2.0, 3.0),
        )
        .unwrap();
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    let export = export_glb(&assembly, &compiled).unwrap();
    let document = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(document.node_names(), ["assembly", "assembly/attachment"]);
    assert_eq!(document.json()["nodes"][0]["children"], json!([1]));
    for field in ["meshes", "accessors", "bufferViews", "buffers"] {
        assert!(
            document.json().get(field).is_none(),
            "frame-only export omits {field}"
        );
    }
    assert!(document.bin().is_empty());
}

#[test]
fn opaque_metadata_survives_escaped_and_unicode_instance_keys() {
    let mut assembly = Assembly::new();
    let root = assembly
        .add_frame(None, "root", Placement3::IDENTITY)
        .unwrap();
    let keys = [
        ("item%2F1", "item/1"),
        ("item%252F1", "item%2F1"),
        ("支架", "支架"),
    ];
    for (segment, original) in keys {
        let id = assembly
            .add_frame(Some(root), segment, Placement3::IDENTITY)
            .unwrap();
        assembly.set_metadata(id, "caller.key", original).unwrap();
    }
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    let export = export_glb(&assembly, &compiled).unwrap();
    let document = GlbDocument::parse(&export.bytes).unwrap();
    for (segment, original) in keys {
        let (_, node) = logical_node(document.json(), &format!("root/{segment}"));
        assert_eq!(node["extras"]["caller.key"], original);
    }
}
