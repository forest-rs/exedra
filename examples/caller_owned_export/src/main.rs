// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Serialize, compile and export construction without an Assembly or native stage.

use exedra::compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource, PreparedSource};
use exedra::constructive::{
    interchange::{RecipeDto, from_dto, to_dto},
    ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder},
};
use exedra::gltf::{
    GlbDocument, GltfExportOptions, GltfMesh, GltfNode, GltfScene, export_glb_with_materials,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = RecipeBuilder::new();
    let root = builder.add(NodeKind::Primitive {
        spec: PrimitiveSpec::Box {
            size: [2.0, 3.0, 1.0],
        },
        placement: Placement3::IDENTITY,
    })?;
    let json = serde_json::to_string(&to_dto(&builder.finish(root)?))?;
    let dto: RecipeDto = serde_json::from_str(&json)?;
    let source = PreparedSource::new(PartSource::Recipe(from_dto(&dto)?));
    let inputs = [PartInput::prepared("panel", &source)];
    let mut compiler = PartCompiler::with_capacity(16);
    let compiled = compiler.compile_parts(&inputs, &CompilePolicy::default())?;

    // The caller prepares hierarchy and appearance using its own application data.
    let mut mesh = GltfMesh::new(PartId(0), "panel", &compiled)?;
    mesh.materials = compiled
        .part(PartId(0))
        .expect("one input")
        .bodies
        .iter()
        .map(|body| vec![Some("paint".into()); body.regions.len()])
        .collect();
    let frame = GltfNode::new("product");
    let mut panel = GltfNode::new("panel");
    panel.parent = Some(0);
    panel.transform = Placement3::translate(4.0, 0.0, 0.0);
    panel.geometry = Some(mesh.into());
    panel.extras.insert("component".into(), "panel".into());
    let scene = GltfScene {
        nodes: vec![frame, panel],
        ..GltfScene::default()
    };
    let resolve = |key: &str| {
        (key == "paint").then(|| {
            serde_json::json!({
                "name": "paint", "pbrMetallicRoughness": {
                    "baseColorFactor": [0.2, 0.4, 0.7, 1.0], "metallicFactor": 0.0
                }
            })
        })
    };
    let export =
        export_glb_with_materials(&scene, &compiled, &resolve, GltfExportOptions::default())?;
    let document = GlbDocument::parse(&export.bytes)?;
    assert_eq!(
        document.node_names(),
        ["product", "panel"],
        "retain hierarchy"
    );
    assert_eq!(
        document.material_names(),
        ["paint"],
        "resolve caller materials"
    );
    assert_eq!(document.triangle_count(), 12, "complete box geometry");
    let second = compiler.compile_parts(&inputs, &CompilePolicy::default())?;
    assert_eq!(
        compiler.counters().parts_compiled,
        1,
        "reuse installed geometry"
    );
    assert_eq!(
        export.bytes,
        export_glb_with_materials(&scene, &second, &resolve, GltfExportOptions::default())?.bytes,
        "repeat export must be deterministic"
    );
    println!(
        "Serialized recipe: {} bytes; GLB: {} bytes; work: {:?}",
        json.len(),
        export.bytes.len(),
        compiler.counters()
    );
    Ok(())
}
