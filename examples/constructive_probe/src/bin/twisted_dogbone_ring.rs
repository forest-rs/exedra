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
    ir::{
        CapMode, CsgOp, NodeKind, Path3, PathClosure, PathJoin, Placement3, PrimitiveSpec, Recipe,
        RecipeBuilder, SectionLaw,
    },
    path::PathSegment3,
    profile::{Loop2, Profile2, ProfileError, Seg2},
    tessellate::EvalPolicy,
};
use exedra_gltf::{GltfExportOptions, export_glb_with_materials};
use serde_json::json;

const RING_RADIUS: f64 = 50.0;
const END_OFFSET: f64 = 7.5;
const END_RADIUS: f64 = 1.5;
const WEB_HALF_WIDTH: f64 = 1.0;
const CUT_RADIUS: f64 = 5.0;
const CUT_DEPTH: f64 = 8.0;
const CUT_COUNT: usize = 21;

fn dogbone() -> Result<Profile2, ProfileError> {
    // Boundary of the union of a 15 x 2 rectangle and two radius-1.5 discs.
    // Each exposed circular lobe is a major arc between the web intersections.
    let inset = (END_RADIUS * END_RADIUS - WEB_HALF_WIDTH * WEB_HALF_WIDTH).sqrt();
    let inner = END_OFFSET - inset;
    let lobe_angle = core::f64::consts::TAU - 2.0 * (WEB_HALF_WIDTH / END_RADIUS).asin();
    let bulge = (lobe_angle / 4.0).tan();
    Profile2::simple(Loop2::new(vec![
        Seg2::line((inner, -WEB_HALF_WIDTH)),
        Seg2::arc((inner, WEB_HALF_WIDTH), bulge),
        Seg2::line((-inner, WEB_HALF_WIDTH)),
        Seg2::arc((-inner, -WEB_HALF_WIDTH), bulge),
    ])?)
}

fn cutter_placement(station: usize) -> Placement3 {
    let parameter = (station as f64 + 0.5) / CUT_COUNT as f64;
    let travel = core::f64::consts::TAU * parameter;
    let (sin, cos) = travel.sin_cos();
    let radial = [cos, sin, 0.0];
    let tangent = [-sin, cos, 0.0];
    let (twist_sin, twist_cos) = (4.0 * travel).sin_cos();
    // The guide is the section's thin direction. The cylinder crosses the
    // web and leaves the two round end rails intact.
    let axis = [radial[0] * twist_cos, radial[1] * twist_cos, -twist_sin];
    let transverse = [
        twist_sin * tangent[1],
        -twist_sin * tangent[0],
        axis[0] * tangent[1] - axis[1] * tangent[0],
    ];
    let base = [
        RING_RADIUS * radial[0] - 0.5 * CUT_DEPTH * axis[0],
        RING_RADIUS * radial[1] - 0.5 * CUT_DEPTH * axis[1],
        -0.5 * CUT_DEPTH * axis[2],
    ];
    let (phase_sin, phase_cos) = (core::f64::consts::PI / 64.0).sin_cos();
    let x = [
        tangent[0] * phase_cos + transverse[0] * phase_sin,
        tangent[1] * phase_cos + transverse[1] * phase_sin,
        tangent[2] * phase_cos + transverse[2] * phase_sin,
    ];
    let y = [
        transverse[0] * phase_cos - tangent[0] * phase_sin,
        transverse[1] * phase_cos - tangent[1] * phase_sin,
        transverse[2] * phase_cos - tangent[2] * phase_sin,
    ];
    Placement3::from_axes(x, y, axis, base)
}

fn ring_recipe(cut_count: usize, first_cut: usize) -> Result<Recipe, Box<dyn std::error::Error>> {
    let mut builder = RecipeBuilder::new();
    let profile = builder.add_profile(dogbone()?);
    let finish = builder.material_slot("finish");
    let sweep = builder.with_material(finish).add(NodeKind::Sweep {
        profile,
        path: Path3::Curves {
            start: [RING_RADIUS, 0.0, 0.0],
            segments: vec![PathSegment3::Arc {
                axis_origin: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                sweep: core::f64::consts::TAU,
            }],
            section_x: [0.0, 0.0, 1.0],
            section_origin: [0.0; 2],
            closure: PathClosure::ClosedPlanar {
                normal: [0.0, 0.0, 1.0],
            },
            joins: PathJoin::Smooth,
        },
        section: SectionLaw::IDENTITY.with_turns(4),
        caps: CapMode::None,
    })?;
    if cut_count == 0 {
        return Ok(builder.finish(sweep)?);
    }
    let mut operands = Vec::with_capacity(cut_count + 1);
    operands.push(sweep);
    for station in first_cut..first_cut + cut_count {
        let cutter = builder.with_material(finish).add(NodeKind::Primitive {
            spec: PrimitiveSpec::Cylinder {
                radius: CUT_RADIUS,
                height: CUT_DEPTH,
                segments: 64,
            },
            placement: cutter_placement(station),
        })?;
        operands.push(cutter);
    }
    let root = builder.add(NodeKind::Csg {
        op: CsgOp::Difference,
        operands,
    })?;
    Ok(builder.finish(root)?)
}

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
    let recipe = ring_recipe(cut_count, first_cut)?;
    let mut policy = EvalPolicy::default();
    policy.sweep_path.max_tangent_angle = 0.0125;
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
