// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Retained hollow supports, queried and rendered from one evaluation snapshot.
//! `cargo run -p constructive_probe --bin retained_planes -- target/retained-planes.glb`

use exedra_assembly::{Assembly, CompilePolicy, PartCompiler};
use exedra_constructive::{
    builders::ring,
    ir::{NodeKind, Placement3, Plane3, PlaneSide, RecipeBuilder},
    section::{CutCap, SectionPolicy},
    tessellate::REGION_CAP_END,
    workplane::{WorkplanePolicy, WorkplaneSelection},
};
use exedra_gltf::{GltfExportOptions, export_glb_with_options};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/retained-planes.glb"));
    let mut assembly = Assembly::new();
    for (index, x) in [-2.0, 0.0, 2.0].into_iter().enumerate() {
        let mut recipe = RecipeBuilder::new();
        recipe.material_slot("surface");
        let cap = recipe.material_slot("cut");
        let profile = recipe.add_profile(ring(0.75, 0.5)?);
        let body = recipe.add(NodeKind::ExtrudeToPlane {
            profile,
            placement: Placement3::translate(x, 0.0, 0.0),
            plane: Plane3 {
                normal: [-0.35, -0.12, 1.0],
                distance: 2.2,
            },
        })?;
        let source = recipe.source_ref(&format!("support/{index}"));
        let root = recipe.with_source(source).add(NodeKind::PlaneCut {
            child: body,
            plane: Plane3 {
                normal: [0.0, 0.1, 1.0],
                distance: 0.2,
            },
            side: PlaneSide::Positive,
            cap: CutCap {
                region: 100,
                material: Some(cap),
            },
        })?;
        let part = assembly.add_recipe_part(&format!("support-{index}"), recipe.finish(root)?)?;
        assembly.set_default_slot(part, "surface")?;
        assembly.bind_region_slot(part, REGION_CAP_END, "cut")?;
        assembly.set_part_material(part, "surface", "surface.blue")?;
        assembly.set_part_material(part, "cut", "cut.gold")?;
        assembly.add_instance(
            None,
            &format!("support-{index}"),
            part,
            Placement3::IDENTITY,
        )?;
    }
    let mut policy = CompilePolicy::default();
    policy.evaluation.discretize.chord_tolerance = 0.002;
    let mut compiler = PartCompiler::new();
    let snapshot = compiler.compile_snapshot(&assembly, &policy)?;
    compiler.clear_cache();
    for item in &snapshot.render().items {
        let body = snapshot.body(item.part, item.body).expect("render body");
        let section = body.section(
            Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance: 0.85,
            },
            &SectionPolicy::default(),
        )?;
        assert_eq!(
            section.regions[0].holes.len(),
            1,
            "retained cuts must preserve the bore"
        );
        let plane = body.workplane(
            WorkplaneSelection::Region(REGION_CAP_END),
            [0.0, 0.0, 2.2],
            [1.0, 0.0, 0.35],
            &WorkplanePolicy::default(),
        )?;
        plane.check(&body)?;
        println!(
            "{}: section has {} hole; cap workplane deviation {:.3e}",
            body.source().unwrap_or("support"),
            section.regions[0].holes.len(),
            plane.plane().max_plane_deviation()
        );
    }
    let glb = export_glb_with_options(
        &assembly,
        snapshot.compiled(),
        GltfExportOptions::z_up_to_y_up(),
    )?;
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, glb.bytes)?;
    println!("{}", output.display());
    Ok(())
}
