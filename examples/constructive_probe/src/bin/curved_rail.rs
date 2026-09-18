// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Asymmetric rails on a circular arc, a planar inflection, and a spatial
//! line/arc/cubic path, all with authored initial section orientation.
//!
//! Run `cargo run -p constructive_probe --bin curved_rail -- target/curved-rail.glb`.
//! The printed bounds describe the centerline and tangent variation, not a
//! whole-surface error guarantee or global solid certification.

use exedra_assembly::{Assembly, PartCompiler};
use exedra_constructive::builders;
use exedra_constructive::evaluate::evaluate;
use exedra_constructive::ir::{
    CapMode, NodeKind, Path3, PathClosure, PathJoin, Placement3, RecipeBuilder,
};
use exedra_constructive::path::PathSegment3;
use exedra_constructive::tessellate::EvalPolicy;
use exedra_gltf::{GltfExportOptions, export_glb_with_options};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/curved-rail.glb".into());
    let quarter = std::f64::consts::FRAC_PI_2;
    let paths = [
        vec![PathSegment3::Arc {
            axis_origin: [4.0, 0.0, 0.0],
            axis: [0.0, 1.0, 0.0],
            sweep: quarter,
        }],
        vec![PathSegment3::Cubic {
            control1: [0.0, 0.0, 3.0],
            control2: [3.0, 0.0, 3.0],
            to: [3.0, 0.0, 6.0],
        }],
        vec![
            PathSegment3::Line {
                to: [0.0, 0.0, 3.0],
            },
            PathSegment3::Arc {
                axis_origin: [3.0, 0.0, 3.0],
                axis: [0.0, 1.0, 0.0],
                sweep: quarter,
            },
            PathSegment3::Cubic {
                control1: [5.0, 0.0, 6.0],
                control2: [6.0, 2.0, 6.0],
                to: [8.0, 2.0, 7.0],
            },
        ],
    ];
    let policy = EvalPolicy::default();
    let mut assembly = Assembly::new();
    for (index, segments) in paths.into_iter().enumerate() {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::l_profile(0.3, 0.2, 0.1, 0.05)?);
        let root = b.add(NodeKind::Sweep {
            profile,
            path: Path3::Curves {
                start: [0.0; 3],
                segments,
                section_x: [1.0, 0.0, 0.0],
                section_origin: [0.0; 2],
                closure: PathClosure::Open,
                joins: PathJoin::Smooth,
            },
            caps: CapMode::Both,
        })?;
        let recipe = b.finish(root)?;
        let evaluated = evaluate(&recipe, &policy)?;
        let body = &evaluated.bodies[0].body;
        assert!(body.mesh.validate_deep().is_empty(), "valid topology");
        assert!(body.mesh.boundary_loops()?.is_empty(), "closed caps");
        let (triangles, _) = body.mesh.to_trimesh(&exedra_mesh::ExtractParams::default());
        triangles.validate_geometry()?;
        let evidence = body.path_sampling.as_ref().expect("sampling evidence");
        let chord = evidence
            .spans
            .iter()
            .map(|s| s.chord_bound)
            .fold(0.0_f64, f64::max);
        let angle = evidence
            .spans
            .iter()
            .map(|s| s.tangent_angle_bound)
            .fold(0.0_f64, f64::max);
        println!(
            "rail {index}: {} spans, chord bound {chord:.6}, tangent bound {angle:.6} rad",
            evidence.spans.len()
        );
        let name = format!("curved-rail-{index}");
        let part = assembly.add_recipe_part(&name, recipe)?;
        assembly.add_instance(
            None,
            &name,
            part,
            Placement3::translate(index as f64 * 8.0, 0.0, 0.0),
        )?;
    }
    let mut compiler = PartCompiler::new();
    let compiled = compiler.compile_parts(&assembly, &policy.into())?;
    let glb = export_glb_with_options(&assembly, &compiled, GltfExportOptions::z_up_to_y_up())?;
    std::fs::write(&out, glb.bytes)?;
    println!("wrote {out}");
    Ok(())
}
