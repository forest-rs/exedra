// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! An authored garden courtyard with a fitted masonry moon gate.

mod architecture;
mod drainage;
mod geometry;
mod planting;
mod rocks;
mod scene;
mod wall_caps;

#[cfg(test)]
mod tests;

use exedra_assembly::{CompilePolicy, PartCompiler, PartId, flatten};
use exedra_constructive::{evaluate::Severity, ir::Placement3};
use exedra_gltf::{GltfExportOptions, export_glb_with_materials};
use exedra_mesh::NormalsSource;
use joiner::Construction;
use scene::Scene;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn policy() -> CompilePolicy {
    let mut p = CompilePolicy {
        normals: NormalsSource::CustomOrDerived,
        ..CompilePolicy::default()
    };
    p.evaluation.discretize.chord_tolerance = 0.001;
    p
}
fn main() -> Result<()> {
    let mut output = PathBuf::from("target/moon-gate-courtyard");
    let mut radius = 1550;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("moon_gate_courtyard [--output DIR] [--radius-mm 1350..1700]");
                return Ok(());
            }
            "--output" => output = args.next().ok_or("missing output path")?.into(),
            "--radius-mm" => radius = args.next().ok_or("missing radius")?.parse()?,
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    let start = Instant::now();
    let (scene, construction) = build(radius, Placement3::IDENTITY)?;
    let build_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let mut compiler = PartCompiler::new();
    let compiled = compiler.compile_parts(&scene.assembly, &policy())?;
    let compile_ms = start.elapsed().as_secs_f64() * 1000.0;
    for (i, part) in scene.assembly.parts().iter().enumerate() {
        if let Some(report) = compiled.report(PartId(u32::try_from(i)?))
            && !report.clean_at(Severity::Warning)
        {
            return Err(format!("{}: {:?}", part.key(), report.diagnostics).into());
        }
    }
    for (id, def) in scene.assembly.parts().iter().enumerate() {
        for (body, compiled) in compiled
            .part(PartId(u32::try_from(id)?))
            .ok_or("missing compiled part")?
            .bodies
            .iter()
            .enumerate()
        {
            compiled
                .tri
                .validate_geometry()
                .map_err(|error| format!("{} body {body}: {error}", def.key()))?;
        }
    }
    let draw = flatten(&scene.assembly, &compiled);
    let start = Instant::now();
    let exported = export_glb_with_materials(
        &scene.assembly,
        &compiled,
        &material,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    let export_ms = start.elapsed().as_secs_f64() * 1000.0;
    std::fs::create_dir_all(&output)?;
    std::fs::write(output.join("courtyard.glb"), &exported.bytes)?;
    let metrics = json!({
        "build_ms": build_ms,
        "compile_ms": compile_ms,
        "export_ms": export_ms,
        "gate_radius_mm": radius,
        "masonry_units": construction.elements().len() - 1,
        "parts": scene.assembly.parts().len(),
        "instances": scene.assembly.instances().len(),
        "spatial_frames": scene.assembly.instances().iter().filter(|i| i.part().is_none()).count(),
        "geometry_instances": scene.assembly.instances().iter().filter(|i| i.part().is_some()).count(),
        "body_instances": draw.items.len(),
        "stored_triangles": compiled.parts().iter().map(|p| p.triangle_count()).sum::<u64>(),
        "placed_triangles": draw.triangle_count(),
        "glb_bytes": exported.bytes.len(),
    });
    std::fs::write(
        output.join("metrics.json"),
        serde_json::to_string_pretty(&metrics)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&metrics)?);
    Ok(())
}
fn build(radius: u64, placement: Placement3) -> Result<(Scene, Construction)> {
    let mut scene = Scene::default();
    let construction = scene.group("courtyard", placement, |scene| {
        let construction = scene.group("entry", Placement3::IDENTITY, |scene| {
            architecture::gate(scene, radius)
        })?;
        scene.group("ground", Placement3::IDENTITY, architecture::ground)?;
        scene.group("enclosure", Placement3::IDENTITY, architecture::enclosure)?;
        let pavilion = yingzao_fashi_pavilion::pavilion(1, 2400, 2400)?;
        scene.assembly.append_selected(
            scene.parent,
            &pavilion,
            "shelter",
            Placement3::translate(-3.05, 6.3, -0.08),
            |_, instance| {
                instance.part().is_none_or(|part| {
                    pavilion
                        .part(part)
                        .is_some_and(|part| part.key() != "ground")
                })
            },
        )?;
        scene.group("planting", Placement3::IDENTITY, planting::garden)?;
        scene.group("rocks", Placement3::IDENTITY, rocks::garden)?;
        Ok(construction)
    })?;
    Ok((scene, construction))
}

fn material(key: &str) -> Option<Value> {
    let tone = key
        .rsplit('.')
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(2);
    let t = (f64::from(tone) - 2.0) * 0.014;
    let (color, roughness) = if key.starts_with("paving.") {
        ([0.30 + t, 0.32 + t, 0.30 + t, 1.0], 0.85)
    } else if key.starts_with("surround.") {
        ([0.23 + t, 0.26 + t, 0.26 + t, 1.0], 0.91)
    } else if key.starts_with("brick.") {
        ([0.19 + t, 0.21 + t, 0.20 + t, 1.0], 0.96)
    } else if key.starts_with("leaf.") {
        ([0.055 + t, 0.17 + t * 2.0, 0.023 + t * 0.6, 1.0], 0.7)
    } else if key.starts_with("tile") || key == "roof.tile" {
        ([0.07, 0.095, 0.10, 1.0], 0.87)
    } else {
        match key {
            "plaster" => ([0.78, 0.77, 0.70, 1.0], 0.97),
            "stone" | "stone.light" => ([0.38, 0.40, 0.35, 1.0], 0.92),
            "rock" => ([0.30, 0.32, 0.29, 1.0], 0.95),
            "earth" => ([0.075, 0.09, 0.047, 1.0], 1.0),
            "bark" => ([0.16, 0.115, 0.07, 1.0], 0.95),
            "bamboo" => ([0.17, 0.23, 0.065, 1.0], 0.7),
            "timber" | "timber.end" => ([0.20, 0.083, 0.03, 1.0], 0.73),
            "timber.dark" => ([0.07, 0.036, 0.019, 1.0], 0.77),
            "metal" => ([0.075, 0.075, 0.068, 1.0], 0.78),
            "drain" => ([0.025, 0.031, 0.026, 1.0], 1.0),
            _ => return None,
        }
    };
    Some(
        json!({"name":key,"doubleSided":key.starts_with("leaf."),"pbrMetallicRoughness":{"baseColorFactor":color,"metallicFactor":0.0,"roughnessFactor":roughness}}),
    )
}
