// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Panel UVs survive constructive edge finishing and GLB export.

use std::error::Error;
use std::path::PathBuf;

use exedra_assembly::{Assembly, CompilePolicy, NormalsSource, PartCompiler, flatten};
use exedra_constructive::edge_finish::{EdgeSelection, RoundPolicy};
use exedra_constructive::ir::{
    BoxUv, NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder, UvMapping, UvProjection,
    UvTransform,
};
use exedra_constructive::tessellate::{EvalPolicy, tessellate_primitive};
use exedra_gltf::{GltfExportOptions, export_glb_with_materials};
use exedra_mesh::Mesh;
use serde_json::json;

fn panel_mesh() -> Result<Mesh, Box<dyn Error>> {
    // A loader can supply this same untextured mesh; mapping belongs to the
    // recipe below, independent of how geometry was obtained.
    Ok(tessellate_primitive(
        PrimitiveSpec::Box {
            size: [0.45, 0.024, 0.7],
        },
        &Placement3::IDENTITY,
        &EvalPolicy::default(),
    )?
    .mesh)
}

fn panel(policy: Option<RoundPolicy>) -> Result<Recipe, Box<dyn Error>> {
    let mut builder = RecipeBuilder::new();
    let surface = builder.material_slot("surface");
    let import = builder.add_import(panel_mesh()?)?;
    let child = builder.with_material(surface).add(NodeKind::MeshImport {
        import,
        placement: Placement3::IDENTITY,
    })?;
    let child = builder.add(NodeKind::UvMap {
        options: exedra_mesh::uv_mapping::UvMappingOptions::default(),
        child,
        mapping: UvMapping {
            projection: UvProjection::Box(BoxUv {
                // Grain follows Z on upright faces, with signed opposite sides.
                swap_axes: [true, false, false],
                flip_v: [true, true, false],
                ..BoxUv::default()
            }),
            transform: UvTransform::rotate_offset_scale(0.0, [0.0; 2], [1.0 / 0.08; 2])?,
        },
    })?;
    let root = if let Some(policy) = policy {
        builder.add(NodeKind::EdgeFinish {
            child,
            selection: EdgeSelection::SharpEdges,
            policy,
        })?
    } else {
        child
    };
    Ok(builder.finish(root)?)
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/textured-finishes"));
    std::fs::create_dir_all(&output)?;
    let mut fillet = RoundPolicy::fillet(0.004);
    fillet.chord_tolerance = 0.0001;
    for (name, finish) in [
        ("panel-sharp", None),
        ("panel-chamfer", Some(RoundPolicy::chamfer(0.004))),
        ("panel-fillet", Some(fillet)),
    ] {
        let mut assembly = Assembly::new();
        let part = assembly.add_recipe_part(name, panel(finish)?)?;
        assembly.set_part_material(part, "surface", "panel.surface")?;
        assembly.add_instance(None, name, part, Placement3::IDENTITY)?;
        let mut compiler = PartCompiler::new();
        let compiled = compiler.compile_parts(
            &assembly,
            &CompilePolicy {
                normals: NormalsSource::CustomOrDerived,
                ..CompilePolicy::default()
            },
        )?;
        let draw = flatten(&assembly, &compiled);
        let glb = export_glb_with_materials(
            &assembly,
            &compiled,
            &|_: &str| {
                Some(json!({"pbrMetallicRoughness": {
                    "baseColorFactor": [0.6, 0.35, 0.15, 1.0],
                    "metallicFactor": 0.0, "roughnessFactor": 0.4
                }}))
            },
            GltfExportOptions::z_up_to_y_up(),
        )?;
        std::fs::write(output.join(format!("{name}.glb")), glb.bytes)?;
        println!("{name}: {} triangles", draw.triangle_count());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_constructive::evaluate::evaluate;
    use exedra_mesh::{ExtractParams, FaceTriangulation, attr};

    #[test]
    fn textured_panel_has_no_missing_or_collapsed_uv_triangles() {
        for policy in [RoundPolicy::chamfer(0.004), RoundPolicy::fillet(0.004)] {
            let evaluated =
                evaluate(&panel(Some(policy)).unwrap(), &EvalPolicy::default()).unwrap();
            assert_eq!(evaluated.report.counters.edge_finish_passes, 1);
            let mesh = &evaluated.bodies[0].body.mesh;
            for corner in mesh.faces().flat_map(|f| mesh.face_loop(f)) {
                assert!(
                    mesh.attrs()
                        .sparse(attr::CORNER_UV)
                        .unwrap()
                        .get(corner.as_id())
                        .is_some()
                );
            }
            let (triangles, _) = mesh.to_trimesh(&ExtractParams {
                face_triangulation: FaceTriangulation::Robust,
                ..ExtractParams::default()
            });
            for triangle in triangles.indices.as_chunks::<3>().0.iter() {
                let [a, b, c] = [triangle[0], triangle[1], triangle[2]]
                    .map(|i| triangles.uvs[i as usize].map(f64::from));
                let twice_area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                assert!(
                    twice_area.abs() > 1e-12,
                    "collapsed UV triangle: {a:?} {b:?} {c:?}"
                );
            }
        }
    }
}
