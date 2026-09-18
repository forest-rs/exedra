// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One-meter rest grids on a curved vault and turned vessels, including a mirror.
//! `cargo run -p material_gallery --bin surface_charts -- target/surface-charts.glb`

use exedra_assembly::{Assembly, CompilePolicy, PartCompiler};
use exedra_constructive::{
    chart::{ChartTransform, SurfaceChart},
    ir::{CapMode, NodeKind, Placement3, Recipe, RecipeBuilder},
    profile::{Loop2, Profile2, Seg2},
};
use exedra_gltf::{GltfExportOptions, MaterialResolver, Texture, export_glb_with_materials};
use serde_json::{Value, json};
use std::path::PathBuf;

struct Grid {
    tint: [f64; 4],
}
impl MaterialResolver for Grid {
    fn resolve(&self, key: &str) -> Option<Value> {
        (key == "unit-grid").then(|| {
            json!({
                "pbrMetallicRoughness": {
                    "baseColorFactor": self.tint,
                    "baseColorTexture": { "index": 0 },
                    "metallicFactor": 0.0, "roughnessFactor": 0.55
                }
            })
        })
    }
    fn resolve_texture(&self, index: u32) -> Option<Texture<'_>> {
        (index == 0).then_some(Texture {
            // Procedural one-repeat grid; dark lines are one recipe unit apart.
            image: include_bytes!("../../assets/unit-grid.png"),
            mime_type: "image/png",
            sampler: Some(
                json!({ "wrapS": 10497, "wrapT": 10497, "magFilter": 9729, "minFilter": 9987 }),
            ),
        })
    }
}

