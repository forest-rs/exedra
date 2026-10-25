// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! An instrument front panel from exact analytic extrusions, exported with
//! materials bound to queried features.
//!
//! A bezel surrounds an inset faceplate with a 1.5 mm reveal: the bezel's
//! opening is the faceplate outline offset outward, so its corners stay
//! concentric arcs. The faceplate has a recessed display window holding a
//! lens, a knob bore, vent slots drawn with a cursor, indicator holes, and
//! four blind bores in its back for threaded inserts.
//!
//! The vent slots' segments carry a tag. Realization groups every wall with
//! that tag into one region, and the window's recess into another, so the
//! export binds a material once per feature instead of once per face.
//!
//! ```sh
//! cargo run -p analytic_extrusion --example instrument_panel -- panel.glb
//! ```

use exedra::analytic::extrusion::{End, Extrusion, FaceId, Pocket, TessellationPolicy};
use exedra::compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource};
use exedra::constructive::builders;
use exedra::constructive::ir::Placement3;
use exedra::constructive::offset::CornerPolicy;
use exedra::constructive::profile::Profile2;
use exedra::gltf::{GltfExportOptions, GltfMesh, GltfNode, GltfScene, export_glb_with_materials};
use exedra::profile::{ProfilePolicy, SegTag};

const POLICY: ProfilePolicy = ProfilePolicy::new(1e-6);

/// Tag on every segment of the vent slots. Builders tag their segments with
/// small indices, so a feature tag must not reuse them; `tagged` replaces
/// them on the slot.
const VENT: SegTag = SegTag(0x5645_4e54);
const WINDOW: SegTag = SegTag(0x5749_4e44);

/// Faceplate regions: the plate, the vents' walls, and the window recess.
const PLATE: u32 = 0;
const VENTS: u32 = 1;
const RECESS: u32 = 2;

fn shared(profile: &Profile2) -> exedra::profile::Profile {
    profile.to_line_arc(&POLICY).expect("line and arc profile")
}

fn solid(profile: &Profile2, z: f64, length: f64) -> Result<Extrusion, Box<dyn std::error::Error>> {
    Ok(Extrusion::new(
        shared(profile),
        Placement3::translate(0.0, 0.0, z),
        length,
    )?)
}

/// The faceplate's region for a native face: vent walls and the window's
/// recess are features; everything else is plate.
fn faceplate_region(solid: &Extrusion, face: FaceId) -> u32 {
    match solid.face_tag(face) {
        Some(VENT) => VENTS,
        Some(WINDOW) => RECESS,
        _ => PLATE,
    }
}

