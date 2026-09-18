// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fitted inside corners: line/cubic, arc/cubic, and cubic/cubic surrounds.
//! `cargo run -p constructive_probe --bin cubic_offsets -- target/cubic-offsets.glb`

use exedra_assembly::{Assembly, CompilePolicy, PartCompiler};
use exedra_constructive::{
    ir::{CapMode, NodeKind, Placement3, Recipe, RecipeBuilder},
    offset::{CornerPolicy, OffsetPolicy},
    profile::{Loop2, Profile2, Seg2},
};
use exedra_gltf::{GltfExportOptions, export_glb_with_options};
use std::path::PathBuf;

fn extrude(profile: Profile2, height: f64) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let profile = builder.add_profile(profile);
    let node = builder
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height,
            caps: CapMode::Both,
        })
        .expect("extrusion");
    builder.finish(node).expect("recipe")
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "target/cubic-offsets.glb".into());
    let mut assembly = Assembly::new();
    for (i, (name, top)) in [
        ("line-cubic", Seg2::line((0.0, 10.0))),
        ("arc-cubic", Seg2::arc((0.0, 10.0), 0.2)),
        (
            "cubic-cubic",
            Seg2::cubic((0.0, 10.0), (7.0, 12.0), (3.0, 12.0)),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let source = Profile2::simple(Loop2::new(vec![
            Seg2::line((10.0, 0.0)),
            Seg2::cubic((10.0, 10.0), (12.0, 3.0), (12.0, 7.0)),
            top,
            Seg2::line((0.0, 0.0)),
        ])?)?;
        let policy = OffsetPolicy::default();
        let opening = source.offset_with_policy(-0.5, CornerPolicy::Round, &policy)?;
        let insert = source.offset_with_policy(-0.75, CornerPolicy::Round, &policy)?;
        println!(
            "{name}: {} trim joins, {} interval steps",
            opening.trims.len(),
            opening.work.trim_steps
        );
        let surround = Profile2::new(
            source.outer().clone(),
            vec![opening.profile.outer().reversed()],
        )?;
        let frame =
            assembly.add_recipe_part(&format!("{name}-surround"), extrude(surround, 0.6))?;
        let panel =
            assembly.add_recipe_part(&format!("{name}-insert"), extrude(insert.profile, 0.3))?;
        let x = f64::from(u32::try_from(i).expect("three cases")) * 14.0;
        assembly.add_instance(
            None,
            &format!("{name}-surround"),
            frame,
            Placement3::translate(x, 0.0, 0.0),
        )?;
        assembly.add_instance(
            None,
            &format!("{name}-insert"),
            panel,
            Placement3::translate(x, 0.0, 0.15),
        )?;
    }
    let mut compiler = PartCompiler::new();
    let compiled = compiler.compile_parts(&assembly, &CompilePolicy::default())?;
    let glb = export_glb_with_options(&assembly, &compiled, GltfExportOptions::z_up_to_y_up())?;
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, glb.bytes)?;
    println!("{}", output.display());
    Ok(())
}
