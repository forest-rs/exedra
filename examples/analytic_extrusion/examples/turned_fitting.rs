// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native revolutions with a tapered bore, shoulders, grooves and construction UVs.
//! `cargo run -p analytic_extrusion --example turned_fitting -- /tmp/turned-fitting`
//! Writes GLB and an exact generating-profile SVG. The three occurrences compare
//! coarse geometry, a charted partial cutaway, and a charted fine full turn.

use exedra::analytic::geometry::Surface;
use exedra::analytic::realize::RealizationPolicy;
use exedra::analytic::revolve::{FaceRole, Revolution};
use exedra::compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource};
use exedra::constructive::chart::{ChartTransform, SurfaceChart};
use exedra::constructive::ir::Placement3;
use exedra::constructive::profile::{Loop2, Profile2, Seg2};
use exedra::curves::profile::{ProfilePolicy, SegTag};
use exedra::gltf::{
    GltfExportOptions, GltfMesh, GltfNode, GltfScene, MaterialResolver, Texture,
    export_glb_with_materials,
};
use serde_json::{Value, json};
use std::error::Error;

const OUTER: SegTag = SegTag(0x004f_5554);
const BORE: SegTag = SegTag(0x0042_4f52);
const OUTSIDE: &[[f64; 2]] = &[
    [22.0, 0.0],
    [22.0, 5.0],
    [18.0, 5.0],
    [18.0, 10.0],
    [20.0, 10.0],
    [20.0, 13.0],
    [18.0, 13.0],
    [18.0, 30.0],
    [14.0, 38.0],
    [14.0, 44.0],
    [11.0, 47.0],
    [7.0, 47.0],
];
const INSIDE: &[[f64; 2]] = &[
    [7.0, 26.0],
    [10.0, 19.0],
    [10.0, 8.0],
    [7.0, 3.0],
    [7.0, 0.0],
];
fn profile() -> Result<Profile2, Box<dyn Error>> {
    // Tag each polyline feature once, carrying it through shared profile lowering.
    let segments = OUTSIDE
        .iter()
        .map(|p| Seg2::line((p[0], p[1])).tagged(OUTER))
        .chain(INSIDE.iter().map(|p| Seg2::line((p[0], p[1])).tagged(BORE)))
        .collect();
    Ok(Profile2::new(Loop2::new(segments)?, vec![])?)
}
struct Materials;
impl MaterialResolver for Materials {
    fn resolve(&self, key: &str) -> Option<Value> {
        let charted = key.starts_with("grid");
        let color = if key.ends_with("bore") {
            [0.82, 0.56, 0.22, 1.0]
        } else if key.ends_with("cap") {
            [0.95, 0.43, 0.11, 1.0]
        } else {
            [0.58, 0.72, 0.79, 1.0]
        };
        let mut pbr = json!({"baseColorFactor":color,"metallicFactor":if charted {0.15}else{0.65},"roughnessFactor":0.38});
        if charted && !key.ends_with("cap") {
            pbr["baseColorTexture"] = json!({"index":0});
        }
        Some(json!({"name":key,"pbrMetallicRoughness":pbr}))
    }
    fn resolve_texture(&self, index: u32) -> Option<Texture<'_>> {
        (index == 0).then_some(Texture {
            image: include_bytes!("../../material_gallery/assets/unit-grid.png"),
            mime_type: "image/png",
            sampler: Some(json!({"wrapS":10497,"wrapT":10497,"magFilter":9729,"minFilter":9987})),
        })
    }
}
fn region(role: FaceRole) -> u32 {
    match role {
        FaceRole::Cap { .. } => 2,
        FaceRole::Wall {
            tag: Some(BORE), ..
        } => 1,
        _ => 0,
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "turned-fitting".to_owned());
    let authoring_profile = profile()?;
    let profile = authoring_profile.to_profile(&ProfilePolicy::new(1e-6))?;
    let chart = SurfaceChart::Revolve {
        reference_radius: 18.0,
        wall: ChartTransform {
            matrix: [[1.0 / 12.0, 0.0], [0.0, 1.0 / 12.0]],
            offset: [0.0; 2],
        },
        caps: ChartTransform {
            matrix: [[1.0 / 12.0, 0.0], [0.0, 1.0 / 12.0]],
            offset: [0.0; 2],
        },
    };
    let cases = [
        ("coarse native", -60.0, core::f64::consts::TAU, 0.35, false),
        (
            "charted cutaway",
            0.0,
            1.5 * core::f64::consts::PI,
            0.015,
            true,
        ),
        (
            "charted full turn",
            60.0,
            core::f64::consts::TAU,
            0.015,
            true,
        ),
    ];
    let mut sources = Vec::new();
    for &(name, x, sweep, tolerance, charted) in &cases {
        let solid = Revolution::new(
            profile.clone(),
            Placement3::rotate_x_then_translate(core::f64::consts::FRAC_PI_2, x, 0.0, 0.0),
            sweep,
        )?;
        let policy = RealizationPolicy {
            tolerance,
            ..RealizationPolicy::default()
        };
        let result = if charted {
            solid.realize_with_uvs(
                &policy,
                |_, f| region(f.role),
                |_, role, points| match role {
                    FaceRole::Wall { .. } => chart.map_revolve_wall_triangle(points),
                    FaceRole::Cap { .. } => chart.map_cap_triangle(points),
                },
            )?
        } else {
            solid.realize_with_regions(&policy, |_, f| region(f.role))?
        };
        assert!(solid.body().is_closed(), "native fitting must be closed");
        assert!(
            result.mesh.boundary_loops()?.is_empty(),
            "emitted fitting must be closed"
        );
        assert!(
            result.mesh.validate_deep().is_empty(),
            "emitted topology must be valid"
        );
        assert!(
            result.report.within_tolerance(),
            "emitted fitting must meet its requested accuracy"
        );
        let cones = solid
            .body()
            .faces()
            .iter()
            .filter(|f| matches!(f.surface, Surface::Cone(_)))
            .count();
        println!(
            "{name}: {} native faces ({cones} cones), {} triangles, volume {:.3} mm3, area {:.3} mm2, deviation {:.6}/{tolerance} mm; {} tagged bore faces",
            solid.body().faces().len(),
            result.mesh_faces.len(),
            solid.volume(),
            solid.surface_area(),
            result.report.max_deviation,
            solid.faces_tagged(BORE).count()
        );
        sources.push(PartSource::Baked(result.mesh));
    }
    write_scene(&format!("{out}.glb"), &cases, &sources)?;
    comparison(&out, &authoring_profile, chart)?;
    let points = OUTSIDE
        .iter()
        .chain(INSIDE)
        .map(|p| format!("{},{}", 60.0 + 10.0 * p[0], 550.0 - 10.0 * p[1]))
        .collect::<Vec<_>>()
        .join(" ");
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="620" viewBox="0 0 1000 620">
<rect width="1000" height="620" fill="#101b28"/><g font-family="sans-serif" fill="#e5eef5"><text x="45" y="38" font-size="25">Retained revolution · generating section</text><text x="45" y="65" font-size="15">Radius / height in mm · positive radius · right-handed rotation about the height axis</text></g>
<line x1="60" y1="90" x2="60" y2="565" stroke="#708597" stroke-dasharray="5 5"/>
<polygon points="{points}" fill="#375369" stroke="#a9d9e3" stroke-width="3"/>
<g font-family="sans-serif" fill="#dbe7f0" font-size="18"><text x="390" y="160">Same profile, three occurrences</text><text x="390" y="200">Left: coarse full turn, 0.35 mm tolerance</text><text x="390" y="235">Middle: 270° cutaway, 0.015 mm tolerance</text><text x="390" y="270">Right: charted full turn, 0.015 mm tolerance</text><text x="390" y="330">Planes · cylinders · conical shoulders and bore</text><text x="390" y="365">Shared boundaries and native segment tags</text><text x="390" y="400">Chart: (18 × theta, profile distance), 12 mm repeats</text><text x="390" y="435">UV seams share geometry vertices</text><text x="390" y="495">GLB embeds the construction-chart checker texture</text></g></svg>"##
    );
    std::fs::write(format!("{out}.svg"), svg)?;
    println!("Wrote {out}.glb and {out}.svg");
    Ok(())
}