fn vault() -> Recipe {
    // Half-annulus with no overlapping support solids; both curved walls share
    // one continuous perimeter-distance chart through every sampled arc chord.
    let p = Profile2::simple(
        Loop2::new(vec![
            Seg2::arc((-3.0, 0.0), 1.0),
            Seg2::line((-2.75, 0.0)),
            Seg2::arc((2.75, 0.0), -1.0),
            Seg2::line((3.0, 0.0)),
        ])
        .unwrap(),
    )
    .unwrap();
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(p);
    let slot = b.material_slot("surface");
    let root = b
        .with_material(slot)
        .with_surface_chart(SurfaceChart::Extrude {
            wall: ChartTransform::IDENTITY,
            caps: ChartTransform::IDENTITY,
        })
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::euler_extrinsic_xyz_then_translate(
                std::f64::consts::FRAC_PI_2,
                0.0,
                0.0,
                [0.0, 4.5, 0.0],
            ),
            height: 4.5,
            caps: CapMode::Both,
        })
        .unwrap();
    b.finish(root).unwrap()
}
fn vessel() -> Recipe {
    let p = Profile2::simple(
        Loop2::new(vec![
            Seg2::line((0.7, 0.0)),
            Seg2::cubic((1.2, 2.0), (1.4, 0.3), (1.5, 1.3)),
            Seg2::cubic((0.55, 3.0), (1.0, 2.7), (0.55, 2.5)),
            Seg2::line((0.55, 3.4)),
            Seg2::line((0.42, 3.4)),
            Seg2::line((0.42, 2.95)),
            Seg2::cubic((1.05, 1.95), (0.45, 2.55), (0.9, 2.65)),
            Seg2::cubic((0.62, 0.15), (1.32, 1.3), (1.25, 0.4)),
            Seg2::line((0.0, 0.15)),
            Seg2::line((0.0, 0.0)),
        ])
        .unwrap(),
    )
    .unwrap();
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(p);
    let slot = b.material_slot("surface");
    let root = b
        .with_material(slot)
        .with_surface_chart(SurfaceChart::Revolve {
            reference_radius: 1.0,
            wall: ChartTransform::IDENTITY,
            caps: ChartTransform::IDENTITY,
        })
        .add(NodeKind::Revolve {
            profile,
            placement: Placement3::euler_extrinsic_xyz_then_translate(
                std::f64::consts::FRAC_PI_2,
                0.0,
                0.0,
                [0.0; 3],
            ),
            sweep: std::f64::consts::TAU,
            caps: CapMode::Both,
        })
        .unwrap();
    b.finish(root).unwrap()
}
fn scene() -> Assembly {
    let mut assembly = Assembly::new();
    let arch = assembly.add_recipe_part("vault", vault()).unwrap();
    let vase = assembly.add_recipe_part("vessel", vessel()).unwrap();
    for part in [arch, vase] {
        assembly
            .set_part_material(part, "surface", "unit-grid")
            .unwrap();
    }
    assembly
        .add_instance(None, "vault", arch, Placement3::IDENTITY)
        .unwrap();
    assembly
        .add_instance(None, "vessel", vase, Placement3::translate(5.0, 1.0, 0.0))
        .unwrap();
    assembly
        .add_instance(
            None,
            "reflected-vessel",
            vase,
            Placement3 {
                rows: [
                    [-1.0, 0.0, 0.0, 8.0],
                    [0.0, 1.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
            },
        )
        .unwrap();
    assembly
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "target/surface-charts.glb".into());
    let assembly = scene();
    let mut compiler = PartCompiler::new();
    let policy = CompilePolicy::default();
    let compiled = compiler.compile_parts(&assembly, &policy)?;
    for part in compiled.parts() {
        for body in &part.bodies {
            assert!(
                body.regions.iter().all(|r| r.has_uvs),
                "every charted region must retain complete UV coverage"
            );
        }
    }
    let baseline = compiler.counters();
    let mut grid = Grid { tint: [1.0; 4] };
    let options = GltfExportOptions::z_up_to_y_up();
    let glb = export_glb_with_materials(&assembly, &compiled, &grid, options)?;
    grid.tint = [0.8, 0.9, 1.0, 1.0];
    let _edited = export_glb_with_materials(&assembly, &compiled, &grid, options)?;
    compiler.compile_parts(&assembly, &policy)?;
    assert_eq!(
        compiler.counters().parts_compiled,
        baseline.parts_compiled,
        "material edits must reuse compiled parts"
    );
    assert_eq!(
        compiler.counters().triangles_emitted,
        baseline.triangles_emitted,
        "material edits must emit no new triangles"
    );
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, glb.bytes)?;
    println!(
        "{}: two compiled parts, three occurrences, one-meter rest grid, material edit reused geometry",
        output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_gltf::GlbDocument;

    #[test]
    fn construction_uvs_reach_textured_glb_and_material_edits_reuse_geometry() {
        let assembly = scene();
        let mut compiler = PartCompiler::new();
        let policy = CompilePolicy::default();
        let compiled = compiler.compile_parts(&assembly, &policy).unwrap();
        for part in compiled.parts() {
            for body in &part.bodies {
                assert!(
                    body.regions.iter().all(|r| r.has_uvs),
                    "every charted region must retain complete UV coverage"
                );
                assert!(body.tri.uvs.iter().flatten().all(|x| x.is_finite()));
                assert!(body.tri.uvs.iter().any(|uv| uv[0] > 1.0 && uv[1] > 1.0));
            }
        }
        let baseline = compiler.counters();
        let mut grid = Grid { tint: [1.0; 4] };
        let options = GltfExportOptions::z_up_to_y_up();
        let first = export_glb_with_materials(&assembly, &compiled, &grid, options).unwrap();
        grid.tint = [0.5, 0.8, 0.9, 1.0];
        let edited = export_glb_with_materials(&assembly, &compiled, &grid, options).unwrap();
        assert_ne!(first.bytes, edited.bytes);
        compiler.compile_parts(&assembly, &policy).unwrap();
        assert_eq!(
            compiler.counters().parts_compiled,
            baseline.parts_compiled,
            "material edits must reuse compiled parts"
        );
        assert_eq!(
            compiler.counters().triangles_emitted,
            baseline.triangles_emitted,
            "material edits must emit no new triangles"
        );
        let doc = GlbDocument::parse(&first.bytes).unwrap();
        assert_eq!(
            doc.bin(),
            GlbDocument::parse(&edited.bytes).unwrap().bin(),
            "material edits must preserve geometry, UV and image bytes"
        );
        let json = doc.json();
        assert_eq!(json["images"].as_array().unwrap().len(), 1);
        for mesh in json["meshes"].as_array().unwrap() {
            for primitive in mesh["primitives"].as_array().unwrap() {
                assert!(primitive["attributes"]["TEXCOORD_0"].is_number());
                assert!(primitive["attributes"]["NORMAL"].is_number());
            }
        }
        let reflected = json["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["name"] == "reflected-vessel")
            .unwrap();
        assert!(reflected["mesh"].is_number());
    }

    #[test]
    fn evaluation_snapshot_retains_chart_metric_after_cache_eviction() {
        let assembly = scene();
        let part = assembly.part_by_key("vessel").unwrap();
        let mut compiler = PartCompiler::new();
        let snapshot = compiler
            .compile_snapshot(&assembly, &CompilePolicy::default())
            .unwrap();
        let body = snapshot.body(part, 0).unwrap();
        let triangles = snapshot.compiled().part(part).unwrap().triangle_count();
        compiler.clear_cache();
        drop(snapshot);
        let geometry = body.geometry();
        geometry.source_map.check(&geometry.mesh).unwrap();
        assert!(triangles > 0);
        for face in geometry.mesh.faces() {
            let chart = geometry.source_map.chart_sampling(face).unwrap();
            assert!(matches!(
                chart.chart,
                SurfaceChart::Revolve {
                    reference_radius: 1.0,
                    ..
                }
            ));
            assert_eq!(chart.loop_lengths.len(), 1);
            assert!(geometry.mesh.face_loop(face).all(|corner| {
                geometry
                    .mesh
                    .attrs()
                    .sparse(exedra_mesh::attr::CORNER_UV)
                    .unwrap()
                    .get(corner.as_id())
                    .is_some()
            }));
        }
    }
}
