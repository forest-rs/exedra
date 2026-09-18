// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Snapshot polygon clearance: fitting plate, covered hole, and crossed notch.
//! `cargo run -p constructive_probe --bin footprint_clearance -- target/footprints.glb`

use exedra_assembly::{Assembly, CompilePolicy, PartCompiler};
use exedra_constructive::{
    builders,
    clearance::{BoundaryPolicy, PolygonClearancePolicy},
    ir::{CapMode, NodeKind, Placement3, Recipe, RecipeBuilder},
    profile::{Loop2, Profile2, Seg2},
    workplane::{SurfaceSelector, WorkplaneAttachment, WorkplanePolicy},
};
use exedra_gltf::{GltfExportOptions, export_glb_with_options};
use std::path::PathBuf;

fn outline(points: &[[f64; 2]]) -> Loop2 {
    Loop2::new(points.iter().map(|p| Seg2::line((p[0], p[1]))).collect()).expect("outline")
}
fn extrude(profile: Profile2, depth: f64) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let profile = builder.add_profile(profile);
    let slot = builder.material_slot("surface");
    let node = builder
        .with_material(slot)
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: depth,
            caps: CapMode::Both,
        })
        .expect("extrusion");
    builder.finish(node).expect("recipe")
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "target/footprints.glb".into());
    let outer = builders::rect_from_corner(10.0, 10.0)?.outer().clone();
    let holed = Profile2::new(
        outer,
        vec![outline(&[[4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0]])],
    )?;
    let notched = Profile2::simple(outline(&[
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 10.0],
        [7.0, 10.0],
        [7.0, 6.0],
        [6.0, 4.0],
        [4.0, 4.0],
        [3.0, 6.0],
        [3.0, 10.0],
        [0.0, 10.0],
    ]))?;
    let cases = [
        (
            "fits",
            holed.clone(),
            [[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0]],
            true,
        ),
        (
            "covers-hole",
            holed,
            [[3.0, 3.0], [7.0, 3.0], [7.0, 7.0], [3.0, 7.0]],
            false,
        ),
        (
            "crosses-notch",
            notched,
            [[1.0, 2.0], [9.0, 2.0], [9.0, 6.0], [1.0, 6.0]],
            false,
        ),
    ];
    let mut assembly = Assembly::new();
    let mut queries = Vec::new();
    for (i, (name, support, footprint, expected)) in cases.into_iter().enumerate() {
        let base = assembly.add_recipe_part(&format!("support-{name}"), extrude(support, 0.3))?;
        assembly.set_part_material(base, "surface", "support")?;
        let x = f64::from(u32::try_from(i).expect("three cases")) * 12.0;
        assembly.add_instance(
            None,
            &format!("support-{name}"),
            base,
            Placement3::translate(x, 0.0, 0.0),
        )?;
        let plate = assembly
            .add_recipe_part(name, extrude(Profile2::simple(outline(&footprint))?, 0.15))?;
        assembly.set_part_material(plate, "surface", name)?;
        // Lift the display plate to keep the underlying exclusion visible.
        // The query itself is in the support workplane, without this display gap.
        assembly.add_instance(None, name, plate, Placement3::translate(x, 0.0, 0.5))?;
        queries.push((base, footprint, expected, name));
    }
    let mut compiler = PartCompiler::new();
    let snapshot = compiler.compile_snapshot(&assembly, &CompilePolicy::default())?;
    compiler.clear_cache();
    for (part, points, expected, name) in queries {
        let body = snapshot.body(part, 0).expect("support body");
        let workplane = body.resolve_attachment(
            &WorkplaneAttachment {
                surface: SurfaceSelector::EndCap,
                anchor: [0.0; 3],
                projection: [0.0, 0.0, 1.0],
                x_direction: [1.0, 0.0, 0.0],
            },
            &WorkplanePolicy::default(),
        )?;
        let patch = workplane.planar_patch(&BoundaryPolicy::default())?;
        let result = patch.polygon_clearance(&points, &PolygonClearancePolicy::default())?;
        assert_eq!(result.is_contained(), expected, "{name}");
        println!("{name}: {result:?}");
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
