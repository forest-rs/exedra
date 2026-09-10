// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Cabinet-door UVs survive constructive edge finishing and GLB export.

use std::error::Error;
use std::path::PathBuf;

use exedra_assembly::{Assembly, CompilePolicy, NormalsSource, PartCompiler, flatten};
use exedra_constructive::edge_finish::{EdgeSelection, RoundPolicy};
use exedra_constructive::ir::{NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder};
use exedra_constructive::tessellate::{EvalPolicy, tessellate_primitive};
use exedra_gltf::{GltfExportOptions, export_glb_with_materials};
use exedra_mesh::{Mesh, op::set_corner_uv};
use serde_json::json;

fn door_mesh() -> Result<Mesh, Box<dyn Error>> {
    let mut mesh = tessellate_primitive(
        PrimitiveSpec::Box {
            size: [0.45, 0.024, 0.7],
        },
        &Placement3::IDENTITY,
        &EvalPolicy::default(),
    )?
    .mesh;
    // A caller-authored box mapping: one repeat per 80 mm, grain along Z on
    // the upright faces. UVs belong to each half-edge's destination vertex.
    let mut values = Vec::new();
    for face in mesh.faces() {
        let corners: Vec<_> = mesh.face_loop(face).collect();
        let positions: Vec<_> = corners
            .iter()
            .map(|&corner| {
                *mesh
                    .vertex_position(mesh.to_vertex(corner).expect("corner"))
                    .expect("position")
            })
            .collect();
        let fixed_axis = (0..3)
            .find(|&axis| positions.iter().all(|p| p[axis] == positions[0][axis]))
            .expect("box face");
        for (corner, p) in corners.into_iter().zip(positions) {
            let uv = match fixed_axis {
                0 => [p[1], p[2]],
                1 => [p[0], p[2]],
                _ => [p[0], p[1]],
            };
            values.push((corner, uv.map(|v| v / 0.08)));
        }
    }
    let mut edit = mesh.edit();
    for (corner, uv) in values {
        set_corner_uv(&mut edit, corner, uv)?;
    }
    let _: () = edit.finish();
    Ok(mesh)
}

fn door(policy: Option<RoundPolicy>) -> Result<Recipe, Box<dyn Error>> {
    let mut builder = RecipeBuilder::new();
    let surface = builder.material_slot("surface");
    let import = builder.add_import(door_mesh()?)?;
    let child = builder.with_material(surface).add(NodeKind::MeshImport {
        import,
        placement: Placement3::IDENTITY,
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
        ("door-sharp", None),
        ("door-chamfer", Some(RoundPolicy::chamfer(0.004))),
        ("door-fillet", Some(fillet)),
    ] {
        let mut assembly = Assembly::new();
        let part = assembly.add_recipe_part(name, door(finish)?)?;
        assembly.set_part_material(part, "surface", "door.surface")?;
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
            &draw,
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
    fn textured_door_has_no_missing_or_collapsed_uv_triangles() {
        for policy in [RoundPolicy::chamfer(0.004), RoundPolicy::fillet(0.004)] {
            let evaluated = evaluate(&door(Some(policy)).unwrap(), &EvalPolicy::default()).unwrap();
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
            for triangle in triangles.indices.chunks_exact(3) {
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
