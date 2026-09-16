// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Three hollow supports terminate at one inclined plane. Run with an optional
//! output path: `cargo run -p constructive_probe --bin extrude_to_plane`.

use exedra_assembly::{Assembly, PartCompiler};
use exedra_constructive::{
    builders::{circle, rect},
    extrude::extrude_to_plane,
    ir::{Placement3, Plane3},
    profile::{Loop2, Profile2, Seg2},
    section::SectionPolicy,
    tessellate::{EvalPolicy, REGION_CAP_END},
};
use exedra_gltf::{GltfExportOptions, export_glb_with_options};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/extrude-to-plane.glb"));
    let mut assembly = Assembly::new();
    let mut policy = EvalPolicy::default();
    policy.discretize.chord_tolerance = 0.002;
    let target = Plane3 {
        normal: [-0.35, -0.12, 1.0],
        distance: 2.2,
    };
    for (index, x) in [-1.8, 0.0, 1.8].into_iter().enumerate() {
        let (outer, inner) = if index == 1 {
            (circle(0.65)?, circle(0.45)?)
        } else {
            (
                rect(1.2, 1.0)?,
                Profile2::simple(Loop2::new(vec![
                    Seg2::line((0.2, 0.2)),
                    Seg2::line((1.0, 0.2)),
                    Seg2::line((1.0, 0.8)),
                    Seg2::line((0.2, 0.8)),
                ])?)?,
            )
        };
        let profile = Profile2::new(outer.outer().clone(), vec![inner.outer().reversed()])?;
        let result = extrude_to_plane(
            &profile,
            &Placement3::translate(x, 0.0, 0.0),
            target,
            &policy,
            &SectionPolicy::default(),
        )?;
        assert!(
            result.body.mesh.validate_deep().is_empty(),
            "support topology must validate"
        );
        assert!(
            result.body.mesh.boundary_loops()?.is_empty(),
            "support caps must be closed"
        );
        assert_eq!(
            result.section.regions[0].holes.len(),
            1,
            "support bore must remain open"
        );
        println!(
            "support {index}: {} cut vertices; plane deviation {:.3e}",
            result.section.stats.section_vertices, result.section.stats.max_plane_deviation
        );
        let part = assembly.add_baked_part(
            &format!("support-{index}"),
            result.body.mesh,
            &["surface", "end"],
        )?;
        assembly.set_default_slot(part, "surface")?;
        assembly.bind_region_slot(part, REGION_CAP_END, "end")?;
        assembly.set_part_material(part, "surface", "surface.blue")?;
        assembly.set_part_material(part, "end", "cut.gold")?;
        assembly.add_instance(
            None,
            &format!("support-{index}"),
            part,
            Placement3::IDENTITY,
        )?;
    }
    let compiled = PartCompiler::new().compile_parts(&assembly, &policy.into())?;
    let glb = export_glb_with_options(&assembly, &compiled, GltfExportOptions::z_up_to_y_up())?;
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, glb.bytes)?;
    println!("{}", output.display());
    Ok(())
}
