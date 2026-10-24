// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Eight imported-mesh mapping studies, one embedded texture and checked exports.

use std::error::Error;
use std::f64::consts::{PI, TAU};
use std::path::PathBuf;

use exedra_assembly::{Assembly, CompilePolicy, PartCompiler};
use exedra_constructive::ir::{
    BoxUv, NodeKind, PerspectiveUv, Placement3, PrimitiveSpec, Recipe, RecipeBuilder, UvFrame,
    UvMapping, UvMappingOptions, UvProjection, UvSet, UvTransform,
};
use exedra_constructive::tessellate::{EvalPolicy, tessellate_primitive};
use exedra_gltf::{
    GltfAttribute, GltfExportOptions, MaterialResolver, Texture, export_glb_with_materials,
};
use exedra_mesh::{ExtractAttribute, Mesh, MeshBuilder, attr};
use serde_json::{Value, json};

struct Grid;
impl MaterialResolver for Grid {
    fn resolve(&self, key: &str) -> Option<Value> {
        let set = if key == "grid" {
            0
        } else if key == "grid-secondary" {
            1
        } else {
            return None;
        };
        Some(
            json!({"pbrMetallicRoughness": {"baseColorTexture": {"index": 0,"texCoord":set},"roughnessFactor":0.7,"metallicFactor":0.0},"doubleSided":true}),
        )
    }
    fn resolve_texture(&self, index: u32) -> Option<Texture<'_>> {
        (index == 0).then_some(Texture {
            image: include_bytes!("../../assets/projection-grid.png"),
            mime_type: "image/png",
            sampler: Some(json!({"wrapS":10497,"wrapT":10497,"magFilter":9729,"minFilter":9987})),
        })
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "small checked sample coordinates narrow to mesh storage"
)]
fn point(x: f64, y: f64, z: f64) -> [f32; 3] {
    [x as f32, y as f32, z as f32]
}

fn surface(
    rows: u32,
    cols: u32,
    wrap_rows: bool,
    wrap_cols: bool,
    position: impl Fn(f64, f64) -> [f32; 3],
) -> Mesh {
    let mut builder = MeshBuilder::new();
    for row in 0..rows {
        for col in 0..cols {
            builder.push_vertex(position(
                f64::from(col) / f64::from(if wrap_cols { cols } else { cols - 1 }),
                f64::from(row) / f64::from(if wrap_rows { rows } else { rows - 1 }),
            ));
        }
    }
    for row in 0..if wrap_rows { rows } else { rows - 1 } {
        for col in 0..if wrap_cols { cols } else { cols - 1 } {
            let next_row = (row + 1) % rows;
            let next_col = (col + 1) % cols;
            builder
                .add_face(&[
                    row * cols + col,
                    row * cols + next_col,
                    next_row * cols + next_col,
                    next_row * cols + col,
                ])
                .unwrap();
        }
    }
    builder.build().unwrap().mesh
}
fn sphere() -> Mesh {
    let cols = 64_u32;
    let rings = 31_u32;
    let mut builder = MeshBuilder::new();
    builder.push_vertex([0.0, 0.0, 0.05]);
    for ring in 0..rings {
        for col in 0..cols {
            let latitude = -PI * 0.5 + PI * f64::from(ring + 1) / f64::from(rings + 1);
            let theta = TAU * f64::from(col) / f64::from(cols);
            builder.push_vertex(point(
                latitude.cos() * theta.cos(),
                latitude.cos() * theta.sin(),
                1.05 + latitude.sin(),
            ));
        }
    }
    let north = builder.push_vertex([0.0, 0.0, 2.05]);
    for col in 0..cols {
        let next = (col + 1) % cols;
        builder.add_face(&[0, 1 + next, 1 + col]).unwrap();
        for ring in 0..rings - 1 {
            builder
                .add_face(&[
                    1 + ring * cols + col,
                    1 + ring * cols + next,
                    1 + (ring + 1) * cols + next,
                    1 + (ring + 1) * cols + col,
                ])
                .unwrap();
        }
        builder
            .add_face(&[
                1 + (rings - 1) * cols + col,
                1 + (rings - 1) * cols + next,
                north,
            ])
            .unwrap();
    }
    builder.build().unwrap().mesh
}
fn panel() -> Mesh {
    surface(25, 25, false, false, |u, v| {
        let x = (u - 0.5) * 2.1;
        let y = (v - 0.5) * 2.1;
        point(x, y, 0.45 + 0.22 * (x * 2.0).sin() * (y * 2.0).cos())
    })
}
fn block() -> Mesh {
    tessellate_primitive(
        PrimitiveSpec::Box {
            size: [1.7, 1.7, 1.7],
        },
        &Placement3::translate(-0.85, -0.85, 0.05),
        &EvalPolicy::default(),
    )
    .unwrap()
    .mesh
}
fn frame(origin: [f64; 3]) -> UvFrame {
    UvFrame {
        origin,
        ..UvFrame::XY
    }
}