#[expect(clippy::too_many_lines, reason = "one linear walkthrough")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1);
    // Millimeters. The faceplate is 200 x 120 with 4 mm corners, 6 thick,
    // its front at z = 6.
    let (width, height, thickness) = (200.0, 120.0, 6.0);
    let faceplate_outline = builders::rounded_rect_centered(width, height, 4.0)?;

    // The bezel's opening is the faceplate grown by a 1.5 mm reveal.
    let opening = faceplate_outline.offset(1.5, CornerPolicy::Round)?;
    let bezel = builders::rounded_rect_centered(236.0, 156.0, 10.0)?.with_cutouts([opening])?;

    // One vent slot, 35 long with 2.5 mm round ends, tagged as a feature once.
    let slot = builders::slot_centered(35.0, 2.5)?.tagged(VENT);

    // Through features: a knob bore, two indicator holes, and the slot
    // placed at five heights.
    let knob_at = (60.0, -10.0);
    let mut cutouts = vec![
        builders::circle(3.5)?.translated(knob_at)?,
        builders::circle(1.5)?.translated((45.0, 35.0))?,
        builders::circle(1.5)?.translated((75.0, 35.0))?,
    ];
    for row in 0..5 {
        cutouts.push(slot.translated((-70.0, -30.0 + 12.0 * f64::from(row)))?);
    }
    let faceplate = faceplate_outline.with_cutouts(cutouts)?;

    // A display window recessed 2 mm into the front, and four 4 mm deep
    // blind bores in the back for threaded inserts.
    let window = builders::rounded_rect_centered(90.0, 50.0, 3.0)?.translated((-5.0, 5.0))?;
    let mut panel = solid(&faceplate, 0.0, thickness)?.with_pocket(
        Pocket::from_region(shared(&window).outer(), End::End, 2.0).tagged(WINDOW),
        &POLICY,
    )?;
    for (x, y) in [(-88.0, -50.0), (88.0, -50.0), (-88.0, 50.0), (88.0, 50.0)] {
        let bore = shared(&builders::circle(2.5)?.translated((x, y))?);
        panel = panel.with_pocket(Pocket::from_region(bore.outer(), End::Start, 4.0), &POLICY)?;
    }

    // A lens floating in the window's recess, 0.3 mm clear of its walls.
    let lens = window.offset(-0.3, CornerPolicy::Round)?;
    let knob_stem = builders::circle(6.0)?.translated(knob_at)?;
    let knob = builders::circle(14.0)?.translated(knob_at)?;

    // Each part, with the material key of each of its regions.
    let parts: [(&str, Extrusion, &[&str]); 5] = [
        ("bezel", solid(&bezel, -2.0, 12.0)?, &["anodized"]),
        ("faceplate", panel, &["aluminum", "vent", "recess"]),
        ("lens", solid(&lens, thickness - 2.0, 1.5)?, &["glass"]),
        ("knob_stem", solid(&knob_stem, thickness, 8.0)?, &["knob"]),
        ("knob", solid(&knob, thickness + 8.0, 12.0)?, &["knob"]),
    ];
    let policy = TessellationPolicy {
        chord_tolerance: 0.02,
        ..TessellationPolicy::default()
    };
    let mut sources = Vec::with_capacity(parts.len());
    for (name, part, _) in &parts {
        let realized = if *name == "faceplate" {
            part.tessellate_with_regions(&policy, |id, _| faceplate_region(part, id))?
        } else {
            part.tessellate_with_regions(&policy, |_, _| PLATE)?
        };
        println!(
            "{name:>9}: {:>3} native faces, volume {:>10.1} mm3, {:>4} mesh faces, \
             deviation {:.4} mm, closed {}",
            part.faces().len(),
            part.volume(),
            realized.mesh.faces().count(),
            realized.report.max_deviation,
            realized.mesh.boundary_loops()?.is_empty(),
        );
        if *name == "faceplate" {
            let vents = part
                .faces()
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    faceplate_region(part, FaceId(u32::try_from(*index).expect("few faces")))
                        == VENTS
                })
                .count();
            let vent_faces = realized
                .mesh_faces
                .iter()
                .filter(|(_, native)| faceplate_region(part, *native) == VENTS)
                .count();
            println!(
                "           vent walls: {vents} native faces, {vent_faces} mesh faces, one region"
            );
        }
        sources.push(PartSource::Baked(realized.mesh));
    }

    // Compile every part as baked geometry and bind each region's material.
    let inputs: Vec<PartInput<'_>> = parts
        .iter()
        .zip(&sources)
        .map(|((name, ..), source)| PartInput::new(name, source))
        .collect();
    let compiled = PartCompiler::new().compile_parts(&inputs, &CompilePolicy::default())?;
    // The root scales millimeters to glTF's meters.
    let mut root = GltfNode::new("instrument_panel");
    root.transform = Placement3::from_axes(
        [1e-3, 0.0, 0.0],
        [0.0, 1e-3, 0.0],
        [0.0, 0.0, 1e-3],
        [0.0; 3],
    );
    let mut nodes = vec![root];
    for (index, (name, _, keys)) in parts.iter().enumerate() {
        let part = PartId(u32::try_from(index)?);
        let mut mesh = GltfMesh::new(part, *name, &compiled)?;
        mesh.materials = compiled
            .part(part)
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
    let scene = GltfScene {
        nodes,
        ..GltfScene::default()
    };
    let resolve = |key: &str| {
        let (color, metallic, roughness) = match key {
            "aluminum" => ([0.80, 0.81, 0.83, 1.0], 0.9, 0.35),
            "anodized" => ([0.08, 0.09, 0.10, 1.0], 0.6, 0.45),
            "vent" => ([0.95, 0.42, 0.10, 1.0], 0.2, 0.5),
            "recess" => ([0.35, 0.36, 0.38, 1.0], 0.8, 0.5),
            "glass" => ([0.05, 0.16, 0.22, 1.0], 0.0, 0.05),
            "knob" => ([0.04, 0.04, 0.05, 1.0], 0.1, 0.6),
            _ => return None,
        };
        Some(serde_json::json!({
            "name": key,
            "pbrMetallicRoughness": {
                "baseColorFactor": color,
                "metallicFactor": metallic,
                "roughnessFactor": roughness,
            }
        }))
    };
    let export = export_glb_with_materials(
        &scene,
        &compiled,
        &resolve,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    println!(
        "GLB: {} bytes, {} meshes, {} materials",
        export.bytes.len(),
        export.stats.meshes,
        export.stats.materials
    );
    if let Some(out) = out {
        std::fs::write(out, &export.bytes)?;
    }
    Ok(())
}
