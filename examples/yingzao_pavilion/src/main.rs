// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A small game-scene interpretation of a Chinese timber pavilion.

mod brackets;
mod geometry;
mod joinery;
mod layout;
mod rafters;
mod roof_section;
mod scene;
mod seats;
mod tiles;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::time::Instant;

use exedra_assembly::{Assembly, CompilePolicy, CompiledParts, PartCompiler, PartId, flatten};
use exedra_constructive::evaluate::Severity;
use exedra_gltf::{GltfExportOptions, export_glb_with_materials};
use exedra_mesh::NormalsSource;
use joiner::ContactMeaning;
use serde_json::{Value, json};

use layout::{Layout, Parameters};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let mut output = PathBuf::from("target/yingzao-pavilion");
    let mut parameters = Parameters::default();
    let mut variants = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" => output = args.next().ok_or("--output needs a directory")?.into(),
            "--bays" => parameters.bays = args.next().ok_or("--bays needs a count")?.parse()?,
            "--span-mm" => {
                parameters.span_mm = args.next().ok_or("--span-mm needs a length")?.parse()?;
            }
            "--depth-mm" => {
                parameters.depth_mm = args.next().ok_or("--depth-mm needs a length")?.parse()?;
            }
            "--variants" => variants = true,
            "--help" | "-h" => {
                println!(
                    "yingzao_pavilion [--output DIR] [--bays 1..5] [--span-mm 2400..5400] [--depth-mm 2400..5400] [--variants]"
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    std::fs::create_dir_all(&output)?;
    let mut metrics = vec![generate(&output, "pavilion", parameters, true)?];
    if variants {
        metrics.push(generate(
            &output,
            "span-4500",
            Parameters {
                span_mm: 4_500,
                ..parameters
            },
            false,
        )?);
        metrics.push(generate(
            &output,
            "three-bay",
            Parameters {
                bays: 3,
                ..parameters
            },
            false,
        )?);
    }
    let layout = Layout::resolve(parameters)?;
    for (name, study) in [
        ("bracket-study", scene::bracket_study(&layout)?),
        ("seat-study", scene::seat_study(&layout)?),
        ("concave-study", scene::concave_study()?),
        ("rafter-study", scene::rafter_study(&layout)?),
    ] {
        write_study(&output, name, &study)?;
    }
    std::fs::write(
        output.join("metrics.json"),
        serde_json::to_string_pretty(&metrics)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&metrics)?);
    Ok(())
}

