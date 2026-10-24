// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Two interleaved, four-turn dogbone rings with radial circular cutouts.
//!
//! Run `cargo run -p constructive_probe --bin twisted_dogbone_ring -- target/twisted-dogbone-ring.glb`.
//! The second instance is rotated 25 degrees around the common centerline.

use std::path::PathBuf;

use exedra_assembly::{Assembly, PartCompiler};
use exedra_constructive::{
    evaluate::{Severity, evaluate},
    ir::Placement3,
    tessellate::EvalPolicy,
};
#[path = "../dogbone.rs"]
mod dogbone;
use dogbone::{CUT_COUNT, ring_recipe};
use exedra_gltf::{GltfExportOptions, export_glb_with_materials};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/twisted-dogbone-ring.glb"));
    let cut_count = std::env::args()
        .nth(2)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(CUT_COUNT);
    let first_cut = std::env::args()
        .nth(3)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(0);
    if first_cut + cut_count > CUT_COUNT {
        return Err("cut range exceeds the 21 authored stations".into());
    }
    let mut policy = EvalPolicy::default();
    policy.sweep_path.max_tangent_angle = 0.0125;
    let recipe = ring_recipe(cut_count, first_cut, &policy, false, None)?;
    let evaluated = evaluate(&recipe, &policy)?;
    assert!(
        evaluated.report.clean_at(Severity::Error),
        "diagnostics: {:?}\nCSG failures: {:?}",
        evaluated.report.diagnostics,
        evaluated.report.csg_failures
    );
    assert!(!evaluated.bodies.is_empty(), "ring retains a solid body");
    for body in &evaluated.bodies {
        assert!(
            body.body.mesh.validate_deep().is_empty(),
            "ring mesh is valid"
        );
        assert!(
            body.body.mesh.boundary_loops()?.is_empty(),
            "ring is closed"
        );
    }

    let mut assembly = Assembly::new();
    let part = assembly.add_recipe_part("twisted-dogbone", recipe)?;
    for (name, material, angle) in [
        ("first", "magenta", 0.0),
        ("second", "yellow", 25.0_f64.to_radians()),
    ] {
        let instance = assembly.add_instance(
            None,
            name,
            part,
            Placement3::rotate_z_then_translate(angle, 0.0, 0.0, 0.0),
        )?;
        assembly.bind_material(instance, "finish", material)?;
    }
    let compiled = PartCompiler::new().compile_parts(&assembly, &policy.into())?;
    let materials = |key: &str| match key {
        "magenta" => Some(json!({
            "pbrMetallicRoughness": {
                "baseColorFactor": [0.75, 0.10, 0.85, 1.0],
                "metallicFactor": 0.0,
                "roughnessFactor": 0.45
            },
            "doubleSided": true
        })),
        "yellow" => Some(json!({
            "pbrMetallicRoughness": {
                "baseColorFactor": [0.92, 0.96, 0.12, 1.0],
                "metallicFactor": 0.0,
                "roughnessFactor": 0.45
            },
            "doubleSided": true
        })),
        _ => None,
    };
    let glb = export_glb_with_materials(
        &assembly,
        &compiled,
        &materials,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, glb.bytes)?;
    println!(
        "wrote {}: {} cutouts, {} body component(s)",
        output.display(),
        cut_count,
        evaluated.bodies.len()
    );
    Ok(())
}
