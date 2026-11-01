// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Three authoring routes, one retained body and one explicit realization path.
//!
//! `cargo run -p analytic_extrusion --example boundary_geometry -- boundary.glb`
//! The cylinder is authored directly: two vertices, three edges, three faces;
//! its wall uses its seam twice. The pocketed plate comes from a profile and
//! the box from six planar faces. All construction records are dropped before
//! realization. Region materials and native correspondence remain available.

use exedra::analytic::body::{
    Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeGeometry, EdgeId, Face, FaceId, LoopId, Sense,
    ValidationPolicy, VertexId,
};
use exedra::analytic::extrusion::{End, Extrusion, Pocket};
use exedra::analytic::geometry::{Curve, ParameterDomain, ParameterRange, Surface};
use exedra::analytic::realize::RealizationPolicy;
use exedra::analytic::{AnalyticShellBuilder, RegionId};
use exedra::compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource};
use exedra::constructive::builders;
use exedra::constructive::ir::Placement3;
use exedra::curves::profile::{ProfilePolicy, SegTag};
use exedra::gltf::{GltfExportOptions, GltfMesh, GltfNode, GltfScene, export_glb_with_materials};

fn cylinder() -> Result<Body, Box<dyn std::error::Error>> {
    let circle = |z| Curve::circle(Placement3::translate(0.0, 0.0, z), 13.0).unwrap();
    let full = ParameterRange::new(0.0, core::f64::consts::TAU).unwrap();
    let coedge = |edge, reversed| Coedge {
        edge: EdgeId(edge),
        reversed,
    };
    let parts = BodyParts {
        trims: Vec::new(),
        vertices: vec![[13.0, 0.0, 0.0], [13.0, 0.0, 24.0]],
        edges: vec![
            Edge {
                geometry: EdgeGeometry::Curve {
                    curve: circle(0.0),
                    range: full,
                },
                start: VertexId(0),
                end: VertexId(0),
                role: (),
            },
            Edge {
                geometry: EdgeGeometry::Curve {
                    curve: circle(24.0),
                    range: full,
                },
                start: VertexId(1),
                end: VertexId(1),
                role: (),
            },
            Edge {
                geometry: EdgeGeometry::Curve {
                    curve: Curve::line([13.0, 0.0, 0.0], [0.0, 0.0, 24.0]).unwrap(),
                    range: ParameterRange::UNIT,
                },
                start: VertexId(0),
                end: VertexId(1),
                role: (),
            },
        ],
        loops: vec![
            BoundaryLoop {
                coedges: vec![
                    coedge(0, false),
                    coedge(2, false),
                    coedge(1, true),
                    coedge(2, true),
                ],
            },
            BoundaryLoop {
                coedges: vec![coedge(0, true)],
            },
            BoundaryLoop {
                coedges: vec![coedge(1, false)],
            },
        ],
        faces: vec![
            Face {
                surface: Surface::cylinder(Placement3::IDENTITY, 13.0).unwrap(),
                domain: ParameterDomain {
                    u: full,
                    v: ParameterRange::new(0.0, 24.0).unwrap(),
                },
                sense: Sense::Forward,
                loops: vec![LoopId(0)],
                role: (),
            },
            Face {
                surface: Surface::plane(Placement3::IDENTITY).unwrap(),
                domain: ParameterDomain {
                    u: ParameterRange::new(-13.0, 13.0).unwrap(),
                    v: ParameterRange::new(-13.0, 13.0).unwrap(),
                },
                sense: Sense::Reverse,
                loops: vec![LoopId(1)],
                role: (),
            },
            Face {
                surface: Surface::plane(Placement3::translate(0.0, 0.0, 24.0)).unwrap(),
                domain: ParameterDomain {
                    u: ParameterRange::new(-13.0, 13.0).unwrap(),
                    v: ParameterRange::new(-13.0, 13.0).unwrap(),
                },
                sense: Sense::Forward,
                loops: vec![LoopId(2)],
                role: (),
            },
        ],
    };
    Ok(Body::try_from_parts(
        parts,
        &ValidationPolicy { max_gap: 1e-10 },
    )?)
}

fn plate() -> Result<Body<u32>, Box<dyn std::error::Error>> {
    let policy = ProfilePolicy::new(1e-6);
    let holes = [-29.0, 29.0].map(|x| builders::circle(3.5).and_then(|p| p.translated((x, 0.0))));
    let profile = builders::rounded_rect_centered(80.0, 44.0, 4.0)?
        .with_cutouts(holes.into_iter().collect::<Result<Vec<_>, _>>()?)?
        .to_profile(&policy)?;
    let window = builders::rounded_rect_centered(36.0, 24.0, 3.0)?.to_profile(&policy)?;
    let recess = SegTag(1000);
    let solid = Extrusion::new(profile, Placement3::IDENTITY, 8.0)?.with_pocket(
        Pocket::from_profile(&window, End::End, 3.0)?.tagged(recess),
        &policy,
    )?;
    let mut parts = solid.body().clone().into_parts();
    // Geometry is independent of the caller metadata: replace the construction
    // roles with material regions and discard the extrusion altogether.
    let faces = parts
        .faces
        .into_iter()
        .enumerate()
        .map(|(i, face)| Face {
            role: u32::from(
                solid.face_tag(FaceId(u32::try_from(i).expect("small body"))) == Some(recess),
            ),
            surface: face.surface,
            domain: face.domain,
            sense: face.sense,
            loops: face.loops,
        })
        .collect();
    let edges = parts
        .edges
        .into_iter()
        .map(|edge| Edge {
            geometry: edge.geometry,
            start: edge.start,
            end: edge.end,
            role: (),
        })
        .collect();
    drop(solid);
    Ok(Body::try_from_parts(
        BodyParts {
            trims: Vec::new(),
            vertices: core::mem::take(&mut parts.vertices),
            edges,
            loops: parts.loops,
            faces,
        },
        &ValidationPolicy { max_gap: 1e-10 },
    )?)
}

