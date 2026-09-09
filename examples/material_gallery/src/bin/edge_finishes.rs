// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Compare a true 3D rail fillet, rounded-profile extrusion, and selected foot chamfer.

use std::error::Error;
use std::path::PathBuf;

use exedra_assembly::{Assembly, CompilePolicy, NormalsSource, PartCompiler, flatten};
use exedra_constructive::builders;
use exedra_constructive::edge_finish::{EdgeSelection, RoundPolicy};
use exedra_constructive::ir::{
    CapMode, NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder,
};
use exedra_gltf::{GltfExportOptions, export_glb_with_materials};
use serde_json::json;

fn rail(profile_rounding: bool) -> Result<Recipe, Box<dyn Error>> {
    let mut b = RecipeBuilder::new();
    let surface = b.material_slot("surface");
    let root = if profile_rounding {
        let profile = b.add_profile(builders::rounded_rect(0.09, 0.2, 0.006)?);
        b.with_material(surface).add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 2.0,
            caps: CapMode::Both,
        })?
    } else {
        let child = b.with_material(surface).add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box {
                size: [0.09, 0.2, 2.0],
            },
            placement: Placement3::IDENTITY,
        })?;
        let mut policy = RoundPolicy::fillet(0.006);
        policy.chord_tolerance = 0.00002;
        b.add(NodeKind::EdgeFinish {
            child,
            selection: EdgeSelection::SharpEdges,
            policy,
        })?
    };
    Ok(b.finish(root)?)
}

fn foot() -> Result<Recipe, Box<dyn Error>> {
    let mut b = RecipeBuilder::new();
    let surface = b.material_slot("surface");
    let child = b.with_material(surface).add(NodeKind::Primitive {
        spec: PrimitiveSpec::Box {
            size: [0.09, 0.2, 0.15],
        },
        placement: Placement3::IDENTITY,
    })?;
    // Stable +X/+Y box regions identify one exposed vertical edge.
    let root = b.add(NodeKind::EdgeFinish {
        child,
        selection: EdgeSelection::RegionBoundaries(vec![[1, 3]]),
        policy: RoundPolicy::chamfer(0.006),
    })?;
    Ok(b.finish(root)?)
}

fn compile_policy() -> CompilePolicy {
    let mut policy = CompilePolicy {
        normals: NormalsSource::CustomOrDerived,
        ..CompilePolicy::default()
    };
    policy.evaluation.discretize.chord_tolerance = 0.00002;
    policy
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/edge-finishes"));
    std::fs::create_dir_all(&output)?;
    for (name, recipe) in [
        ("rail-fillet", rail(false)?),
        ("rail-profile", rail(true)?),
        ("foot-chamfer", foot()?),
    ] {
        let mut assembly = Assembly::new();
        let part = assembly.add_recipe_part(name, recipe)?;
        assembly.set_part_material(part, "surface", "preview.metal")?;
        assembly.add_instance(None, name, part, Placement3::IDENTITY)?;
        let mut compiler = PartCompiler::new();
        let compiled = compiler.compile_parts(&assembly, &compile_policy())?;
        let draw = flatten(&assembly, &compiled);
        let glb = export_glb_with_materials(
            &assembly,
            &compiled,
            &draw,
            &|_: &str| {
                Some(
                    json!({"pbrMetallicRoughness": {"baseColorFactor": [0.16, 0.32, 0.46, 1.0], "metallicFactor": 0.65, "roughnessFactor": 0.24}}),
                )
            },
            GltfExportOptions::z_up_to_y_up(),
        )?;
        std::fs::write(output.join(format!("{name}.glb")), glb.bytes)?;
        println!(
            "{name}: {} triangles, bounds {:?}",
            draw.triangle_count(),
            draw.bounds()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_constructive::evaluate::evaluate;
    use exedra_constructive::tessellate::Feature;
    use std::rc::Rc;

    #[test]
    fn finished_rail_geometry_is_shared_across_placements_and_material_edits() {
        let mut assembly = Assembly::new();
        let part = assembly
            .add_recipe_part("rail", rail(false).unwrap())
            .unwrap();
        let a = assembly
            .add_instance(None, "a", part, Placement3::IDENTITY)
            .unwrap();
        assembly
            .add_instance(None, "b", part, Placement3::translate(0.3, 0.0, 0.0))
            .unwrap();
        assembly.bind_material(a, "surface", "blue").unwrap();
        let mut compiler = PartCompiler::new();
        let first = compiler
            .compile_parts(&assembly, &compile_policy())
            .unwrap();
        let count = compiler.counters().parts_compiled;
        assembly.bind_material(a, "surface", "gold").unwrap();
        assembly
            .add_instance(None, "c", part, Placement3::translate(0.6, 0.0, 0.0))
            .unwrap();
        let second = compiler
            .compile_parts(&assembly, &compile_policy())
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, count);
        assert_eq!(count, 1);
        assert!(Rc::ptr_eq(
            first.part(part).unwrap(),
            second.part(part).unwrap()
        ));
        let draw = flatten(&assembly, &second);
        assert_eq!(draw.items.len(), 3);
        assert!(
            draw.items[0]
                .regions
                .iter()
                .all(|r| r.material.as_deref() == Some("gold"))
        );
    }

    #[test]
    fn rounded_profile_keeps_sharp_ends_and_its_section_radius() {
        let eval = evaluate(&rail(true).unwrap(), &compile_policy().evaluation).unwrap();
        let body = &eval.bodies[0].body;
        let mut samples = 0;
        for face in body.mesh.faces() {
            if matches!(
                body.source_map.face_feature(face),
                Some(Feature::CapStart | Feature::CapEnd)
            ) {
                for edge in body.mesh.face_loop(face) {
                    assert_eq!(body.mesh.edge_sharpness(edge), Some(1.0));
                }
            }
        }
        for v in body.mesh.vertices() {
            let p = body.mesh.vertex_position(v).unwrap();
            assert!(
                p[2] == 0.0 || p[2] == 2.0,
                "profile rounding must leave end planes intact"
            );
            let x = f64::from(p[0]);
            let y = f64::from(p[1]);
            let dx = x - x.clamp(0.006, 0.084);
            let dy = y - y.clamp(0.006, 0.194);
            assert!(((dx * dx + dy * dy).sqrt() - 0.006).abs() < 1e-7);
            if dx.abs() > 1e-5 && dy.abs() > 1e-5 {
                samples += 1;
            }
        }
        assert!(samples > 20);
    }
}
