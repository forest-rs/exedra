// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{CompilePolicy, PartCompiler, flatten};
use exedra_constructive::builders;
use exedra_constructive::ir::{CapMode, Node, NodeKind, Placement3, RecipeBuilder};
use exedra_mesh::{BuildParams, Mesh};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

fn example() -> (Assembly, CompiledParts) {
    let mut b = RecipeBuilder::new();
    let front = b.material_slot("front");
    let profile = b.add_profile(builders::rect_from_corner(40.0, 20.0).unwrap());
    let node = b
        .add_node(
            Node::new(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 10.0,
                caps: CapMode::Both,
            })
            .with_material(front),
        )
        .unwrap();
    let recipe = b.finish(node).unwrap();

    let mut asm = Assembly::new();
    let part = asm.add_recipe_part("panel", recipe).unwrap();
    asm.set_part_material(part, "front", "oak").unwrap();
    let a = asm
        .add_instance(None, "a", part, Placement3::IDENTITY)
        .unwrap();
    asm.set_metadata(a, "review.state", "ready").unwrap();
    asm.set_metadata(a, "instancePath", "opaque collision")
        .unwrap();
    let b2 = asm
        .add_instance(None, "b", part, Placement3::translate(60.0, 0.0, 0.0))
        .unwrap();
    asm.bind_material(b2, "front", "walnut").unwrap();

    let mut compiler = PartCompiler::new();
    let compiled = compiler
        .compile_parts(&asm, &CompilePolicy::default())
        .unwrap();
    (asm, compiled)
}

