// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Drill an angled panel at authored face-local coordinates. Run with an optional
//! output GLB path: `cargo run -p constructive_probe --bin face_workplane`.

use exedra_assembly::{Assembly, PartCompiler};
use exedra_constructive::{
    builders::{circle, rect},
    ir::{CapMode, Placement3},
    profile::Profile2,
    tessellate::{EvalPolicy, REGION_CAP_END, tessellate_extrude, tessellate_sweep},
    workplane::{WorkplanePolicy, WorkplaneSelection, face_workplane},
};
use exedra_gltf::{GltfExportOptions, export_glb_with_options};
use exedra_mesh::{
    FaceTriangulation, Mesh,
    boolean::{BooleanDiagnostics, BooleanOp, BooleanScratch, boolean_mesh},
};
use std::path::PathBuf;

fn add(assembly: &mut Assembly, name: &str, mesh: Mesh, material: &str) {
    let part = assembly
        .add_baked_part(name, mesh, &["surface"])
        .expect("part");
    assembly.set_default_slot(part, "surface").expect("slot");
    assembly
        .set_part_material(part, "surface", material)
        .expect("material");
    assembly
        .add_instance(None, name, part, Placement3::IDENTITY)
        .expect("instance");
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/face-workplane.glb"));
    let mut policy = EvalPolicy::default();
    policy.discretize.chord_tolerance = 0.002;
    let placement = Placement3::rotate_x_then_translate(0.5, -2.0, -1.0, 0.4);
    let panel = tessellate_extrude(&rect(4.0, 3.0)?, &placement, 0.35, CapMode::Both, &policy)?;
    let origin = placement.rows.map(|row| row[2] * 0.35 + row[3]);
    let frame = face_workplane(
        &panel,
        WorkplaneSelection::Region(REGION_CAP_END),
        origin,
        [1.0, 0.0, 0.0],
        &WorkplanePolicy::default(),
    )?;
    let mut drilled = panel.mesh.clone();
    let mut assembly = Assembly::new();
    let ring = Profile2::new(
        circle(0.32)?.outer().clone(),
        vec![circle(0.22)?.outer().reversed()],
    )?;
    for (i, [x, y]) in [[0.8, 0.7], [3.2, 0.7], [0.8, 2.3], [3.2, 2.3]]
        .into_iter()
        .enumerate()
    {
        let cutter = tessellate_extrude(
            &circle(0.22)?,
            &frame.placement_at([x, y, -0.8]),
            1.6,
            CapMode::Both,
            &policy,
        )?;
        drilled = boolean_mesh(
            &drilled,
            &cutter.mesh,
            BooleanOp::Difference,
            FaceTriangulation::Robust,
            &mut BooleanScratch::default(),
            &mut BooleanDiagnostics::default(),
        )?
        .mesh;
        let collar = tessellate_extrude(
            &ring,
            &frame.placement_at([x, y, 0.015]),
            0.08,
            CapMode::Both,
            &policy,
        )?;
        add(
            &mut assembly,
            &format!("collar-{i}"),
            collar.mesh,
            "cap.gold",
        );
    }
    assert!(
        drilled.validate_deep().is_empty(),
        "drilled panel topology must validate"
    );
    assert!(
        drilled.boundary_loops()?.is_empty(),
        "drilled panel must remain closed"
    );
    add(&mut assembly, "angled-panel", drilled, "surface.blue");
    // A raised triad shows the authored local axes without obscuring a bore.
    for (axis, name, material) in [
        (0, "local-X", "axis.red"),
        (1, "local-Y", "axis.green"),
        (2, "local-Z", "cap.gold"),
    ] {
        let base = [1.6, 1.1, 0.08];
        let mut end = base;
        end[axis] += 0.65;
        let axis_body = tessellate_sweep(
            &circle(0.022)?,
            &Placement3::IDENTITY,
            &[frame.to_body(base), frame.to_body(end)],
            CapMode::Both,
            &policy,
        )?;
        add(&mut assembly, name, axis_body.mesh, material);
    }
    let compiled = PartCompiler::new().compile_parts(&assembly, &policy.into())?;
    let glb = export_glb_with_options(&assembly, &compiled, GltfExportOptions::z_up_to_y_up())?;
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, glb.bytes)?;
    println!(
        "{} source faces; plane deviation {:.3e}",
        frame.faces().len(),
        frame.max_plane_deviation()
    );
    println!("{}", output.display());
    Ok(())
}
