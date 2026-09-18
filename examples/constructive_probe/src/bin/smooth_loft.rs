// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Three authored asymmetric sections, their smooth loft, and a ruled comparison.
//!
//! Run `cargo run -p constructive_probe --bin smooth_loft -- target/smooth-loft.glb`.
//! Left: section slabs with seam markers. Center: smooth. Right: ruled.
//! The segment boundaries are correspondence landmarks; the marked seam starts
//! segment zero on every section. This is locally checked geometry, not a
//! certificate against arbitrary distant self-intersection.

use exedra_assembly::{Assembly, PartCompiler};
use exedra_constructive::evaluate::{Fidelity, evaluate};
use exedra_constructive::ir::{
    CapMode, LoftPolicy, LoftSection, NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder,
};
use exedra_constructive::profile::{Loop2, Profile2, Seg2};
use exedra_constructive::tessellate::EvalPolicy;
use exedra_gltf::{GltfExportOptions, export_glb_with_options};
use std::path::PathBuf;

fn profiles() -> Vec<(Placement3, Profile2)> {
    [
        (1.0, 1.0, [0.0, 0.0, 0.0]),
        (1.5, 0.8, [0.3, 0.1, 1.5]),
        (0.7, 1.2, [-0.1, 0.2, 3.0]),
    ]
    .into_iter()
    .map(|(x, y, offset)| {
        let source = Loop2::new(vec![
            Seg2::line((-0.5 * x, -0.3 * y)),
            Seg2::line((0.4 * x, -0.35 * y)),
            Seg2::line((0.65 * x, 0.05 * y)),
            Seg2::line((0.15 * x, 0.4 * y)),
            Seg2::line((-0.45 * x, 0.2 * y)),
        ])
        .expect("section");
        let profile =
            Profile2::simple(source.with_seam(1).expect("authored seam")).expect("profile");
        (
            Placement3::translate(offset[0], offset[1], offset[2]),
            profile,
        )
    })
    .collect()
}

fn loft(sections: &[(Placement3, Profile2)], policy: LoftPolicy) -> Recipe {
    let mut b = RecipeBuilder::new();
    let finish = b.material_slot("finish");
    let sections = sections
        .iter()
        .map(|(placement, profile)| LoftSection::new(*placement, b.add_profile(profile.clone())))
        .collect();
    let root = b
        .with_material(finish)
        .add(NodeKind::Loft {
            sections,
            policy,
            caps: CapMode::Both,
        })
        .expect("loft");
    b.finish(root).expect("recipe")
}

fn add(assembly: &mut Assembly, name: &str, recipe: Recipe, x: f64, material: &str) {
    let part = assembly.add_recipe_part(name, recipe).expect("part");
    assembly
        .set_part_material(part, "finish", material)
        .expect("material");
    assembly
        .add_instance(None, name, part, Placement3::translate(x, 0.0, 0.0))
        .expect("instance");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/smooth-loft.glb"));
    let sections = profiles();
    let mut policy = EvalPolicy::default();
    policy.loft.chord_tolerance = 0.001;
    let mut assembly = Assembly::new();
    for (name, mode, x, material) in [
        ("smooth", LoftPolicy::Smooth, 0.0, "smooth.ceramic"),
        ("ruled", LoftPolicy::Ruled, 2.4, "ruled.ceramic"),
    ] {
        let recipe = loft(&sections, mode);
        let result = evaluate(&recipe, &policy)?;
        assert_eq!(
            result.report.fidelity_of(recipe.root()),
            Some(Fidelity::Exact),
            "{name} must evaluate without fallback"
        );
        let body = &result.bodies[0].body;
        assert!(
            body.mesh.validate_deep().is_empty(),
            "{name} topology must be valid"
        );
        assert!(
            body.mesh.boundary_loops().expect("boundaries").is_empty(),
            "{name} caps must close the mesh"
        );
        if let Some(sampling) = &body.loft_sampling {
            println!(
                "{name}: {} authored sections, {} sampled bands, {} vertices per ring; trajectory tolerance {}",
                sections.len(),
                sampling.spans.len(),
                sampling.section_vertices,
                sampling.policy.chord_tolerance
            );
        }
        add(&mut assembly, name, recipe, x, material);
    }
    for (i, (placement, profile)) in sections.iter().enumerate() {
        let mut b = RecipeBuilder::new();
        let finish = b.material_slot("finish");
        let id = b.add_profile(profile.clone());
        let root = b.with_material(finish).add(NodeKind::Extrude {
            profile: id,
            placement: *placement,
            height: 0.025,
            caps: CapMode::Both,
        })?;
        add(
            &mut assembly,
            &format!("section-{i}"),
            b.finish(root)?,
            -2.4,
            "section.paper",
        );
        let (seam, _) = profile.outer().iter_with_starts().next().expect("seam");
        let mut b = RecipeBuilder::new();
        let finish = b.material_slot("finish");
        let root = b.with_material(finish).add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: [0.06; 3] },
            placement: Placement3::translate(
                placement.rows[0][3] + seam.x - 0.03,
                placement.rows[1][3] + seam.y - 0.03,
                placement.rows[2][3] + 0.025,
            ),
        })?;
        add(
            &mut assembly,
            &format!("seam-{i}"),
            b.finish(root)?,
            -2.4,
            "seam.gold",
        );
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