fn write_study(output: &Path, name: &str, assembly: &Assembly) -> Result<()> {
    let compiled = PartCompiler::new().compile_parts(assembly, &compile_policy())?;
    check_reports(assembly, &compiled)?;
    let exported = export_glb_with_materials(
        assembly,
        &compiled,
        &flatten(assembly, &compiled),
        &material,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    std::fs::write(output.join(format!("{name}.glb")), exported.bytes)?;
    Ok(())
}

fn compile_policy() -> CompilePolicy {
    let mut policy = CompilePolicy {
        normals: NormalsSource::CustomOrDerived,
        ..CompilePolicy::default()
    };
    policy.evaluation.discretize.chord_tolerance = 0.001;
    policy
}

fn generate(
    output: &Path,
    name: &str,
    parameters: Parameters,
    material_variant: bool,
) -> Result<Value> {
    let start = Instant::now();
    let layout = Layout::resolve(parameters)?;
    let roof_frame = rafters::build(&layout)?;
    let mut assembly = scene::build_with_roof(&layout, &roof_frame)?;
    let build_ms = start.elapsed().as_secs_f64() * 1_000.0;
    let mut compiler = PartCompiler::new();
    let policy = compile_policy();
    let start = Instant::now();
    let compiled = compiler.compile_parts(&assembly, &policy)?;
    let compile_ms = start.elapsed().as_secs_f64() * 1_000.0;
    check_reports(&assembly, &compiled)?;
    let start = Instant::now();
    let contact_area = roof_frame.verify(&assembly, &compiled)?;
    let verify_ms = start.elapsed().as_secs_f64() * 1_000.0;
    let draw = flatten(&assembly, &compiled);
    let start = Instant::now();
    let export = export_glb_with_materials(
        &assembly,
        &compiled,
        &draw,
        &material,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    let export_ms = start.elapsed().as_secs_f64() * 1_000.0;
    std::fs::write(output.join(format!("{name}.glb")), &export.bytes)?;
    let mut metrics = json!({
        "bearing_contacts": roof_frame.construction.contacts().iter().filter(|c| c.meaning == ContactMeaning::Bearing).count(),
        "side_fit_contacts": roof_frame.construction.contacts().iter().filter(|c| c.meaning == ContactMeaning::SideFit).count(),
        "checked_contact_area_m2": contact_area, "contact_verify_ms": verify_ms,
        "name":name, "bays":parameters.bays, "span_mm":parameters.span_mm, "depth_mm":parameters.depth_mm,
        "build_ms": build_ms, "cold_compile_ms": compile_ms, "export_ms": export_ms,
        "parts": assembly.parts().len(), "instances": assembly.instances().len(),
        "unique_triangles": compiled.parts().iter().map(|p| p.triangle_count()).sum::<u64>(),
        "placed_triangles": draw.triangle_count(), "glb_bytes": export.bytes.len(),
        "geometry_bytes": export.stats.buffer_bytes,
    });
    if material_variant {
        let before = compiler.counters();
        let start = Instant::now();
        reassign_materials(&mut assembly)?;
        let edited = compiler.compile_parts(&assembly, &policy)?;
        let new_parts = compiler.counters().parts_compiled - before.parts_compiled;
        let new_triangles = compiler.counters().triangles_emitted - before.triangles_emitted;
        if new_parts != 0 || new_triangles != 0 {
            return Err("material edit recompiled geometry".into());
        }
        let reuse_ms = start.elapsed().as_secs_f64() * 1_000.0;
        let export = export_glb_with_materials(
            &assembly,
            &edited,
            &flatten(&assembly, &edited),
            &material,
            GltfExportOptions::z_up_to_y_up(),
        )?;
        std::fs::write(output.join("lacquered.glb"), &export.bytes)?;
        metrics["material_edit"] = json!({"new_parts_compiled":new_parts,"new_triangles":new_triangles,"rebind_and_cache_lookup_ms":reuse_ms});
    }
    Ok(metrics)
}

fn reassign_materials(assembly: &mut Assembly) -> Result<()> {
    // Select by caller-owned part identity. The same geometry receives new
    // opaque material IDs; no geometry-specific material representation.
    let parts: Vec<_> = assembly
        .parts()
        .iter()
        .enumerate()
        .map(|(index, part)| {
            (
                PartId(u32::try_from(index).expect("assembly part count fits u32")),
                part.key().to_owned(),
            )
        })
        .collect();
    for (part, key) in parts {
        let material = if key.contains("tile") {
            Some("glaze.green")
        } else if key.contains("column") && !key.contains("base")
            || key.contains("arm")
            || key.contains("beam")
            || key == "bearing-block"
        {
            Some("paint.vermilion")
        } else {
            None
        };
        if let Some(material) = material {
            assembly.set_part_material(part, "surface", material)?;
        }
    }
    Ok(())
}

fn check_reports(assembly: &Assembly, compiled: &CompiledParts) -> Result<()> {
    for (index, part) in assembly.parts().iter().enumerate() {
        // Successful compilation can include partial evaluations: refuse those too.
        if let Some(report) = compiled.report(PartId(u32::try_from(index)?))
            && !report.clean_at(Severity::Warning)
        {
            return Err(format!("geometry diagnostics on {:?}: {report:?}", part.key()).into());
        }
    }
    Ok(())
}

fn material(key: &str) -> Option<Value> {
    let (color, roughness) = match key {
        "contact" => ([0.06, 0.55, 0.32, 1.0], 0.65),
        "timber" => ([0.24, 0.085, 0.028, 1.0], 0.63),
        "timber.end" => ([0.34, 0.15, 0.055, 1.0], 0.7),
        "timber.dark" => ([0.085, 0.029, 0.012, 1.0], 0.72),
        "stone" => ([0.32, 0.30, 0.255, 1.0], 0.88),
        "stone.light" => ([0.46, 0.43, 0.36, 1.0], 0.88),
        "earth" => ([0.105, 0.13, 0.075, 1.0], 1.0),
        "tile.0" => ([0.10, 0.135, 0.14, 1.0], 0.77),
        "tile.1" => ([0.135, 0.17, 0.17, 1.0], 0.8),
        "tile.2" => ([0.075, 0.10, 0.105, 1.0], 0.74),
        "paint.vermilion" => ([0.42, 0.035, 0.012, 1.0], 0.43),
        "glaze.green" => ([0.035, 0.19, 0.115, 1.0], 0.3),
        _ => return None,
    };
    Some(json!({"pbrMetallicRoughness": {"baseColorFactor": color,
        "metallicFactor": 0.0, "roughnessFactor": roughness}}))
}
