// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Authored arched surrounds at two sizes and a smooth circular surround.
//! `cargo run -p constructive_probe --bin curved_surround -- target/curved-surrounds.glb`

use exedra_assembly::{Assembly, CompilePolicy, PartCompiler};
use exedra_constructive::{
    builders,
    ir::{CapMode, NodeKind, Path3, PathClosure, PathJoin, Placement3, Recipe, RecipeBuilder},
    path::PathSegment3,
};
use exedra_gltf::{GltfExportOptions, export_glb_with_options};
use std::path::PathBuf;

fn surround(size: f64, circle: bool) -> Recipe {
    let mut builder = RecipeBuilder::new();
    // One physical molding section, independent of the path dimensions.
    let profile = builder.add_profile(builders::l_profile(0.8, 0.6, 0.6, 0.45).unwrap());
    let (start, segments, section_x, joins) = if circle {
        (
            [5.0, 0.0, 0.0],
            vec![PathSegment3::Arc {
                axis_origin: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                sweep: std::f64::consts::TAU,
            }],
            [-1.0, 0.0, 0.0],
            PathJoin::Smooth,
        )
    } else {
        (
            [0.0; 3],
            vec![
                PathSegment3::Line {
                    to: [10.0 * size, 0.0, 0.0],
                },
                PathSegment3::Line {
                    to: [10.0 * size, 6.0 * size, 0.0],
                },
                PathSegment3::Arc {
                    axis_origin: [5.0 * size, 6.0 * size, 0.0],
                    axis: [0.0, 0.0, 1.0],
                    sweep: std::f64::consts::PI,
                },
                PathSegment3::Line { to: [0.0; 3] },
            ],
            [0.0, 1.0, 0.0],
            PathJoin::Miter { limit: 2.0 },
        )
    };
    let source = builder.source_ref(if circle {
        "circular-molding"
    } else {
        "arched-molding"
    });
    let root = builder
        .with_source(source)
        .add(NodeKind::Sweep {
            profile,
            path: Path3::Curves {
                start,
                segments,
                section_x,
                section_origin: [0.0; 2],
                closure: PathClosure::ClosedPlanar {
                    normal: [0.0, 0.0, 1.0],
                },
                joins,
            },
            caps: CapMode::None,
        })
        .unwrap();
    builder.finish(root).unwrap()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "target/curved-surrounds.glb".into());
    let mut assembly = Assembly::new();
    for (name, size, circle, origin) in [
        ("arch", 1.0, false, [0.0, 0.0, 0.0]),
        ("larger-arch", 1.5, false, [13.0, 0.0, 0.0]),
        ("circle", 1.0, true, [36.0, 6.0, 0.0]),
    ] {
        let part = assembly.add_recipe_part(name, surround(size, circle))?;
        assembly.add_instance(
            None,
            name,
            part,
            Placement3::translate(origin[0], origin[1], origin[2]),
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