#[test]
fn resolved_materials_share_geometry_and_resolve_once_per_key() {
    let (mut assembly, _) = example();
    let part = assembly.part_by_key("panel").unwrap();
    assembly
        .add_instance(None, "c", part, Placement3::translate(120.0, 0.0, 0.0))
        .unwrap();
    let mut compiler = PartCompiler::new();
    let compiled = compiler
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    let counters = compiler.counters();
    let oak = json!({"pbrMetallicRoughness": {"baseColorFactor": [0.5, 0.25, 0.1, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.7}});
    let walnut = json!({"pbrMetallicRoughness": {"baseColorFactor": [0.1, 0.05, 0.02, 0.4], "metallicFactor": 0.2, "roughnessFactor": 0.3}, "emissiveFactor": [0.01, 0.02, 0.03], "alphaMode": "BLEND", "doubleSided": true});
    let table = BTreeMap::from([("oak", oak.clone()), ("walnut", walnut.clone())]);
    let calls = RefCell::new(Vec::new());
    let resolve = |key: &str| {
        calls.borrow_mut().push(key.to_owned());
        table.get(key).cloned()
    };
    let export = export_glb_with_materials(
        &assembly,
        &compiled,
        &resolve,
        GltfExportOptions::z_up_to_y_up(),
    )
    .unwrap();
    assert_eq!(*calls.borrow(), ["oak", "walnut"]);
    let document = GlbDocument::parse(&export.bytes).unwrap();
    let json = document.json();
    assert_eq!(document.material_names(), ["oak", "walnut"]);
    assert_eq!(
        json["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"],
        oak["pbrMetallicRoughness"]["baseColorFactor"]
    );
    assert_eq!(
        json["materials"][0]["pbrMetallicRoughness"]["roughnessFactor"],
        0.7
    );
    assert_eq!(
        json["materials"][1]["pbrMetallicRoughness"]["metallicFactor"],
        0.2
    );
    assert_eq!(
        json["materials"][1]["emissiveFactor"],
        walnut["emissiveFactor"]
    );
    assert_eq!(json["materials"][1]["alphaMode"], "BLEND");
    assert_eq!(json["materials"][1]["doubleSided"], true);
    assert!(json["materials"][1].get("alphaCutoff").is_none());
    assert_eq!(export.stats.meshes, 2);
    assert_eq!(json["nodes"][0]["mesh"], json["nodes"][2]["mesh"]);
    let first = &json["meshes"][0]["primitives"][0];
    let second = &json["meshes"][1]["primitives"][0];
    assert_ne!(first["material"], second["material"]);
    assert_eq!(first["attributes"], second["attributes"]);
    let index_view = |primitive: &Value| {
        &json["accessors"][usize::try_from(primitive["indices"].as_u64().unwrap()).unwrap()]["bufferView"]
    };
    assert_eq!(index_view(first), index_view(second));
    let tri = &compiled.part(part).unwrap().bodies[0].tri;
    assert_eq!(
        export.stats.buffer_bytes,
        (tri.positions.len() * 32 + tri.indices.len() * 4) as u64
    );
    assert_eq!(
        compiler.counters(),
        counters,
        "export performs no compilation"
    );
    let repeat = export_glb_with_materials(
        &assembly,
        &compiled,
        &resolve,
        GltfExportOptions::z_up_to_y_up(),
    )
    .unwrap();
    assert_eq!(export.bytes, repeat.bytes);
    let text = export_gltf_with_materials(
        &assembly,
        &compiled,
        &resolve,
        GltfExportOptions::z_up_to_y_up(),
    )
    .unwrap();
    let text_json: Value = serde_json::from_str(&text.json).unwrap();
    assert_eq!(text_json["materials"], json["materials"]);
    assert_eq!(text_json["meshes"], json["meshes"]);
    assert_eq!(text.stats, export.stats);
}

#[test]
fn appearance_edits_and_rebinding_reuse_compiled_parts() {
    let (mut assembly, _) = example();
    let mut compiler = PartCompiler::new();
    let compiled = compiler
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    let part = assembly.part_by_key("panel").unwrap();
    let tri_before = compiled.part(part).unwrap().bodies[0].tri.clone();
    let work_before = compiler.counters();
    let mut table = BTreeMap::from([
        ("oak", json!({"pbrMetallicRoughness": {}})),
        ("walnut", json!({"pbrMetallicRoughness": {}})),
    ]);
    let export = |assembly: &Assembly, table: &BTreeMap<&str, Value>| {
        export_glb_with_materials(
            assembly,
            &compiled,
            &|key: &str| table.get(key).cloned(),
            GltfExportOptions::default(),
        )
        .unwrap()
    };
    let before = export(&assembly, &table);
    let oak = table.get_mut("oak").unwrap();
    oak["pbrMetallicRoughness"]["roughnessFactor"] = json!(0.25);
    oak["pbrMetallicRoughness"]["baseColorFactor"] = json!([0.6, 0.2, 0.1, 1.0]);
    let after = export(&assembly, &table);
    assert_ne!(before.bytes, after.bytes);
    let old = GlbDocument::parse(&before.bytes).unwrap();
    let new = GlbDocument::parse(&after.bytes).unwrap();
    assert_eq!(
        old.bin(),
        new.bin(),
        "appearance edits preserve geometry bytes"
    );
    assert_eq!(
        old.json()["materials"][1],
        new.json()["materials"][1],
        "other material is unchanged"
    );
    for key in ["meshes", "accessors", "bufferViews"] {
        assert_eq!(old.json()[key], new.json()[key]);
    }
    assert_eq!(compiler.counters(), work_before);
    // Change only the second occurrence; the first keeps the part default.
    assembly
        .bind_material(assembly.roots()[1], "front", "oak")
        .unwrap();
    let rebound_export = export(&assembly, &table);
    assert_eq!(rebound_export.stats.materials, 1);
    assert_eq!(rebound_export.stats.meshes, 1);
    assert_eq!(before.stats.buffer_bytes, rebound_export.stats.buffer_bytes);
    let reused = compiler
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    assert!(Arc::ptr_eq(
        compiled.part(part).unwrap(),
        reused.part(part).unwrap()
    ));
    assert_eq!(
        compiler.counters().parts_compiled,
        work_before.parts_compiled
    );
    assert_eq!(
        compiler.counters().triangles_emitted,
        work_before.triangles_emitted
    );
    assert_eq!(reused.part(part).unwrap().bodies[0].tri, tri_before);
    assert_eq!(compiled.part(part).unwrap().bodies[0].tri, tri_before);
}

#[test]
fn strict_resolution_distinguishes_missing_from_unassigned() {
    let (assembly, compiled) = example();
    let missing = |_: &str| None;
    assert!(matches!(
        export_glb_with_materials(&assembly, &compiled, &missing, GltfExportOptions::default()),
        Err(GltfError::MissingMaterial { key }) if key == "oak"
    ));
    assert!(
        export_glb(&assembly, &compiled).is_ok(),
        "the preview exporter supplies deterministic material colors"
    );
    let body = &compiled.parts()[0].bodies[0];
    let triangles: Vec<[u32; 3]> = body
        .tri
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| [t[0], t[1], t[2]])
        .collect();
    let mesh =
        Mesh::from_indexed_triangles(&body.tri.positions, &triangles, &BuildParams::default())
            .unwrap();
    let mut assembly = Assembly::new();
    let part = assembly.add_baked_part("unassigned", mesh, &[]).unwrap();
    assembly
        .add_instance(None, "unassigned", part, Placement3::IDENTITY)
        .unwrap();
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .unwrap();
    let never_resolve =
        |_: &str| -> Option<Value> { panic!("unassigned is not a missing resource") };
    let export = export_glb_with_materials(
        &assembly,
        &compiled,
        &never_resolve,
        GltfExportOptions::default(),
    )
    .unwrap();
    let document = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(export.stats.materials, 0);
    assert!(document.json().get("materials").is_none());
    assert!(
        document.json()["meshes"][0]["primitives"][0]
            .get("material")
            .is_none()
    );
}

