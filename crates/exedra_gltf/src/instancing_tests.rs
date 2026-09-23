// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use exedra_assembly::{CompilePolicy, PartCompiler};
use exedra_mesh::{BuildParams, Mesh};

fn triangle() -> Mesh {
    Mesh::from_indexed_triangles(
        &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        &[[0, 1, 2]],
        &BuildParams::default(),
    )
    .unwrap()
}

fn gpu() -> GltfExportOptions<'static> {
    GltfExportOptions::default().with_instancing(GltfInstancing::GpuInstancing)
}

fn glb(assembly: &Assembly, options: GltfExportOptions<'_>) -> GlbExport {
    let compiled = PartCompiler::new()
        .compile_parts(assembly, &CompilePolicy::default())
        .unwrap();
    export_glb_with_options(assembly, &compiled, options).unwrap()
}

/// A grove: three trees under one frame, plus a lone rock.
fn grove() -> Assembly {
    let mut assembly = Assembly::new();
    let tree = assembly.add_baked_part("tree", triangle(), &[]).unwrap();
    let rock = assembly.add_baked_part("rock", triangle(), &[]).unwrap();
    let frame = assembly
        .add_frame(None, "grove", Placement3::translate(10.0, 0.0, 0.0))
        .unwrap();
    let mut scaled =
        Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_2, 0.0, 4.0, 0.0);
    for row in &mut scaled.rows {
        for c in &mut row[..3] {
            *c *= 2.0;
        }
    }
    for (key, placement) in [
        ("a", Placement3::IDENTITY),
        ("b", Placement3::translate(3.0, 0.0, 0.0)),
        ("c", scaled),
    ] {
        let id = assembly
            .add_instance(Some(frame), key, tree, placement)
            .unwrap();
        assembly.set_metadata(id, "species", "oak").unwrap();
    }
    assembly
        .add_instance(Some(frame), "rock", rock, Placement3::IDENTITY)
        .unwrap();
    assembly
}