fn box_body() -> Result<Body<RegionId>, Box<dyn std::error::Error>> {
    let mut builder = AnalyticShellBuilder::new();
    let vertices = [
        [-13.0, -13.0, 0.0],
        [13.0, -13.0, 0.0],
        [13.0, 13.0, 0.0],
        [-13.0, 13.0, 0.0],
        [-13.0, -13.0, 26.0],
        [13.0, -13.0, 26.0],
        [13.0, 13.0, 26.0],
        [-13.0, 13.0, 26.0],
    ]
    .map(|p| builder.push_vertex(p));
    for (i, face) in [
        [0, 3, 2, 1],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ]
    .into_iter()
    .enumerate()
    {
        builder.add_planar_face(&face.map(|j| vertices[j]), RegionId(u32::try_from(i % 2)?))?;
    }
    Ok(builder.build()?.body().clone())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let policy = RealizationPolicy {
        tolerance: 0.02,
        ..RealizationPolicy::default()
    };
    let cylinder = cylinder()?;
    let plate = plate()?;
    let box_body = box_body()?;
    let models = [
        (
            "periodic cylinder",
            cylinder.realize_with_regions(
                Placement3::translate(-65.0, 0.0, 0.0),
                &policy,
                |id, _| u32::from(id != FaceId(0)),
            )?,
        ),
        (
            "pocketed plate",
            plate.realize_with_regions(Placement3::IDENTITY, &policy, |_, face| face.role)?,
        ),
        (
            "planar box",
            box_body.realize_with_regions(
                Placement3::translate(65.0, 0.0, 0.0),
                &policy,
                |_, face| face.role.0,
            )?,
        ),
    ];
    let mut sources = Vec::new();
    for (name, result) in &models {
        assert!(
            result.mesh.boundary_loops()?.is_empty(),
            "{name} must remain closed"
        );
        assert!(
            result.mesh.validate_deep().is_empty(),
            "{name} must have valid mesh topology"
        );
        assert!(
            result.report.within_tolerance(),
            "{name} must meet the requested accuracy"
        );
        println!(
            "{name}: {} vertices, {} triangles, {} shared edge chains; deviation {:.6} / {}",
            result.mesh.vertices().count(),
            result.mesh_faces.len(),
            result.edge_chains.len(),
            result.report.max_deviation,
            policy.tolerance
        );
        sources.push(PartSource::Baked(result.mesh.clone()));
    }
    let inputs: Vec<_> = models
        .iter()
        .zip(&sources)
        .map(|((name, _), source)| PartInput::new(name, source))
        .collect();
    let compiled = PartCompiler::new().compile_parts(&inputs, &CompilePolicy::default())?;
    let mut root = GltfNode::new("shared analytic boundary");
    root.transform = Placement3::from_axes(
        [0.001, 0.0, 0.0],
        [0.0, 0.001, 0.0],
        [0.0, 0.0, 0.001],
        [0.0; 3],
    );
    let mut nodes = vec![root];
    for (index, (name, _)) in models.iter().enumerate() {
        let id = PartId(u32::try_from(index)?);
        let keys = [
            ["cylinder", "cap"],
            ["plate", "recess"],
            ["box", "box_accent"],
        ][index];
        let mut mesh = GltfMesh::new(id, *name, &compiled)?;
        mesh.materials = compiled
            .part(id)
            .expect("compiled part")
            .bodies
            .iter()
            .map(|body| {
                body.regions
                    .iter()
                    .map(|range| Some(keys[range.region as usize].to_owned()))
                    .collect()
            })
            .collect();
        let mut node = GltfNode::new(*name);
        node.parent = Some(0);
        node.geometry = Some(mesh.into());
        nodes.push(node);
    }
    let resolve = |key: &str| {
        let color = match key {
            "cylinder" => [0.10, 0.38, 0.48, 1.0],
            "cap" => [0.24, 0.65, 0.68, 1.0],
            "plate" => [0.72, 0.75, 0.78, 1.0],
            "recess" => [0.11, 0.16, 0.23, 1.0],
            "box" => [0.82, 0.38, 0.12, 1.0],
            "box_accent" => [0.98, 0.64, 0.22, 1.0],
            _ => return None,
        };
        Some(
            serde_json::json!({"name":key, "pbrMetallicRoughness": {"baseColorFactor":color, "metallicFactor":0.45, "roughnessFactor":0.35}}),
        )
    };
    let output = export_glb_with_materials(
        &GltfScene {
            nodes,
            ..GltfScene::default()
        },
        &compiled,
        &resolve,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "boundary_geometry.glb".to_owned());
    std::fs::write(&path, output.bytes)?;
    println!("Exported {path}");
    Ok(())
}