#[test]
fn empty_scene_omits_optional_arrays_and_binary_payload() {
    let assembly = Assembly::new();
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .expect("empty assembly compiles");
    for options in [
        GltfExportOptions::default(),
        GltfExportOptions::z_up_to_y_up(),
    ] {
        let export =
            export_glb_with_options(&assembly, &compiled, options).expect("empty scene exports");
        let document = GlbDocument::parse(&export.bytes).expect("valid JSON-only GLB");
        for name in ["meshes", "accessors", "bufferViews", "buffers", "materials"] {
            assert!(document.json().get(name).is_none(), "omit {name}");
        }
        assert_eq!(export.stats.buffer_bytes, 0);
        assert_eq!(export.stats.meshes, 0);
        let json_len = u32::from_le_bytes(export.bytes[12..16].try_into().expect("chunk length"));
        assert_eq!(
            export.bytes.len(),
            20 + usize::try_from(json_len).expect("JSON length"),
            "no empty BIN chunk"
        );
        if options.coordinates == GltfCoordinates::Preserve {
            assert!(document.json().get("nodes").is_none());
            assert!(document.json()["scenes"][0].get("nodes").is_none());
        } else {
            assert_eq!(document.node_names(), ["Exedra Z-up to glTF Y-up"]);
            assert!(document.json()["nodes"][0].get("children").is_none());
        }
    }
}

#[test]
fn empty_intersection_exports_identity_without_empty_geometry() {
    use exedra_constructive::evaluate::{Fidelity, Severity};
    use exedra_constructive::ir::{CsgOp, PrimitiveSpec};
    let mut builder = RecipeBuilder::new();
    let operands = [0.0, 2.0, 4.0].map(|x| {
        builder
            .add_node(NodeKind::Primitive {
                spec: PrimitiveSpec::Box { size: [1.0; 3] },
                placement: Placement3::translate(x, 0.0, 0.0),
            })
            .expect("box")
    });
    let root = builder
        .add_node(NodeKind::Csg {
            op: CsgOp::Intersection,
            operands: operands.to_vec(),
        })
        .expect("intersection");
    let recipe = builder.finish(root).expect("recipe");
    let mut assembly = Assembly::new();
    let part = assembly
        .add_recipe_part("empty-part", recipe)
        .expect("part");
    assembly
        .add_instance(None, "empty-instance", part, Placement3::IDENTITY)
        .expect("instance");
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &CompilePolicy::default())
        .expect("exact empty part compiles");
    let report = compiled.report(part).expect("evaluation report");
    assert_eq!(report.fidelity_of(root), Some(Fidelity::Exact));
    assert!(report.clean_at(Severity::Warning));
    let list = flatten(&assembly, &compiled);
    assert_eq!(list.triangle_count(), 0);
    for options in [
        GltfExportOptions::default(),
        GltfExportOptions::z_up_to_y_up(),
    ] {
        let export = export_gltf_with_options(&assembly, &compiled, options).expect("JSON export");
        let json: Value = serde_json::from_str(&export.json).expect("JSON");
        assert_eq!(json["nodes"][0]["name"], "empty-instance");
        assert_eq!(json["nodes"][0]["extras"]["partKey"], "empty-part");
        assert!(json["nodes"][0].get("mesh").is_none());
        for field in ["meshes", "accessors", "bufferViews", "buffers"] {
            assert!(json.get(field).is_none(), "empty geometry omits {field}");
        }
        let glb = export_glb_with_options(&assembly, &compiled, options).expect("GLB export");
        let document = GlbDocument::parse(&glb.bytes).expect("valid container");
        assert_eq!(document.json(), &json);
        assert!(document.bin().is_empty());
        assert_eq!(document.triangle_count(), 0);
        assert_eq!(glb.stats.meshes, 0);
        assert_eq!(glb.stats.buffer_bytes, 0);
    }
}

