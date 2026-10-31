// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reusable profiles, mirrored outlines and radius/tangent paths through the public API.
//! `cargo run --release -p analytic_extrusion --example authoring -- /tmp/authoring.glb`

use exedra::Mesh;
use exedra::analytic::extrusion::{
    Extrusion, ProfileExtrusionError, TessellationPolicy, Workplane,
};
use exedra::compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource};
use exedra::constructive::builders;
use exedra::constructive::chart::{ChartTransform, SurfaceChart};
use exedra::constructive::ir::Placement3;
use exedra::constructive::path::PathCursor3;
use exedra::constructive::profile::{LineArcError, LoopCursor, MirrorLine, Profile2, Seg2};
use exedra::curves::profile::{ProfilePolicy, SegTag};
use exedra::gltf::{
    GltfExportOptions, GltfMesh, GltfNode, GltfScene, MaterialResolver, Texture,
    export_glb_with_materials,
};
use serde_json::{Value, json};
use std::{
    error::Error,
    f64::consts::{FRAC_PI_2, PI},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const POLICY: ProfilePolicy = ProfilePolicy::new(1e-7);
const BORE: SegTag = SegTag(1000);

fn arched_profile() -> Result<Extrusion> {
    let outline = LoopCursor::new((0.0, 0.0))
        .line_to((80.0, 0.0))
        .line_to((80.0, 60.0))
        .arc_through((60.0, 90.0), (0.0, 60.0))
        .close()?;
    Ok(Workplane::XY.extrude_profile(Profile2::simple(outline)?, 10.0, &POLICY)?)
}

fn repeated_bores() -> Result<Extrusion> {
    let bore = builders::circle(10.0)?.tagged(BORE);
    let plate = builders::circle(80.0)?.with_cutouts_at(
        &bore,
        [(60.0, 0.0), (0.0, 60.0), (-60.0, 0.0), (0.0, -60.0)],
    )?;
    let solid = Workplane::XY.extrude_profile(plate, 10.0, &POLICY)?;
    assert_eq!(
        solid.faces_tagged(BORE).count(),
        8,
        "four bores retain their two tagged arc walls"
    );
    Ok(solid)
}

fn mirrored_bracket() -> Result<Extrusion> {
    let outline = LoopCursor::new((0.0, 0.0))
        .line_by((80.0, 0.0))
        .line_by((0.0, 40.0))
        .line_by((-20.0, 0.0))
        .line_by((0.0, -20.0))
        .line_to((0.0, 20.0))
        .close_mirrored(MirrorLine::Vertical(0.0))?;
    Ok(Workplane::XY.extrude_profile(Profile2::simple(outline)?, 20.0, &POLICY)?)
}

fn curved_rail() -> Result<Mesh> {
    let path = PathCursor3::new([0.0; 3], [0.0, 1.0, 0.0])
        .tangent_arc(40.0, PI, [0.0, 0.0, 1.0])
        .tangent_arc(40.0, -FRAC_PI_2, [0.0, 0.0, 1.0])
        .line_by([-40.0, 0.0, 0.0])
        .finish([1.0, 0.0, 0.0])?;
    let sweep = exedra::analytic::sweep::Sweep::from_profile(
        builders::rect_centered(20.0, 20.0)?,
        &path,
        Placement3::IDENTITY,
        &POLICY,
    )?;
    let metric = ChartTransform {
        matrix: [[0.05, 0.0], [0.0, 0.05]],
        offset: [0.0; 2],
    };
    let chart = SurfaceChart::Sweep {
        wall: metric,
        caps: metric,
    };
    let output = sweep.realize_with_uvs(
        &exedra::analytic::realize::RealizationPolicy::default(),
        |_, _| 0,
        |role, points| match role {
            exedra::analytic::sweep::FaceRole::Wall { .. } => chart.map_sweep_wall_triangle(points),
            exedra::analytic::sweep::FaceRole::Cap { .. } => chart.map_cap_triangle(points),
        },
    )?;
    assert!(
        output.report.within_tolerance(),
        "native rail must meet its geometric bound"
    );
    println!(
        "native rail: {} native faces, volume {:.6}, area {:.6}, {} mesh faces",
        sweep.body().faces().len(),
        sweep.volume(),
        sweep.surface_area(),
        output.mesh.faces().count()
    );
    Ok(output.mesh)
}

fn verify_typed_refusal() -> Result<()> {
    let outline = LoopCursor::new((0.0, 0.0))
        .line_to((2.0, 0.0))
        .segment(Seg2::cubic((2.0, 2.0), (3.0, 0.0), (3.0, 2.0)))
        .line_to((0.0, 2.0))
        .close()?;
    match Workplane::XY.extrude_profile(Profile2::simple(outline)?, 2.0, &POLICY) {
        Err(ProfileExtrusionError::Profile(LineArcError::Unsupported { hole: None, seg: 1 })) => {
            println!(
                "Cubic input refused at authored segment 1; no flattening or lost error context"
            );
            Ok(())
        }
        other => Err(format!("expected typed unsupported-curve error, got {other:?}").into()),
    }
}

fn check_mesh(name: &str, mesh: &Mesh) -> Result<()> {
    let issues = mesh.validate_deep();
    assert!(issues.is_empty(), "{name}: {issues:?}");
    assert!(
        mesh.boundary_loops()?.is_empty(),
        "{name}: expected closed mesh"
    );
    Ok(())
}

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "authoring.glb".to_owned());
    let names = [
        "arched profile",
        "reused bore pattern",
        "mirrored outline",
        "radius/tangent sweep",
    ];
    let mut sources = Vec::new();
    for (name, solid) in
        names
            .iter()
            .zip([arched_profile()?, repeated_bores()?, mirrored_bracket()?])
    {
        let realized = solid.tessellate_with_regions(
            &TessellationPolicy {
                chord_tolerance: 0.01,
                ..TessellationPolicy::default()
            },
            |face, _| {
                if solid.face_tag(face) == Some(BORE) {
                    BORE.0
                } else {
                    0
                }
            },
        )?;
        assert!(
            realized.report.within_tolerance(),
            "{name}: tolerance must be met"
        );
        check_mesh(name, &realized.mesh)?;
        println!(
            "{name}: volume {:.6}, area {:.6}, {} mesh faces, deviation {:.6}",
            solid.volume(),
            solid.surface_area(),
            realized.mesh.faces().count(),
            realized.report.max_deviation
        );
        sources.push(PartSource::Baked(realized.mesh));
    }
    let rail = curved_rail()?;
    check_mesh(names[3], &rail)?;
    sources.push(PartSource::Baked(rail));
    verify_typed_refusal()?;
    write_scene(&path, &names, &sources)?;
    println!("Wrote {path}");
    Ok(())
}