#[test]
fn repeated_leaves_become_one_instanced_node() {
    let assembly = grove();
    let export = glb(&assembly, gpu());
    assert_eq!(export.stats.instanced_nodes, 1);
    assert_eq!(export.stats.batched_instances, 3);
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    // The frame, the lone rock, and one instanced node.
    assert_eq!(
        doc.node_names(),
        ["grove", "grove/rock", "tree [3 instances]"]
    );
    let j = doc.json();
    assert_eq!(j["extensionsUsed"], json!(["EXT_mesh_gpu_instancing"]));
    assert_eq!(j["extensionsRequired"], json!(["EXT_mesh_gpu_instancing"]));
    assert_eq!(j["nodes"][0]["children"], json!([1, 2]));

    let name = "tree [3 instances]";
    assert_eq!(
        doc.instancing_components(name, "TRANSLATION").unwrap(),
        [0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 4.0, 0.0]
    );
    let scale = doc.instancing_components(name, "SCALE").unwrap();
    assert_eq!(scale[..6], [1.0; 6]);
    assert!(scale[6..].iter().all(|s| (s - 2.0).abs() < 1e-6));
    let rotation = doc.instancing_components(name, "ROTATION").unwrap();
    assert_eq!(rotation[..8], [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let half = core::f32::consts::FRAC_1_SQRT_2;
    assert!((rotation[10] - half).abs() < 1e-6 && (rotation[11] - half).abs() < 1e-6);

    let extras = doc.node_extras(name).unwrap();
    assert_eq!(extras["partKey"], "tree");
    let instances = extras["instances"].as_array().unwrap();
    let paths: Vec<_> = instances
        .iter()
        .map(|i| i["instancePath"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["grove/a", "grove/b", "grove/c"]);
    assert_eq!(instances[2]["species"], "oak");
    assert!(instances.iter().all(|i| i.get("partKey").is_none()));

    let repeat = glb(&assembly, gpu());
    assert_eq!(repeat.bytes, export.bytes, "export is deterministic");
}

#[test]
fn default_export_keeps_one_node_per_instance() {
    let export = glb(&grove(), GltfExportOptions::default());
    assert_eq!(export.stats.instanced_nodes, 0);
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(doc.node_names().len(), 5);
    assert!(doc.json().get("extensionsUsed").is_none());
}

#[test]
fn mirrors_shears_materials_and_parents_split_batches() {
    let mut assembly = Assembly::new();
    let tree = assembly
        .add_baked_part("tree", triangle(), &["bark"])
        .unwrap();
    assembly.set_default_slot(tree, "bark").unwrap();
    assembly.set_part_material(tree, "bark", "oak").unwrap();
    let mut mirrored = Placement3::IDENTITY;
    mirrored.rows[0][0] = -1.0;
    let mut sheared = Placement3::IDENTITY;
    sheared.rows[0][1] = 0.5;
    let birch = assembly
        .add_instance(None, "birch", tree, Placement3::translate(9.0, 0.0, 0.0))
        .unwrap();
    assembly.bind_material(birch, "bark", "birch").unwrap();
    for (key, placement) in [
        ("a", Placement3::IDENTITY),
        ("b", Placement3::translate(1.0, 0.0, 0.0)),
        ("mirrored", mirrored),
        ("sheared", sheared),
    ] {
        assembly.add_instance(None, key, tree, placement).unwrap();
    }
    let frame = assembly
        .add_frame(None, "frame", Placement3::IDENTITY)
        .unwrap();
    assembly
        .add_instance(Some(frame), "under_frame", tree, Placement3::IDENTITY)
        .unwrap();
    // A reflection with no batch partner could never have batched, so it is
    // not counted.
    let lone = assembly
        .add_frame(None, "lone", Placement3::IDENTITY)
        .unwrap();
    assembly
        .add_instance(Some(lone), "lone_mirror", tree, mirrored)
        .unwrap();
    let export = glb(&assembly, gpu());
    assert_eq!(export.stats.unbatched_mirrored_instances, 1);
    assert_eq!(export.stats.unbatched_sheared_instances, 1);
    // `a` and `b` share a batch; the birch binding, the reflection, the shear
    // and the instance under another parent each keep their node.
    assert_eq!(export.stats.batched_instances, 2);
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(
        doc.node_names(),
        [
            "birch",
            "mirrored",
            "sheared",
            "frame",
            "frame/under_frame",
            "lone",
            "lone/lone_mirror",
            "tree [2 instances]"
        ]
    );
    let scene = &doc.json()["scenes"][0]["nodes"];
    assert_eq!(scene, &json!([0, 1, 2, 3, 5, 7]));
}

#[test]
fn multi_body_parts_batch_every_body_with_shared_transforms() {
    use exedra_constructive::ir::{NodeKind, PrimitiveSpec, RecipeBuilder};
    let mut builder = RecipeBuilder::new();
    let children = (0..2)
        .map(|i| {
            builder
                .add(NodeKind::Primitive {
                    spec: PrimitiveSpec::Box {
                        size: [0.5, 0.1, 0.5],
                    },
                    placement: Placement3::translate(0.0, f64::from(i) * 0.2, 0.0),
                })
                .unwrap()
        })
        .collect();
    let root = builder.add(NodeKind::Group { children }).unwrap();
    let mut assembly = Assembly::new();
    let part = assembly
        .add_recipe_part("pair", builder.finish(root).unwrap())
        .unwrap();
    for (key, x) in [("a", 0.0), ("b", 2.0)] {
        assembly
            .add_instance(None, key, part, Placement3::translate(x, 0.0, 0.0))
            .unwrap();
    }
    let export = glb(&assembly, gpu());
    assert_eq!(export.stats.instanced_nodes, 2);
    assert_eq!(export.stats.batched_instances, 2);
    let doc = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(
        doc.node_names(),
        ["pair [2 instances] [body 0]", "pair [2 instances] [body 1]"]
    );
    let j = doc.json();
    let attributes =
        |n: usize| &j["nodes"][n]["extensions"]["EXT_mesh_gpu_instancing"]["attributes"];
    assert_eq!(
        attributes(0),
        attributes(1),
        "bodies share one transform set"
    );
    assert_eq!(j["nodes"][1]["extras"]["body"], 1);
}

#[test]
fn z_up_conversion_parents_instanced_roots() {
    let mut assembly = Assembly::new();
    let part = assembly.add_baked_part("tree", triangle(), &[]).unwrap();
    for (key, x) in [("a", 0.0), ("b", 2.0)] {
        assembly
            .add_instance(None, key, part, Placement3::translate(x, 0.0, 0.0))
            .unwrap();
    }
    let options = GltfExportOptions::z_up_to_y_up().with_instancing(GltfInstancing::GpuInstancing);
    let doc = GlbDocument::parse(&glb(&assembly, options).bytes).unwrap();
    let j = doc.json();
    assert_eq!(j["scenes"][0]["nodes"], json!([1]));
    assert_eq!(j["nodes"][1]["children"], json!([0]));
}