#[test]
fn structural_validity() {
    let (asm, compiled) = example();
    let export = export_gltf(&asm, &compiled).unwrap();
    let doc: Value = serde_json::from_str(&export.json).unwrap();

    assert_eq!(doc["asset"]["version"], "2.0");
    assert_eq!(doc["scene"], 0);
    let nodes = doc["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2, "one node per logical instance");
    assert_eq!(nodes[0]["extras"]["instancePath"], "a");
    assert_eq!(nodes[1]["extras"]["partKey"], "panel");
    assert!(
        nodes[1]["matrix"].as_array().is_some(),
        "translated node carries a matrix"
    );

    // Different material resolutions force distinct meshes here.
    let meshes = doc["meshes"].as_array().unwrap();
    assert_eq!(meshes.len(), 2);
    assert_eq!(export.stats.meshes, 2);

    // Buffer byteLength matches the embedded payload exactly.
    let byte_length = doc["buffers"][0]["byteLength"].as_u64().unwrap();
    let uri = doc["buffers"][0]["uri"].as_str().unwrap();
    let payload = uri.split(',').nth(1).unwrap();
    assert_eq!(payload.len() % 4, 0);
    let padding = payload.bytes().rev().take_while(|b| *b == b'=').count();
    assert_eq!(
        usize::try_from(byte_length).unwrap(),
        payload.len() / 4 * 3 - padding,
        "buffer length must match the base64 payload"
    );

    // Every accessor's bufferView window stays inside the buffer, and
    // every index accessor holds whole triangles.
    let views = doc["bufferViews"].as_array().unwrap();
    for view in views {
        let offset = view["byteOffset"].as_u64().unwrap();
        let length = view["byteLength"].as_u64().unwrap();
        assert!(offset + length <= byte_length);
    }
    for accessor in doc["accessors"].as_array().unwrap() {
        if accessor["type"] == "SCALAR" {
            assert_eq!(accessor["count"].as_u64().unwrap() % 3, 0);
        }
        if accessor["type"] == "VEC3" && accessor.get("min").is_some() {
            assert_eq!(accessor["min"].as_array().unwrap().len(), 3);
            assert_eq!(accessor["max"].as_array().unwrap().len(), 3);
        }
    }

    // Materials: oak and walnut, named, deterministic colors.
    let materials = doc["materials"].as_array().unwrap();
    let names: Vec<&str> = materials
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["oak", "walnut"]);
}

#[test]
fn shared_resolution_shares_meshes() {
    let (mut asm, compiled) = example();
    // Make occurrence b's materials identical to a's: same mesh key.
    asm.bind_material(asm.roots()[1], "front", "oak").unwrap();
    let export = export_gltf(&asm, &compiled).unwrap();
    assert_eq!(export.stats.nodes, 2);
    assert_eq!(export.stats.meshes, 1, "identical items share one mesh");
}