fn write_scene(
    path: &str,
    cases: &[(&str, f64, f64, f64, bool)],
    sources: &[PartSource],
) -> Result<(), Box<dyn Error>> {
    let inputs: Vec<_> = cases
        .iter()
        .zip(sources)
        .map(|((name, ..), source)| PartInput::new(name, source))
        .collect();
    let compiled = PartCompiler::new().compile_parts(&inputs, &CompilePolicy::default())?;
    let mut root = GltfNode::new("native turned fitting studies");
    root.transform = Placement3::from_axes(
        [0.001, 0.0, 0.0],
        [0.0, 0.001, 0.0],
        [0.0, 0.0, 0.001],
        [0.0; 3],
    );
    let mut nodes = vec![root];
    for (i, (name, _, _, _, charted)) in cases.iter().enumerate() {
        let id = PartId(u32::try_from(i)?);
        let keys = if *charted {
            ["grid_outer", "grid_bore", "grid_cap"]
        } else {
            ["metal_outer", "metal_bore", "metal_cap"]
        };
        let mut mesh = GltfMesh::new(id, *name, &compiled)?;
        mesh.materials = compiled
            .part(id)
            .expect("compiled fitting")
            .bodies
            .iter()
            .map(|body| {
                body.regions
                    .iter()
                    .map(|range| {
                        assert!(
                            range.has_uvs == *charted,
                            "construction UVs must survive compilation"
                        );
                        Some(keys[range.region as usize].to_owned())
                    })
                    .collect()
            })
            .collect();
        let mut node = GltfNode::new(*name);
        node.parent = Some(0);
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
    assert_eq!(
        output.stats.images, 1,
        "both charted occurrences share one embedded grid image"
    );
    std::fs::write(path, output.bytes)?;
    Ok(())
}

fn comparison(out: &str, profile: &Profile2, chart: SurfaceChart) -> Result<(), Box<dyn Error>> {
    use exedra::constructive::ir::CapMode;
    use exedra::constructive::tessellate::{EvalPolicy, tessellate_revolve_with_chart};
    use exedra::curves::profile::{CircularEdgeConstraints, circular_edge_count};
    let sweep = 1.5 * core::f64::consts::PI;
    let tolerance = 0.015;
    let steps = circular_edge_count(
        22.0,
        sweep,
        tolerance,
        CircularEdgeConstraints::new(4, 4096),
    )?;
    let cases = [
        ("native analytic", -35.0, sweep, tolerance, true),
        ("mesh-based constructive", 35.0, sweep, tolerance, true),
    ];
    let pose = |x| Placement3::rotate_x_then_translate(core::f64::consts::FRAC_PI_2, x, 0.0, 0.0);
    let native = Revolution::new(
        profile.to_profile(&ProfilePolicy::new(1e-6))?,
        pose(-35.0),
        sweep,
    )?;
    let native = native.realize_with_uvs(
        &RealizationPolicy {
            tolerance,
            min_arc_segments: steps,
            max_edge_segments: steps,
            ..RealizationPolicy::default()
        },
        |_, _| 0,
        |_, role, p| match role {
            FaceRole::Wall { .. } => chart.map_revolve_wall_triangle(p),
            FaceRole::Cap { .. } => chart.map_cap_triangle(p),
        },
    )?;
    let mut policy = EvalPolicy::default();
    policy.discretize.chord_tolerance = tolerance;
    policy.discretize.min_arc_edges = steps;
    policy.discretize.max_segment_edges = steps;
    let mut mesh =
        tessellate_revolve_with_chart(profile, &pose(35.0), sweep, CapMode::Both, chart, &policy)?
            .mesh;
    let faces: Vec<_> = mesh.faces().collect();
    let mut edit = mesh.edit();
    for face in faces {
        exedra::mesh::op::set_face_region(&mut edit, face, 0)?;
    }
    let _: () = edit.finish();
    let meshes = [native.mesh, mesh];
    let local_points = |mesh: &exedra::Mesh, x: f64| {
        mesh.vertices()
            .map(|v| {
                let p = mesh.vertex_position(v).unwrap();
                [f64::from(p[0]) - x, f64::from(p[1]), f64::from(p[2])]
            })
            .collect::<Vec<_>>()
    };
    let a = local_points(&meshes[0], -35.0);
    let b = local_points(&meshes[1], 35.0);
    let distance = |a: [f64; 3], b: [f64; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    let directed = |from: &[[f64; 3]], to: &[[f64; 3]]| {
        from.iter()
            .map(|&p| {
                to.iter()
                    .map(|&q| distance(p, q))
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0.0, f64::max)
    };
    let vertex_difference = directed(&a, &b).max(directed(&b, &a));
    assert!(
        vertex_difference < 1e-4,
        "matched sampling geometry must agree within emitted rounding"
    );
    println!(
        "matched local point sets: maximum nearest-vertex difference {vertex_difference:.9} mm"
    );
    let mut records = Vec::new();
    for (i, mesh) in meshes.iter().enumerate() {
        assert!(
            mesh.boundary_loops()?.is_empty(),
            "comparison geometry must be closed"
        );
        assert!(
            mesh.validate_deep().is_empty(),
            "comparison topology must be valid"
        );
        let positions: Vec<_> = mesh
            .vertices()
            .map(|v| *mesh.vertex_position(v).unwrap())
            .collect();
        let edges: Vec<_> = mesh
            .half_edges()
            .filter(|&h| mesh.canonical_edge(h) == Some(h))
            .map(|h| {
                [
                    mesh.from_vertex(h).unwrap().index(),
                    mesh.to_vertex(h).unwrap().index(),
                ]
            })
            .collect();
        let face_sizes: Vec<_> = mesh.faces().map(|f| mesh.face_loop(f).count()).collect();
        let triangles: usize = face_sizes.iter().map(|n| n - 2).sum();
        println!(
            "comparison {}: {} vertices, {} polygon faces, {triangles} compiled triangles, {} raw edges, {steps} angular intervals",
            cases[i].0,
            positions.len(),
            face_sizes.len(),
            edges.len()
        );
        records.push(json!({"name":cases[i].0,"positions_mm":positions,"edges":edges,"polygon_faces":face_sizes.len(),"quad_faces":face_sizes.iter().filter(|&&n|n==4).count(),"triangular_faces":face_sizes.iter().filter(|&&n|n==3).count(),"triangles":triangles,"angular_intervals":steps,"matched_vertex_difference_mm":vertex_difference}));
    }
    assert_eq!(
        records[0]["triangles"], records[1]["triangles"],
        "matched angular sampling must yield equal final triangle counts"
    );
    std::fs::write(
        format!("{out}-comparison-topology.json"),
        serde_json::to_vec(&records)?,
    )?;
    let sources = meshes
        .into_iter()
        .map(PartSource::Baked)
        .collect::<Vec<_>>();
    write_scene(&format!("{out}-comparison.glb"), &cases, &sources)?;
    Ok(())
}
