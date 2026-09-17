// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Parametric mounting holes, semantic attachments, and planar clearance witnesses.
//! Run `cargo run -p material_gallery --bin semantic_attachments`.
//! The retained recipes contain the actual Boolean drills. One query snapshot
//! supplies both measurements and the exact evaluated bodies displayed in the
//! GLB. A presentation assembly selects those bodies and adds witness strokes;
//! it does not reevaluate the recipes. Clearance concerns the mounting face,
//! not the complete three-dimensional bore or minimum wall thickness.

use exedra_assembly::{Assembly, CompilePolicy, PartCompiler};
use exedra_constructive::{
    builders::{circle, rect, ring},
    clearance::{BoundaryPolicy, ClearanceDecision},
    evaluate::Severity,
    ir::{CapMode, CsgOp, NodeKind, Placement3, Plane3, Recipe, RecipeBuilder},
    tessellate::{EvalPolicy, tessellate_sweep},
    text::dump_recipe,
    workplane::{SurfaceSelector, WorkplaneAttachment, WorkplaneError},
};
use exedra_gltf::{GltfExportOptions, export_glb_with_materials};
use exedra_mesh::Mesh;
use std::{fmt::Write, path::PathBuf};

const HOLES: [[f64; 2]; 4] = [[0.45, 0.45], [1.55, 0.45], [0.45, 1.55], [1.55, 1.55]];
const RADIUS: f64 = 0.12;
const REQUIRED: f64 = 0.25;
fn attachment() -> WorkplaneAttachment {
    WorkplaneAttachment {
        surface: SurfaceSelector::EndCap,
        anchor: [0.0; 3],
        projection: [0.0, 0.0, 1.0],
        x_direction: [1.0, 0.0, 0.0],
    }
}

fn recipe(
    width: f64,
    height: f64,
    thickness: f64,
    slope: f64,
) -> Result<Recipe, Box<dyn std::error::Error>> {
    let mut b = RecipeBuilder::new();
    let panel_profile = b.add_profile(rect(width, height)?);
    let definition = b.add(NodeKind::ExtrudeToPlane {
        profile: panel_profile,
        placement: Placement3::IDENTITY,
        plane: Plane3 {
            normal: [-slope, 0.0, 1.0],
            distance: thickness,
        },
    })?;
    let source = b.source_ref("panel");
    // Explicitly share the support definition among queries and construction.
    let panel = b.with_source(source).add(NodeKind::Instance {
        of: definition,
        placement: Placement3::IDENTITY,
    })?;
    let hole_profile = b.add_profile(circle(RADIUS)?);
    let collar_profile = b.add_profile(ring(RADIUS + 0.07, RADIUS)?);
    let mut cutters = Vec::new();
    let mut collars = Vec::new();
    for (index, [x, y]) in HOLES.into_iter().enumerate() {
        cutters.push(b.add(NodeKind::Extrude {
            profile: hole_profile,
            placement: Placement3::translate(x, y, -1.0),
            height: 2.0,
            caps: CapMode::Both,
        })?);
        let source = b.source_ref(&format!("mount/{index}"));
        collars.push(b.with_source(source).add(NodeKind::Extrude {
            profile: collar_profile,
            placement: Placement3::translate(x, y, 0.006),
            height: 0.05,
            caps: CapMode::Both,
        })?);
    }
    let cutters = b.add(NodeKind::Group { children: cutters })?;
    let cutters = b.add(NodeKind::OnWorkplane {
        support: panel,
        child: cutters,
        attachment: attachment(),
    })?;
    let source = b.source_ref("drilled");
    let drilled = b.with_source(source).add(NodeKind::Csg {
        op: CsgOp::Difference,
        operands: vec![panel, cutters],
    })?;
    let collars = b.add(NodeKind::Group { children: collars })?;
    let collars = b.add(NodeKind::OnWorkplane {
        support: panel,
        child: collars,
        attachment: attachment(),
    })?;
    let root = b.add(NodeKind::Group {
        children: vec![panel, drilled, collars],
    })?;
    Ok(b.finish(root)?)
}

