// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One compiled part, four finishes, and material-only edits.
//!
//! Run `cargo run -p material_gallery -- <output-directory>`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use exedra_assembly::{Assembly, CompilePolicy, PartCompiler, flatten};
use exedra_constructive::ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};
use exedra_gltf::{GltfExportOptions, export_glb_with_materials};
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/material-gallery"));
    let mut recipe = RecipeBuilder::new();
    let finish = recipe.material_slot("finish");
    let root = recipe.with_material(finish).add(NodeKind::Primitive {
        spec: PrimitiveSpec::Cylinder {
            radius: 0.4,
            height: 1.2,
            segments: 64,
        },
        placement: Placement3::IDENTITY,
    })?;
    let mut assembly = Assembly::new();
    let part = assembly.add_recipe_part("sample", recipe.finish(root)?)?;
    assembly.set_part_material(part, "finish", "paint.red")?;

    // This table can instead be a closure adapting the application's own material
    // descriptions. It stays outside geometry and assembly compilation.
    let mut materials = BTreeMap::from([
        (
            "paint.red",
            json!({"pbrMetallicRoughness": {"baseColorFactor": [0.65, 0.025, 0.015, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.6}}),
        ),
        (
            "metal.gold",
            json!({"pbrMetallicRoughness": {"baseColorFactor": [0.83, 0.55, 0.18, 1.0], "metallicFactor": 1.0, "roughnessFactor": 0.16}}),
        ),
        (
            "metal.brushed",
            json!({"pbrMetallicRoughness": {"baseColorFactor": [0.55, 0.6, 0.65, 1.0], "metallicFactor": 1.0, "roughnessFactor": 0.65}}),
        ),
        (
            "plastic.blue",
            json!({"pbrMetallicRoughness": {"baseColorFactor": [0.015, 0.15, 0.65, 0.45], "metallicFactor": 0.0, "roughnessFactor": 0.25}, "alphaMode": "BLEND", "doubleSided": true}),
        ),
    ]);
    let mut instances = Vec::new();
    for (index, key) in ["paint.red", "metal.gold", "metal.brushed", "plastic.blue"]
        .into_iter()
        .enumerate()
    {
        let x = f64::from(u32::try_from(index)?) * 1.15;
        let instance =
            assembly.add_instance(None, key, part, Placement3::translate(x, 0.0, 0.0))?;
        assembly.bind_material(instance, "finish", key)?;
        instances.push(instance);
    }
    let mut compiler = PartCompiler::new();
    let compiled = compiler.compile_parts(&assembly, &CompilePolicy::default())?;
    let baseline = compiler.counters();
    let options = GltfExportOptions::z_up_to_y_up();
    std::fs::create_dir_all(&output)?;
    let mut write = |name: &str,
                     assembly: &Assembly,
                     materials: &BTreeMap<&str, Value>|
     -> Result<(), Box<dyn std::error::Error>> {
        let list = flatten(assembly, &compiled);
        let export = export_glb_with_materials(
            assembly,
            &compiled,
            &list,
            &|key: &str| materials.get(key).cloned(),
            options,
        )?;
        std::fs::write(output.join(name), &export.bytes)?;
        // Recompiling after a material edit is unnecessary, but asking for it
        // here proves the compiler still reuses the part.
        compiler.compile_parts(assembly, &CompilePolicy::default())?;
        assert_eq!(
            compiler.counters().parts_compiled,
            baseline.parts_compiled,
            "material edits must reuse compiled parts"
        );
        assert_eq!(
            compiler.counters().triangles_emitted,
            baseline.triangles_emitted,
            "material edits must emit no new geometry"
        );
        println!(
            "{name}: {} materials, {} mesh wrappers, {} geometry bytes, 0 new part compilations",
            export.stats.materials, export.stats.meshes, export.stats.buffer_bytes
        );
        Ok(())
    };
    write("materials.glb", &assembly, &materials)?;
    let paint = materials
        .get_mut("paint.red")
        .expect("table contains paint");
    paint["pbrMetallicRoughness"]["baseColorFactor"] = json!([0.02, 0.5, 0.08, 1.0]);
    paint["pbrMetallicRoughness"]["roughnessFactor"] = json!(0.2);
    write("edited.glb", &assembly, &materials)?;
    assembly.bind_material(instances[1], "finish", "paint.red")?;
    write("rebound.glb", &assembly, &materials)?;
    Ok(())
}
