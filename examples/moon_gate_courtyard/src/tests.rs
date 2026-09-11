// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use exedra_assembly::{Assembly, CompiledParts, InstanceId, InstancePath};

#[test]
fn gate_variants_compile_cleanly_and_material_edits_reuse_geometry() -> Result<()> {
    for radius in [1350, 1550, 1700] {
        let mut scene = Scene::default();
        let c = architecture::gate(&mut scene, radius)?;
        let mut compiler = PartCompiler::new();
        let compiled = compiler.compile_parts(&scene.assembly, &policy())?;
        for (i, part) in compiled.parts().iter().enumerate() {
            assert!(part.triangle_count() > 0);
            assert!(
                compiled
                    .report(PartId(u32::try_from(i)?))
                    .is_none_or(|report| report.clean_at(Severity::Warning))
            );
        }
        assert!(
            scene
                .assembly
                .resolve_path(&InstancePath::from_segments(&["entry-gate-wall"]))
                .is_none()
        );
        for e in c.elements().iter().skip(1) {
            let key = format!("entry-{}", e.key);
            let id = scene
                .assembly
                .resolve_path(&InstancePath::from_segments(&[&key]))
                .ok_or("missing unit")?;
            let instance = scene.assembly.instance(id).ok_or("unit instance")?;
            assert_eq!(instance.placement(), &e.extent.placement());
            assert!(
                instance
                    .metadata()
                    .iter()
                    .any(|(key, value)| key == "generated_by" && value == "lay-gate-wall")
            );
        }
        assert_outlets_clear(&scene.assembly, &compiled);
        let before = compiler.counters();
        let id = scene
            .assembly
            .instances_with_ids()
            .next()
            .ok_or("empty scene")?
            .0;
        scene.assembly.bind_material(id, "surface", "brick.4")?;
        compiler.compile_parts(&scene.assembly, &policy())?;
        assert_eq!(compiler.counters().parts_compiled, before.parts_compiled);
    }
    Ok(())
}

#[test]
fn composed_assemblies_preserve_child_frames_bindings_and_metadata() -> Result<()> {
    let mut source = Assembly::new();
    let part = source.add_recipe_part("block", geometry::block([1.0; 3], 0.0)?)?;
    source.set_default_slot(part, "surface")?;
    source.set_part_material(part, "surface", "stone")?;
    let root = source.add_instance(None, "root", part, Placement3::translate(1.0, 0.0, 0.0))?;
    let child = source.add_instance(
        Some(root),
        "child",
        part,
        Placement3::translate(0.0, 2.0, 0.0),
    )?;
    source.bind_material(child, "surface", "plaster")?;
    source.set_metadata(child, "role", "detail")?;
    let mut scene = Scene::default();
    scene.assembly.append(
        None,
        &source,
        "copied",
        Placement3::translate(3.0, 4.0, 5.0),
    )?;
    let root = &scene.assembly.instances()[0];
    let child = &scene.assembly.instances()[1];
    assert_eq!(root.placement(), &Placement3::translate(4.0, 4.0, 5.0));
    assert_eq!(child.placement(), &Placement3::translate(0.0, 2.0, 0.0));
    assert_eq!(child.parent(), Some(InstanceId(0)));
    assert_eq!(root.key(), "copied-root");
    assert_eq!(child.key(), "child");
    assert_eq!(
        child.metadata(),
        &[("role".to_owned(), "detail".to_owned())]
    );
    assert_eq!(child.bindings()[0].1, "plaster");
    Ok(())
}

// Rays through the wall must see daylight along both channel interiors,
// including where a channel crosses the buried curved surround.
fn assert_outlets_clear(assembly: &Assembly, compiled: &CompiledParts) {
    for instance in assembly
        .instances()
        .iter()
        .filter(|i| i.key().starts_with("entry-"))
    {
        let part = compiled.part(instance.part().unwrap()).unwrap();
        for body in &part.bodies {
            for triangle in body.tri.indices.chunks_exact(3) {
                let points: Vec<_> = triangle
                    .iter()
                    .map(|i| {
                        let p = body.tri.positions[*i as usize].map(f64::from);
                        let frame = instance.placement();
                        [0, 2].map(|r| {
                            (0..3).map(|c| frame.rows[r][c] * p[c]).sum::<f64>() + frame.rows[r][3]
                        })
                    })
                    .collect();
                let cross = |a: [f64; 2], b: [f64; 2], p: [f64; 2]| {
                    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
                };
                if cross(points[0], points[1], points[2]).abs() < 1e-12 {
                    continue;
                }
                for x in [0.35, 0.40, 0.45, 4.82, 4.87, 4.92] {
                    for z in [-0.17, -0.14, -0.10] {
                        let sides =
                            [0, 1, 2].map(|i| cross(points[i], points[(i + 1) % 3], [x, z]));
                        let blocked =
                            sides.iter().all(|v| *v >= 0.0) || sides.iter().all(|v| *v <= 0.0);
                        assert!(!blocked, "{} blocks channel at {x}, {z}", instance.key());
                    }
                }
            }
        }
    }
}

