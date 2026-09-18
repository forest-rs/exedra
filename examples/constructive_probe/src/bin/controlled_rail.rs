// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! An asymmetric rail with authored orientation and dimensional miter joins.
//!
//! Run `cargo run -p constructive_probe --bin controlled_rail -- target/controlled-rail.glb`.
//! Three instances show the same L section on slightly different spatial
//! paths. Section X stays authored as +X at the start, even across the legacy
//! automatic seed's world-axis boundary. Checks are local, not global solid
//! certification; the unit tests exercise failure cases and geometric oracles.

use exedra_assembly::{Assembly, PartCompiler};
use exedra_constructive::builders;
use exedra_constructive::evaluate::evaluate;
use exedra_constructive::ir::{CapMode, NodeKind, Path3, PathClosure, Placement3, RecipeBuilder};
use exedra_constructive::tessellate::EvalPolicy;
use exedra_gltf::{GltfExportOptions, export_glb_with_options};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/controlled-rail.glb".into());
    let policy = EvalPolicy::default();
    let mut assembly = Assembly::new();
    for (index, initial) in [[0.0, 0.0], [1e-6, 2e-6], [2e-6, 1e-6]]
        .into_iter()
        .enumerate()
    {
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(builders::l_profile(0.8, 0.6, 0.2, 0.15)?);
        let root = builder.add(NodeKind::Sweep {
            profile,
            path: Path3::MiteredPolyline {
                points: vec![
                    [0.0; 3],
                    [initial[0], initial[1], 4.0],
                    [3.0, 0.0, 4.0],
                    [3.0, 3.0, 4.0],
                ],
                section_x: [1.0, 0.0, 0.0],
                section_origin: [0.0; 2],
                closure: PathClosure::Open,
                miter_limit: 2.0,
            },
            caps: CapMode::Both,
        })?;
        let recipe = builder.finish(root)?;
        let evaluated = evaluate(&recipe, &policy)?;
        let body = &evaluated.bodies[0].body;
        assert!(body.mesh.validate_deep().is_empty(), "valid topology");
        assert!(
            body.mesh.boundary_loops()?.is_empty(),
            "both caps close the rail"
        );
        println!(
            "rail {index}: {:?}; global intersections unchecked",
            body.sweep_checks.expect("local checks")
        );
        let name = format!("controlled-rail-{index}");
        let part = assembly.add_recipe_part(&name, recipe)?;
        assembly.add_instance(
            None,
            &name,
            part,
            Placement3::translate(index as f64 * 5.0, 0.0, 0.0),
        )?;
    }
    let mut compiler = PartCompiler::new();
    let compiled = compiler.compile_parts(&assembly, &policy.into())?;
    let glb = export_glb_with_options(&assembly, &compiled, GltfExportOptions::z_up_to_y_up())?;
    std::fs::write(&out, glb.bytes)?;
    println!("wrote {out}");
    Ok(())
}
