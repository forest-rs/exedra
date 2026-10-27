// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reusable profile-level shoulder treatments, retained as native revolutions.
//! `cargo run -p analytic_extrusion --example profile_shoulders -- /tmp/profile-shoulders`

use exedra::analytic::geometry::Surface;
use exedra::analytic::revolve::Revolution;
use exedra::compile::{CompilePolicy, NativeSource, PartCompiler, PartId, PartInput, PartSource};
use exedra::constructive::ir::Placement3;
use exedra::gltf::{GltfExportOptions, GltfMesh, GltfNode, GltfScene, export_glb_with_materials};
use exedra::profile::{CornerTreatment, Loop, Profile, ProfilePolicy, SegTag, Segment};
use serde_json::{Value, json};
use std::error::Error;

const BODY: SegTag = SegTag(1);
const SHOULDER: SegTag = SegTag(2);
const POLICY: ProfilePolicy = ProfilePolicy::new(1e-6);

fn section() -> Result<Profile, Box<dyn Error>> {
    Ok(Profile::new(
        Loop::new(
            [
                [12.0, 0.0],
                [22.0, 0.0],
                [22.0, 8.75],
                [18.0, 8.75],
                [18.0, 35.0],
                [12.0, 35.0],
            ]
            .into_iter()
            .map(Segment::line)
            .collect(),
        )?,
        vec![],
        &POLICY,
    )?
    .tagged(BODY))
}
fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "profile-shoulders".into());
    let source = section()?;
    let selected = source.corner_at([18.0, 8.75], 1e-6)?;
    let rounded = source.with_corners(
        [selected.segment],
        CornerTreatment::fillet(2.0).tagged(SHOULDER),
        &POLICY,
    )?;
    let chamfered = source.with_corners(
        [selected.segment],
        CornerTreatment::chamfer(2.0).tagged(SHOULDER),
        &POLICY,
    )?;
    assert_eq!(
        rounded.changed_corners, 1,
        "one authored shoulder must be treated"
    );
    let profiles = [source, rounded.profile, chamfered.profile];
    let names = ["sharp", "fillet radius 2", "chamfer distance 2"];
    let mut sources = Vec::new();
    let mut evidence = Vec::new();
    for (index, profile) in profiles.into_iter().enumerate() {
        let placement = Placement3::euler_extrinsic_xyz_then_translate(
            core::f64::consts::FRAC_PI_2,
            0.0,
            0.0,
            [60.0 * f64::from(u32::try_from(index)?) - 60.0, 0.0, 0.0],
        );
        let part = Revolution::new(profile, placement, 1.5 * core::f64::consts::PI)?;
        let transitions: Vec<_> = part.body().faces().iter().filter_map(|face| match &face.surface {
            Surface::Torus(t) => Some(json!({"surface":"torus","majorRadius":t.major_radius(),"minorRadius":t.minor_radius()})),
            Surface::Cone(_) => Some(json!({"surface":"cone"})),
            _ => None,
        }).collect();
        assert_eq!(
            transitions.len(),
            usize::from(index != 0),
            "only treated profiles generate a tagged transition"
        );
        evidence.push(json!({"name":names[index],"volume":part.volume(),"area":part.surface_area(),"nativeFaces":part.body().faces().len(),"transitions":transitions}));
        sources.push(PartSource::Native(NativeSource::from(part)));
    }
    let inputs: Vec<_> = sources
        .iter()
        .zip(names)
        .map(|(source, name)| PartInput::new(name, source))
        .collect();
    let mut policy = CompilePolicy::default();
    policy.native.tolerance = 0.05;
    let compiled = PartCompiler::new().compile_parts(&inputs, &policy)?;
    let mut root = GltfNode::new("profile shoulders");
    root.transform = Placement3::from_axes(
        [0.001, 0.0, 0.0],
        [0.0, 0.001, 0.0],
        [0.0, 0.0, 0.001],
        [0.0; 3],
    );
    let mut nodes = vec![root];
    for (index, name) in names.into_iter().enumerate() {
        let id = PartId(u32::try_from(index)?);
        let mut node = GltfNode::new(name);
        node.parent = Some(0);
        node.geometry = Some(GltfMesh::new(id, name, &compiled)?.into());
        nodes.push(node);
    }
    let materials = |_: &str| -> Option<Value> { None };
    let glb = export_glb_with_materials(
        &GltfScene {
            nodes,
            ..GltfScene::default()
        },
        &compiled,
        &materials,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    std::fs::write(format!("{output}.glb"), glb.bytes)?;
    std::fs::write(
        format!("{output}.json"),
        serde_json::to_vec_pretty(
            &json!({"units":"mm","selectedPoint":selected.point,"cases":evidence}),
        )?,
    )?;
    println!("Wrote {output}.glb and {output}.json");
    Ok(())
}
