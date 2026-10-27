// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native rounded turns and smooth round-section sweeps, with construction charts.
//! `cargo run --release -p analytic_extrusion --example rounded_geometry -- /tmp/rounded.glb`

use exedra::Mesh;
use exedra::analytic::{
    realize::RealizationPolicy,
    revolve::{FaceRole as TurnFace, Revolution},
    sweep::{FaceRole as SweepFace, Sweep},
};
use exedra::compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource};
use exedra::constructive::ir::Placement3;
use exedra::constructive::{
    builders,
    chart::{ChartTransform, SurfaceChart},
    path::PathCursor3,
};
use exedra::gltf::{
    GltfExportOptions, GltfMesh, GltfNode, GltfScene, MaterialResolver, Texture,
    export_glb_with_materials,
};
use exedra::profile::ProfilePolicy;
use serde_json::{Value, json};
use std::{
    error::Error,
    f64::consts::{PI, TAU},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const PROFILE: ProfilePolicy = ProfilePolicy::new(1e-7);
const REALIZE: RealizationPolicy = RealizationPolicy {
    tolerance: 0.04,
    geometry_gap: 1e-9,
    min_arc_segments: 4,
    max_edge_segments: 4096,
    max_vertices: 1 << 20,
    max_boundary_samples: 1 << 22,
};
const GRID: ChartTransform = ChartTransform {
    matrix: [[0.05, 0.0], [0.0, 0.05]],
    offset: [0.0; 2],
};

fn turn(rounded: bool) -> Result<Mesh> {
    let (profile, sweep) = if rounded {
        (
            builders::rounded_rect_centered(30.0, 38.0, 5.0)?.translated((60.0, 19.0))?,
            TAU,
        )
    } else {
        (builders::circle(12.0)?.translated((60.0, 12.0))?, 1.5 * PI)
    };
    let upright =
        Placement3::from_axes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0; 3]);
    let solid = Revolution::from_profile(profile, upright, sweep, &PROFILE)?;
    let chart = SurfaceChart::Revolve {
        wall: GRID,
        caps: GRID,
        reference_radius: 60.0,
    };
    let output = solid.realize_with_uvs(
        &REALIZE,
        |_, face| u32::from(matches!(face.role, TurnFace::Cap { .. })),
        |role, p| match role {
            TurnFace::Wall { .. } => chart.map_revolve_wall_triangle(p),
            TurnFace::Cap { .. } => chart.map_cap_triangle(p),
        },
    )?;
    assert!(
        output.report.within_tolerance(),
        "rounded turn accuracy: {:?}",
        output.report
    );
    println!(
        "turn: {} native faces, volume {:.6}, area {:.6}, {} triangles, deviation {:.6}",
        solid.body().faces().len(),
        solid.volume(),
        solid.surface_area(),
        output.mesh.faces().count(),
        output.report.max_deviation
    );
    Ok(output.mesh)
}

