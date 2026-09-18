// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Closed planar surrounds: asymmetric rabbet, curved molding, and Boolean cuts.
//!
//! `cargo run -p constructive_probe --bin closed_surround -- target/closed-surrounds.glb`
//! Paths contain unique stations and declare closure. No seam caps are emitted.
//! The middle specimen changes the perimeter while retaining the same physical
//! molding section. These are local construction checks, not global certificates.

use exedra_assembly::{Assembly, PartCompiler};
use exedra_constructive::builders;
use exedra_constructive::evaluate::{Severity, evaluate};
use exedra_constructive::ir::{
    CapMode, CsgOp, NodeKind, Path3, PathClosure, Placement3, PrimitiveSpec, Recipe, RecipeBuilder,
};
use exedra_constructive::profile::{Loop2, Profile2, Seg2, SegTag};
use exedra_constructive::tessellate::EvalPolicy;
use exedra_gltf::{GltfExportOptions, export_glb_with_options};
use std::path::PathBuf;

fn molding() -> Profile2 {
    Profile2::simple(
        Loop2::new(vec![
            Seg2::line((0.7, 0.0)).tagged(SegTag(0)),
            Seg2::line((0.7, 0.1)).tagged(SegTag(1)),
            Seg2::arc((0.4, 0.4), std::f64::consts::SQRT_2 - 1.0).tagged(SegTag(2)),
            Seg2::line((0.2, 0.4)).tagged(SegTag(3)),
            Seg2::line((0.2, 0.65)).tagged(SegTag(4)),
            Seg2::line((0.0, 0.65)).tagged(SegTag(5)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(6)),
        ])
        .expect("molding loop"),
    )
    .expect("molding profile")
}

fn surround(profile: Profile2, concave: bool, cuts: bool) -> Recipe {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(profile);
    let source = b.source_ref("surround");
    let finish = b.material_slot("finish");
    let points = if concave {
        vec![
            [0.0; 3],
            [10.0, 0.0, 0.0],
            [10.0, 3.5, 0.0],
            [6.0, 3.5, 0.0],
            [6.0, 6.0, 0.0],
            [0.0, 6.0, 0.0],
        ]
    } else {
        vec![
            [0.0; 3],
            [10.0, 0.0, 0.0],
            [10.0, 6.0, 0.0],
            [0.0, 6.0, 0.0],
        ]
    };
    let mut root = b
        .with_source(source)
        .with_material(finish)
        .add(NodeKind::Sweep {
            profile,
            path: Path3::MiteredPolyline {
                points,
                section_x: [0.0, 1.0, 0.0],
                section_origin: [0.0; 2],
                closure: PathClosure::ClosedPlanar {
                    normal: [0.0, 0.0, 1.0],
                },
                miter_limit: 2.0,
            },
            caps: CapMode::None,
        })
        .expect("surround");
    if cuts {
        for x in [2.3, 6.3] {
            let slot = b.material_slot("cut");
            let tool = b
                .with_material(slot)
                .add(NodeKind::Primitive {
                    spec: PrimitiveSpec::Box {
                        size: [0.7, 2.0, 2.0],
                    },
                    placement: Placement3::translate(x, -0.5, -0.5),
                })
                .expect("cutter");
            root = b
                .add(NodeKind::Csg {
                    op: CsgOp::Difference,
                    operands: vec![root, tool],
                })
                .expect("cut");
        }
    }
    b.finish(root).expect("recipe")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/closed-surrounds.glb"));
    let mut policy = EvalPolicy::default();
    policy.discretize.chord_tolerance = 0.0005;
    let mut assembly = Assembly::new();
    let specimens = [
        (
            "rabbet",
            surround(builders::l_profile(0.8, 0.6, 0.6, 0.45)?, false, false),
        ),
        ("molding", surround(molding(), false, false)),
        ("concave", surround(molding(), true, false)),
        (
            "cut-rabbet",
            surround(builders::l_profile(0.8, 0.6, 0.6, 0.45)?, false, true),
        ),
    ];
    for (i, (name, recipe)) in specimens.into_iter().enumerate() {
        let result = evaluate(&recipe, &policy)?;
        assert!(
            result.report.clean_at(Severity::Error),
            "{:?}",
            result.report.diagnostics
        );
        let body = &result.bodies[0].body;
        assert!(
            body.mesh.validate_deep().is_empty(),
            "{name} has valid mesh topology"
        );
        assert!(
            body.mesh.boundary_loops()?.is_empty(),
            "{name} has no open mesh boundary"
        );
        println!(
            "{name}: {} faces, {} vertices, local checks {:?}",
            body.mesh.faces().count(),
            body.mesh.vertices().count(),
            body.sweep_checks
        );
        let part = assembly.add_recipe_part(name, recipe)?;
        assembly.set_part_material(part, "finish", name)?;
        if name == "cut-rabbet" {
            assembly.set_part_material(part, "cut", "cut.gold")?;
        }
        let x = if i % 2 == 0 { 0.0 } else { 12.0 };
        let y = if i < 2 { 0.0 } else { 8.0 };
        assembly.add_instance(None, name, part, Placement3::translate(x, y, 0.0))?;
    }
    let compiled = PartCompiler::new().compile_parts(&assembly, &policy.into())?;
    let glb = export_glb_with_options(&assembly, &compiled, GltfExportOptions::z_up_to_y_up())?;
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, glb.bytes)?;
    println!("{}", output.display());
    Ok(())
}