fn show(
    assembly: &mut Assembly,
    name: &str,
    mesh: Mesh,
    placement: Placement3,
    material: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let part = assembly.add_baked_part(name, mesh, &["surface"])?;
    assembly.set_default_slot(part, "surface")?;
    assembly.set_part_material(part, "surface", material)?;
    assembly.add_instance(None, name, part, placement)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/semantic-attachments.glb"));
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let cases = [
        ("baseline", 2.0, 2.0, 0.32, 0.0, [0.0, 0.0]),
        ("resized", 2.5, 2.25, 0.5, 0.0, [3.2, 0.0]),
        ("tilted", 2.0, 2.0, 0.38, 0.18, [0.0, 3.7]),
        ("clearance-failure", 1.82, 2.0, 0.32, 0.0, [3.2, 3.7]),
    ];
    let mut input = Assembly::new();
    let mut parts = Vec::new();
    for (name, width, height, thickness, slope, _) in cases {
        let recipe = recipe(width, height, thickness, slope)?;
        std::fs::write(
            output.with_file_name(format!("semantic-attachments-{name}.recipe")),
            dump_recipe(&recipe),
        )?;
        parts.push(input.add_recipe_part(name, recipe)?);
    }
    let mut compiler = PartCompiler::new();
    let mut policy = CompilePolicy::default();
    policy.evaluation.discretize.chord_tolerance = 0.002;
    let snapshot = compiler.compile_snapshot(&input, &policy)?;
    compiler.clear_cache();
    let mut display = Assembly::new();
    let mut report = String::from(
        "Planar mounting-face clearance in recipe units.\nMinimum margin: 0.25; decision tolerance: 0.00001.\nMeasurements use evaluated polygonal boundaries, not analytic curves or wall thickness.\n\n",
    );
    let mut failures = 0;
    for ((name, _, _, _, _, [x, y]), part) in cases.into_iter().zip(parts) {
        let evaluation = snapshot.compiled().report(part).expect("recipe report");
        assert!(
            evaluation.clean_at(Severity::Error),
            "{name}: construction must be complete: {:?}",
            evaluation.diagnostics
        );
        let support = snapshot.body_by_source(part, "panel")?;
        let plane = support.resolve_attachment(&attachment(), &policy.evaluation.workplane)?;
        let patch = plane.planar_patch(&BoundaryPolicy::default())?;
        let placement = Placement3::translate(x, y, 0.0);
        let drilled = snapshot.body_by_source(part, "drilled")?;
        assert!(
            drilled.geometry().mesh.validate_deep().is_empty(),
            "{name}: drilled topology must validate"
        );
        assert!(
            drilled.geometry().mesh.boundary_loops()?.is_empty(),
            "{name}: drilled panel must remain closed"
        );
        show(
            &mut display,
            &format!("{name}-panel"),
            drilled.geometry().mesh.clone(),
            placement,
            "surface.blue",
        )?;
        let _ = writeln!(
            report,
            "{name}: {} checked cap faces; {} boundary segments",
            plane.plane().faces().len(),
            patch.stats().boundary_edges
        );
        for (index, center) in HOLES.into_iter().enumerate() {
            let measured = patch.circle_clearance(center, RADIUS)?;
            let decision = measured.classify(REQUIRED, 1e-5)?;
            let failed = decision == ClearanceDecision::Violated;
            failures += usize::from(failed);
            let material = if failed {
                "clearance.red"
            } else {
                "clearance.green"
            };
            let collar = snapshot.body_by_source(part, &format!("mount/{index}"))?;
            show(
                &mut display,
                &format!("{name}-mount-{index}"),
                collar.geometry().mesh.clone(),
                placement,
                material,
            )?;
            let nearest = measured.nearest.point;
            let vector = [nearest[0] - center[0], nearest[1] - center[1]];
            let distance = vector[0].hypot(vector[1]);
            let start = [
                center[0] + RADIUS * vector[0] / distance,
                center[1] + RADIUS * vector[1] / distance,
                0.075,
            ];
            let end = [nearest[0], nearest[1], 0.075];
            let witness = tessellate_sweep(
                &circle(0.012)?,
                &Placement3::IDENTITY,
                &[plane.plane().to_body(start), plane.plane().to_body(end)],
                CapMode::Both,
                &EvalPolicy::default(),
            )?;
            show(
                &mut display,
                &format!("{name}-witness-{index}"),
                witness.mesh,
                placement,
                material,
            )?;
            let _ = writeln!(
                report,
                "  mount/{index}: margin {:.6}; {decision:?}; nearest {:?}; source {:?}",
                measured.clearance, nearest, measured.nearest.source.adjacent_feature
            );
        }
        let missing = WorkplaneAttachment {
            surface: SurfaceSelector::Region(999),
            ..attachment()
        };
        assert_eq!(
            support
                .resolve_attachment(&missing, &policy.evaluation.workplane)
                .unwrap_err(),
            WorkplaneError::EmptySelection,
            "missing surfaces must not reuse the successful attachment"
        );
    }
    assert_eq!(
        failures, 2,
        "only the two right-hand holes of the narrow panel should fail clearance"
    );
    let compiled = compiler.compile_parts(&display, &policy)?;
    let materials = |key: &str| {
        let color = match key {
            "surface.blue" => [0.025, 0.24, 0.39, 1.0],
            "clearance.green" => [0.04, 0.55, 0.23, 1.0],
            "clearance.red" => [0.85, 0.035, 0.025, 1.0],
            _ => return None,
        };
        Some(serde_json::json!({ "pbrMetallicRoughness": {
            "baseColorFactor": color, "metallicFactor": 0.0, "roughnessFactor": 0.45
        }}))
    };
    let glb = export_glb_with_materials(
        &display,
        &compiled,
        &materials,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    std::fs::write(&output, glb.bytes)?;
    std::fs::write(output.with_extension("txt"), &report)?;
    print!("{report}");
    println!("{}", output.display());
    Ok(())
}