fn rail(hollow: bool) -> Result<Mesh> {
    let profile = builders::circle(10.0)?;
    let profile = if hollow {
        profile.with_cutouts_at(&builders::circle(6.0)?, [(0.0, 0.0)])?
    } else {
        profile
    };
    let path = PathCursor3::new([0.0; 3], [0.0, 1.0, 0.0])
        .tangent_arc(40.0, PI, [0.0, 0.0, 1.0])
        .tangent_arc(40.0, -0.5 * PI, [0.0, 0.0, 1.0])
        .line_by([-40.0, 0.0, 0.0])
        .finish([1.0, 0.0, 0.0])?;
    let solid = Sweep::from_profile(
        profile,
        &path,
        Placement3::translate(0.0, 0.0, 10.0),
        &PROFILE,
    )?;
    let chart = SurfaceChart::Sweep {
        wall: GRID,
        caps: GRID,
    };
    let output = solid.realize_with_uvs(
        &REALIZE,
        |_, face| u32::from(matches!(face.role, SweepFace::Cap { .. })),
        |role, p| match role {
            SweepFace::Wall { .. } => chart.map_sweep_wall_triangle(p),
            SweepFace::Cap { .. } => chart.map_cap_triangle(p),
        },
    )?;
    assert!(
        output.report.within_tolerance(),
        "rounded sweep accuracy: {:?}",
        output.report
    );
    println!(
        "rail: {} native faces, volume {:.6}, area {:.6}, {} triangles, deviation {:.6}",
        solid.body().faces().len(),
        solid.volume(),
        solid.surface_area(),
        output.mesh.faces().count(),
        output.report.max_deviation
    );
    Ok(output.mesh)
}

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rounded.glb".to_owned());
    let names = [
        "rounded native turn",
        "toroidal cutaway",
        "round native sweep",
        "hollow native sweep",
    ];
    let sources = [turn(true)?, turn(false)?, rail(false)?, rail(true)?].map(PartSource::Baked);
    for (name, source) in names.iter().zip(&sources) {
        let PartSource::Baked(mesh) = source else {
            unreachable!()
        };
        assert!(mesh.validate_deep().is_empty(), "{name}: deeply valid mesh");
        assert!(mesh.boundary_loops()?.is_empty(), "{name}: closed mesh");
    }
    let inputs: Vec<_> = names
        .iter()
        .zip(&sources)
        .map(|(name, source)| PartInput::new(name, source))
        .collect();
    let compiled = PartCompiler::new().compile_parts(&inputs, &CompilePolicy::default())?;
    let mut root = GltfNode::new("native rounded geometry");
    root.transform = Placement3::from_axes(
        [0.001, 0.0, 0.0],
        [0.0, 0.001, 0.0],
        [0.0, 0.0, 0.001],
        [0.0; 3],
    );
    let mut nodes = vec![root];
    let positions = [
        [-100.0, 100.0],
        [100.0, 100.0],
        [-20.0, -80.0],
        [180.0, -80.0],
    ];
    for (i, name) in names.iter().enumerate() {
        let id = PartId(u32::try_from(i)?);
        let mut mesh = GltfMesh::new(id, *name, &compiled)?;
        mesh.materials = compiled
            .part(id)
            .expect("compiled rounded part")
            .bodies
            .iter()
            .map(|b| {
                b.regions
                    .iter()
                    .map(|r| {
                        Some(if r.region == 1 {
                            "cut".to_owned()
                        } else {
                            format!("part-{i}")
                        })
                    })
                    .collect()
            })
            .collect();
        let mut node = GltfNode::new(*name);
        node.parent = Some(0);
        node.transform = Placement3::translate(positions[i][0], positions[i][1], 0.0);
        node.geometry = Some(mesh.into());
        nodes.push(node);
    }
    let output = export_glb_with_materials(
        &GltfScene {
            nodes,
            ..GltfScene::default()
        },
        &compiled,
        &Materials,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    std::fs::write(&path, output.bytes)?;
    println!("Wrote {path}");
    Ok(())
}

struct Materials;
impl MaterialResolver for Materials {
    fn resolve(&self, key: &str) -> Option<Value> {
        let color = match key {
            "part-0" => [0.18, 0.42, 0.66, 1.0],
            "part-1" => [0.75, 0.59, 0.29, 1.0],
            "part-2" => [0.12, 0.52, 0.42, 1.0],
            "part-3" => [0.55, 0.25, 0.15, 1.0],
            "cut" => [0.65, 0.3, 0.1, 1.0],
            _ => return None,
        };
        let mut material = json!({"name":key,"pbrMetallicRoughness":{"baseColorFactor":color,"metallicFactor":0.35,"roughnessFactor":0.32}});
        if key != "cut" {
            material["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index":0});
        }
        Some(material)
    }
    fn resolve_texture(&self, index: u32) -> Option<Texture<'_>> {
        (index == 0).then_some(Texture {
            image: include_bytes!("../../material_gallery/assets/unit-grid.png"),
            mime_type: "image/png",
            sampler: Some(json!({"wrapS":10497,"wrapT":10497,"magFilter":9729,"minFilter":9987})),
        })
    }
}