#[test]
fn wall_cap_tiles_drain_outward_on_both_sides() -> Result<()> {
    let mut scene = Scene::default();
    wall_caps::build(
        &mut scene,
        "test-cap",
        1.0,
        Placement3::IDENTITY,
        wall_caps::Ends::SQUARE,
    )?;
    let mut checked = 0;
    for instance in scene.assembly.instances() {
        if scene.assembly.part(instance.part().unwrap()).unwrap().key() != "test-cap" {
            continue;
        }
        let frame = instance.placement();
        // Midpoints along a straight cover must get lower toward the eave.
        let y = frame.rows[1][3] + frame.rows[1][1] * 0.19;
        assert!(
            y * frame.rows[1][1] * frame.rows[2][1] < 0.0,
            "tile rises outward: {:?}",
            frame.rows
        );
        checked += 1;
    }
    assert!(checked > 0);
    Ok(())
}

#[test]
fn retained_courtyard_moves_as_one_frame_without_recompiling_its_modules() -> Result<()> {
    use exedra_assembly::compose;
    use exedra_gltf::GlbDocument;
    use std::sync::Arc;

    let placement = Placement3::rotate_z_then_translate(0.35, 7.0, -3.0, 0.2);
    let (original, _) = build(1550, Placement3::IDENTITY)?;
    let (moved, _) = build(1550, placement)?;
    let mut compiler = PartCompiler::new();
    let before = compiler.compile_parts(&original.assembly, &policy())?;
    let counters = compiler.counters();
    let after = compiler.compile_parts(&moved.assembly, &policy())?;
    assert_eq!(compiler.counters().parts_compiled, counters.parts_compiled);
    assert_eq!(
        compiler.counters().triangles_emitted,
        counters.triangles_emitted
    );
    for (a, b) in before.parts().iter().zip(after.parts()) {
        assert!(Arc::ptr_eq(a, b));
    }
    let a = flatten(&original.assembly, &before);
    let b = flatten(&moved.assembly, &after);
    assert_eq!(
        original
            .assembly
            .instances()
            .iter()
            .filter(|i| i.part().is_some())
            .count(),
        5337
    );
    assert_eq!(a.items.len(), 5677);
    assert_eq!(a.items.len(), b.items.len());
    assert_eq!(a.triangle_count(), b.triangle_count());
    for (a, b) in a.items.iter().zip(&b.items) {
        assert_eq!(a.path, b.path);
        let expected = compose(&placement, &a.world);
        for (expected, actual) in expected
            .rows
            .iter()
            .flatten()
            .zip(b.world.rows.iter().flatten())
        {
            assert!(
                (expected - actual).abs() < 1e-12,
                "wrong module pose for {}",
                a.path
            );
        }
    }
    assert!(
        moved
            .assembly
            .parts()
            .iter()
            .all(|p| p.key() != "shelter-ground")
    );
    let export = export_glb_with_materials(
        &moved.assembly,
        &after,
        &material,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    let document = GlbDocument::parse(&export.bytes)?;
    let nodes = document.json()["nodes"].as_array().unwrap();
    let root = nodes
        .iter()
        .find(|n| n["extras"]["instancePath"] == "courtyard")
        .unwrap();
    let children: Vec<_> = root["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| {
            nodes[usize::try_from(id.as_u64().unwrap()).unwrap()]["name"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(
        children,
        [
            "courtyard/entry",
            "courtyard/ground",
            "courtyard/enclosure",
            "courtyard/shelter-pavilion",
            "courtyard/planting",
            "courtyard/rocks"
        ]
    );
    assert!(
        nodes
            .iter()
            .any(|n| n["extras"]["instancePath"] == "courtyard/shelter-pavilion/roof/tiles")
    );
    Ok(())
}