fn recipe(
    mesh: Mesh,
    projection: UvProjection,
    transform: UvTransform,
    secondary: bool,
) -> Result<Recipe, Box<dyn Error>> {
    let mut builder = RecipeBuilder::new();
    let slot = builder.material_slot("surface");
    let import = builder.add_import(mesh)?;
    let child = builder.with_material(slot).add(NodeKind::MeshImport {
        import,
        placement: Placement3::IDENTITY,
    })?;
    let mut root = builder.map_uv(
        child,
        UvMapping {
            projection,
            transform,
        },
    )?;
    if secondary {
        let transform = UvTransform::rotation(0.45)?
            .around_pivot([0.5; 2])?
            .then(UvTransform {
                matrix: [[1.0, 0.35], [0.0, 1.0]],
                offset: [0.0; 2],
            })?;
        root = builder.add(NodeKind::UvMap {
            child: root,
            mapping: UvMapping {
                projection: UvProjection::Existing,
                transform,
            },
            options: UvMappingOptions {
                target: UvSet::Secondary,
                ..UvMappingOptions::default()
            },
        })?;
    }
    Ok(builder.finish(root)?)
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/uv-projections"));
    std::fs::create_dir_all(&output)?;
    let cases = [
        (
            "Planar / fitted",
            panel(),
            UvProjection::Planar(UvFrame::XY),
            UvTransform::fit_bounds([-1.05; 2], [1.05; 2], true)?,
            false,
        ),
        (
            "Box / face axes",
            block(),
            UvProjection::Box(BoxUv {
                swap_axes: [true, false, true],
                ..BoxUv::default()
            }),
            UvTransform::scale([0.7; 2])?,
            false,
        ),
        (
            "Per-face planes",
            block(),
            UvProjection::Face,
            UvTransform::scale([0.7; 2])?,
            false,
        ),
        (
            "Cylinder / seam",
            surface(17, 64, false, true, |u, v| {
                point(
                    0.85 * (u * TAU).cos(),
                    0.85 * (u * TAU).sin(),
                    0.05 + v * 1.9,
                )
            }),
            UvProjection::Cylindrical(UvFrame::XY),
            UvTransform::scale([2.0, 0.6])?,
            false,
        ),
        (
            "Sphere / poles",
            sphere(),
            UvProjection::Spherical(frame([0.0, 0.0, 1.05])),
            UvTransform::scale([2.0, 1.0])?,
            false,
        ),
        (
            "Torus / two seams",
            surface(32, 80, true, true, |u, v| {
                let radius = 0.85 + 0.3 * (v * TAU).cos();
                point(
                    radius * (u * TAU).cos(),
                    radius * (u * TAU).sin(),
                    0.5 + 0.3 * (v * TAU).sin(),
                )
            }),
            UvProjection::Toroidal {
                frame: frame([0.0, 0.0, 0.5]),
                major_radius: 0.85,
            },
            UvTransform::scale([2.0, 1.0])?,
            false,
        ),
        (
            "Perspective",
            panel(),
            UvProjection::Perspective(PerspectiveUv {
                frame: frame([0.0, 0.0, -2.0]),
                focal: [1.2; 2],
                near: 0.1,
            }),
            UvTransform::IDENTITY,
            false,
        ),
        (
            "Pivot + shear / UV1",
            block(),
            UvProjection::Box(BoxUv::default()),
            UvTransform::scale([0.7; 2])?,
            true,
        ),
    ];
    let mut assembly = Assembly::new();
    let mut cards = Vec::new();
    for (index, (label, mesh, projection, transform, secondary)) in cases.into_iter().enumerate() {
        let name = format!("projection-{index}");
        let part =
            assembly.add_recipe_part(&name, recipe(mesh, projection, transform, secondary)?)?;
        assembly.set_part_material(
            part,
            "surface",
            if secondary { "grid-secondary" } else { "grid" },
        )?;
        let x = f64::from(u32::try_from(index % 4)?) * 4.5;
        let y = f64::from(u32::try_from(index / 4)?) * 4.8;
        assembly.add_instance(None, &name, part, Placement3::translate(x, y, 0.0))?;
        cards.push((part,json!({"label":label,"name":name,"position":[x,y,0.0],"uv_set":if secondary {1}else{0}})));
    }
    let policy = CompilePolicy {
        attributes: vec![ExtractAttribute::new(attr::CORNER_UV1, [0.0; 2])],
        ..CompilePolicy::default()
    };
    let mut compiler = PartCompiler::new();
    let compiled = compiler.compile_parts(&assembly, &policy)?;
    for (part, card) in &mut cards {
        let report = compiled.report(*part).expect("recipe report");
        card["mapping"] = json!({"passes":report.counters.uv_mapping_passes,"corners_written":report.counters.uv_corners_written,"seam_corners":report.counters.uv_seam_corners_adjusted,"pole_corners":report.counters.uv_pole_corners_repaired});
    }
    let mappings = [GltfAttribute::tex_coord(attr::CORNER_UV1, 1)];
    let glb = export_glb_with_materials(
        &assembly,
        &compiled,
        &Grid,
        GltfExportOptions::z_up_to_y_up().with_attributes(&mappings),
    )?;
    std::fs::write(output.join("uv-projections.glb"), glb.bytes)?;
    let cards: Vec<_> = cards.into_iter().map(|(_, card)| card).collect();
    std::fs::write(
        output.join("gallery.json"),
        serde_json::to_vec_pretty(&json!({"cards":cards}))?,
    )?;
    println!(
        "{}: eight projection studies, embedded grid, primary and secondary UVs",
        output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_constructive::evaluate::evaluate;
    use exedra_gltf::GlbDocument;

    #[test]
    fn sphere_sample_has_complete_uvs_and_expected_seam_and_pole_evidence() {
        let recipe = recipe(
            sphere(),
            UvProjection::Spherical(frame([0.0, 0.0, 1.05])),
            UvTransform::scale([2.0, 1.0]).unwrap(),
            false,
        )
        .unwrap();
        let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
        assert_eq!(result.report.counters.uv_pole_corners_repaired, 128);
        assert_eq!(result.report.counters.uv_seam_corners_adjusted, 62);
        let body = &result.bodies[0].body;
        body.source_map.check(&body.mesh).unwrap();
        assert!(body.mesh.validate_deep().is_empty());
        for face in body.mesh.faces() {
            for corner in body.mesh.face_loop(face) {
                let uv = body
                    .mesh
                    .attrs()
                    .sparse(attr::CORNER_UV)
                    .unwrap()
                    .get(corner.into())
                    .unwrap();
                assert!(uv.iter().all(|x| x.is_finite()));
            }
        }
    }

    #[test]
    fn secondary_sample_exports_an_embedded_texture_and_reuses_compilation() {
        let mut assembly = Assembly::new();
        let part = assembly
            .add_recipe_part(
                "secondary",
                recipe(
                    block(),
                    UvProjection::Box(BoxUv::default()),
                    UvTransform::IDENTITY,
                    true,
                )
                .unwrap(),
            )
            .unwrap();
        assembly
            .set_part_material(part, "surface", "grid-secondary")
            .unwrap();
        assembly
            .add_instance(None, "secondary", part, Placement3::IDENTITY)
            .unwrap();
        let policy = CompilePolicy {
            attributes: vec![ExtractAttribute::new(attr::CORNER_UV1, [0.0; 2])],
            ..CompilePolicy::default()
        };
        let mut compiler = PartCompiler::new();
        let compiled = compiler.compile_parts(&assembly, &policy).unwrap();
        let baseline = compiler.counters().parts_compiled;
        let _ = compiler.compile_parts(&assembly, &policy).unwrap();
        assert_eq!(compiler.counters().parts_compiled, baseline);
        let attributes = [GltfAttribute::tex_coord(attr::CORNER_UV1, 1)];
        let export = export_glb_with_materials(
            &assembly,
            &compiled,
            &Grid,
            GltfExportOptions::z_up_to_y_up().with_attributes(&attributes),
        )
        .unwrap();
        let document = GlbDocument::parse(&export.bytes).unwrap();
        assert_eq!(document.json()["images"].as_array().unwrap().len(), 1);
        for mesh in document.json()["meshes"].as_array().unwrap() {
            for primitive in mesh["primitives"].as_array().unwrap() {
                assert!(primitive["attributes"]["TEXCOORD_1"].is_number());
            }
        }
    }
}