fn write_scene(path: &str, names: &[&str], sources: &[PartSource]) -> Result<()> {
    let inputs: Vec<_> = names
        .iter()
        .zip(sources)
        .map(|(name, source)| PartInput::new(name, source))
        .collect();
    let compiled = PartCompiler::new().compile_parts(&inputs, &CompilePolicy::default())?;
    let mut root = GltfNode::new("profile authoring");
    root.transform = Placement3::from_axes(
        [0.001, 0.0, 0.0],
        [0.0, 0.001, 0.0],
        [0.0, 0.0, 0.001],
        [0.0; 3],
    );
    let mut nodes = vec![root];
    let positions = [
        [-140.0, 80.0, 0.0],
        [80.0, 120.0, 0.0],
        [-90.0, -100.0, 0.0],
        [200.0, -70.0, 10.0],
    ];
    for (i, name) in names.iter().enumerate() {
        let id = PartId(u32::try_from(i)?);
        let mut mesh = GltfMesh::new(id, *name, &compiled)?;
        mesh.materials = compiled
            .part(id)
            .expect("compiled part")
            .bodies
            .iter()
            .map(|body| {
                body.regions
                    .iter()
                    .map(|region| {
                        Some(if region.region == BORE.0 {
                            "bore".to_owned()
                        } else {
                            format!("part-{i}")
                        })
                    })
                    .collect()
            })
            .collect();
        let mut node = GltfNode::new(*name);
        node.parent = Some(0);
        node.transform = Placement3::translate(positions[i][0], positions[i][1], positions[i][2]);
        node.geometry = Some(mesh.into());
        nodes.push(node);
    }
    let exported = export_glb_with_materials(
        &GltfScene {
            nodes,
            ..GltfScene::default()
        },
        &compiled,
        &Materials,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    std::fs::write(path, exported.bytes)?;
    Ok(())
}

struct Materials;
impl MaterialResolver for Materials {
    fn resolve(&self, key: &str) -> Option<Value> {
        let (color, metal) = match key {
            "part-0" => ([0.18, 0.42, 0.66, 1.0], 0.25),
            "part-1" => ([0.75, 0.59, 0.29, 1.0], 0.65),
            "part-2" => ([0.64, 0.23, 0.12, 1.0], 0.25),
            "part-3" => ([0.12, 0.52, 0.42, 1.0], 0.3),
            "bore" => ([0.16, 0.2, 0.24, 1.0], 0.85),
            _ => return None,
        };
        let mut material = json!({"name":key,"pbrMetallicRoughness":{
            "baseColorFactor":color,"metallicFactor":metal,"roughnessFactor":0.35}});
        if key == "part-3" {
            material["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index": 0});
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