#[test]
fn z_up_to_y_up_uses_one_right_handed_scene_root() {
    let (asm, compiled) = example();
    let export =
        export_gltf_with_options(&asm, &compiled, GltfExportOptions::z_up_to_y_up()).unwrap();
    let doc: Value = serde_json::from_str(&export.json).unwrap();
    let nodes = doc["nodes"].as_array().unwrap();
    let root_index = usize::try_from(doc["scenes"][0]["nodes"][0].as_u64().unwrap()).unwrap();
    let root = &nodes[root_index];

    assert_eq!(root["children"], json!([0, 1]));
    assert_eq!(
        root["matrix"],
        json!([
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0
        ])
    );
    assert_eq!(export.stats.nodes, 3);
    assert_eq!(nodes[0]["extras"]["instancePath"], "a");
}

#[test]
fn deterministic_output() {
    let (asm, compiled) = example();
    let a = export_gltf(&asm, &compiled).unwrap();
    let b = export_gltf(&asm, &compiled).unwrap();
    assert_eq!(a.json, b.json);
    assert_eq!(a.stats, b.stats);
}

#[test]
fn binary_export_has_valid_glb_chunks() {
    let (asm, compiled) = example();
    let export =
        export_glb_with_options(&asm, &compiled, GltfExportOptions::z_up_to_y_up()).unwrap();
    let bytes = &export.bytes;

    assert_eq!(&bytes[0..4], b"glTF");
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
    assert_eq!(
        usize::try_from(u32::from_le_bytes(bytes[8..12].try_into().unwrap())).unwrap(),
        bytes.len()
    );
    let json_len = usize::try_from(u32::from_le_bytes(bytes[12..16].try_into().unwrap())).unwrap();
    assert_eq!(&bytes[16..20], b"JSON");
    let json_end = 20 + json_len;
    let document: Value = serde_json::from_slice(&bytes[20..json_end]).unwrap();
    assert_eq!(document["asset"]["version"], "2.0");
    assert!(document["buffers"][0].get("uri").is_none());
    assert_eq!(&bytes[json_end + 4..json_end + 8], b"BIN\0");
    let bin_len = usize::try_from(u32::from_le_bytes(
        bytes[json_end..json_end + 4].try_into().unwrap(),
    ))
    .unwrap();
    assert_eq!(json_end + 8 + bin_len, bytes.len());
    assert!(bin_len >= usize::try_from(export.stats.buffer_bytes).unwrap());
    assert!(bin_len - usize::try_from(export.stats.buffer_bytes).unwrap() < 4);
}

#[test]
fn binary_output_is_deterministic() {
    let (asm, compiled) = example();
    let a = export_glb(&asm, &compiled).unwrap();
    let b = export_glb(&asm, &compiled).unwrap();
    assert_eq!(a.bytes, b.bytes);
    assert_eq!(a.stats, b.stats);
}

#[test]
fn binary_export_has_semantic_inspection_queries() {
    // Integration tests should ask about exporter semantics instead of
    // slicing GLB offsets or depending on serialized JSON whitespace and
    // key order. The coordinate-conversion root leaves mesh accessors in
    // the authored local frame by design.
    let (asm, compiled) = example();
    let export =
        export_glb_with_options(&asm, &compiled, GltfExportOptions::z_up_to_y_up()).unwrap();
    let document = GlbDocument::parse(&export.bytes).expect("exporter emits valid GLB");

    assert_eq!(
        document.node_names(),
        vec!["a", "b", "Exedra Z-up to glTF Y-up"]
    );
    let extras = document.node_extras("a").expect("named node extras");
    assert_eq!(extras["instancePath"], "a");
    assert_eq!(extras["partKey"], "panel");
    assert_eq!(extras["review.state"], "ready");
    assert_eq!(document.material_names(), vec!["oak", "walnut"]);
    assert_eq!(
        document.position_bounds(),
        Some(([0.0, 0.0, 0.0], [40.0, 20.0, 10.0]))
    );
    assert_eq!(document.triangle_count(), 24);
    assert_eq!(document.json()["asset"]["version"], "2.0");
    assert!(!document.bin().is_empty());
}
