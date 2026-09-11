// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::Arc;

use exedra_assembly::{PartSource, compose as compose_placements};
use exedra_constructive::ir::{Placement3, Recipe};
use exedra_gltf::GlbDocument;
use exedra_math::{cross, dot, norm, sub};
use joiner::{ElementOrigin, compose, instance_path};

use super::*;

pub(crate) mod solid;

#[test]
fn fitted_instances_keep_their_composed_recipes_and_provenance() -> Result<()> {
    let layout = Layout::resolve(Parameters::default())?;
    let frame = rafters::build(&layout)?;
    let scene = scene::build_with_roof(&layout, &frame)?;
    let assembly = &scene.assembly;
    for element in frame.construction.elements() {
        let id = scene.fitted_instance(&element.key).unwrap();
        let instance = assembly.instance(id).unwrap();
        let PartSource::Recipe(actual) = assembly.part(instance.part().unwrap()).unwrap().source()
        else {
            panic!("recipe")
        };
        assert_eq!(
            actual.recipe_fingerprint(),
            compose(&frame.construction, element)?.recipe_fingerprint(),
            "{} lost its own cuts or sources",
            element.key
        );
        assert_eq!(*instance.placement(), element.extent.placement());
        assert!(
            instance
                .metadata()
                .contains(&("structural_role".into(), element.role.clone()))
        );
        if let ElementOrigin::Generated(application) = &element.origin {
            assert!(
                instance
                    .metadata()
                    .contains(&("generated_by".into(), application.clone()))
            );
        }
    }
    Ok(())
}

#[test]
fn contact_verification_rejects_missing_instances_and_stale_placements_or_parts() -> Result<()> {
    let mut frame = seats::study(&Layout::resolve(Parameters::default())?)?;
    let mut assembly = frame.lower()?;
    let compiled = PartCompiler::new().compile_parts(&assembly, &compile_policy())?;
    assert!(
        frame.verify(&assembly, &compiled, |key| assembly
            .resolve_path(&instance_path(key)))?
            > 0.0
    );
    assert!(
        frame
            .verify(&Assembly::new(), &compiled, |_| None)
            .unwrap_err()
            .to_string()
            .contains("missing fitted instance")
    );

    let key = frame.construction.elements()[0].key.clone();
    let extent = frame.construction.element(&key).unwrap().extent.clone();
    frame
        .construction
        .set_element_extent(&key, extent.translated([0.01, 0.0, 0.0]))?;
    assert!(
        frame
            .verify(&assembly, &compiled, |key| assembly
                .resolve_path(&instance_path(key)))
            .unwrap_err()
            .to_string()
            .contains("different exported placement")
    );
    frame
        .construction
        .set_element_extent(&key, extent.clone())?;

    let id = assembly.resolve_path(&instance_path(&key)).unwrap();
    let part = assembly.instance(id).unwrap().part().unwrap();
    // A box could cover the bearing rectangle while losing every authored cut.
    assembly.replace_part_source(part, PartSource::Recipe(geometry::block(extent.size)?))?;
    let wrong = PartCompiler::new().compile_parts(&assembly, &compile_policy())?;
    assert!(
        frame
            .verify(&assembly, &wrong, |key| assembly
                .resolve_path(&instance_path(key)))
            .unwrap_err()
            .to_string()
            .contains("different exported geometry")
    );
    Ok(())
}

#[test]
fn setout_matches_successive_raise_and_depress_construction() -> Result<()> {
    let l = Layout::resolve(Parameters::default())?;
    let run = l.roof[0][0];
    let eave = l.roof[0][1];
    let rise = l.roof[4][1] - eave;
    assert!(
        (rise - run / 3.0).abs() < 1.0e-12,
        "ridge uses the chosen one-third raise"
    );
    let mut previous = (0.0, rise);
    for (index, depression) in [10.0, 20.0, 40.0].into_iter().enumerate() {
        let station = run * f64::from(u32::try_from(index + 1)?) / 4.0;
        let working_line = previous.1 * (run - station) / (run - previous.0);
        let expected = working_line - rise / depression;
        assert!(
            (l.roof[3 - index][1] - eave - expected).abs() < 1.0e-12,
            "purlin {index}"
        );
        previous = (station, expected);
    }
    assert_eq!(l.frames, [-1.8, 1.8]);
    assert_eq!([l.arm_width, l.arm_depth], [0.15, 0.225]);
    Ok(())
}

#[test]
fn parameter_extremes_produce_sound_geometry_and_shared_parts() -> Result<()> {
    for size in [2_400, 3_601, 5_400] {
        let mut counts = Vec::new();
        for bays in [1, 5] {
            let l = Layout::resolve(Parameters {
                bays,
                span_mm: size,
                depth_mm: size,
            })?;
            assert_eq!(l.frames.len(), usize::try_from(bays + 1)?);
            let roof_frame = rafters::build(&l)?;
            let scene = scene::build_with_roof(&l, &roof_frame)?;
            let assembly = &scene.assembly;
            let compiled = PartCompiler::new().compile_parts(assembly, &compile_policy())?;
            check_reports(assembly, &compiled)?;
            assert!(scene.verify(&roof_frame, &compiled)? > 0.0);
            counts.push((
                assembly.parts().len(),
                compiled
                    .parts()
                    .iter()
                    .map(|p| p.triangle_count())
                    .sum::<u64>(),
            ));
            for (definition, part) in assembly.parts().iter().zip(compiled.parts()) {
                assert!(!part.bodies.is_empty(), "no silently omitted part");
                for body in &part.bodies {
                    let mesh = &body.tri;
                    for triangle in mesh.indices.chunks_exact(3) {
                        let [a, b, c] = [triangle[0], triangle[1], triangle[2]]
                            .map(|i| mesh.positions[i as usize].map(f64::from));
                        assert!(
                            norm(cross(sub(b, a), sub(c, a))) > 1.0e-12,
                            "nondegenerate exported triangles: size={size}, bays={bays}, part={:?}, triangle={a:?} {b:?} {c:?}",
                            part.fingerprint
                        );
                    }
                    for normal in &mesh.normals {
                        assert!(
                            (norm(normal.map(f64::from)) - 1.0).abs() < 1.0e-5,
                            "finite unit normals: size={size}, bays={bays}, part={}, normal={normal:?}",
                            definition.key()
                        );
                    }
                }
            }
        }
        assert_eq!(
            counts[0].0, counts[1].0,
            "bay repetition keeps the same shared part families"
        );
        assert!(
            counts[1].1 > counts[0].1,
            "longer purlins contain more real seats"
        );
    }
    assert!(
        Layout::resolve(Parameters {
            bays: 0,
            ..Parameters::default()
        })
        .is_err()
    );
    assert!(
        Layout::resolve(Parameters {
            depth_mm: 100,
            ..Parameters::default()
        })
        .is_err()
    );
    Ok(())
}

#[test]
fn material_reassignment_keeps_geometry_and_exports_repeatably() -> Result<()> {
    let mut assembly = scene::build(&Layout::resolve(Parameters::default())?)?;
    let mut compiler = PartCompiler::new();
    let before = compiler.compile_parts(&assembly, &compile_policy())?;
    let counters = compiler.counters();
    reassign_materials(&mut assembly)?;
    let after = compiler.compile_parts(&assembly, &compile_policy())?;
    assert_eq!(counters.parts_compiled, compiler.counters().parts_compiled);
    assert_eq!(
        counters.triangles_emitted,
        compiler.counters().triangles_emitted
    );
    for (a, b) in before.parts().iter().zip(after.parts()) {
        assert!(
            Arc::ptr_eq(a, b),
            "the compiled allocation itself is reused"
        );
    }
    let export = || {
        export_glb_with_materials(
            &assembly,
            &after,
            &material,
            GltfExportOptions::z_up_to_y_up(),
        )
    };
    let first = export()?;
    assert_eq!(first.bytes, export()?.bytes, "deterministic scene export");
    let document = GlbDocument::parse(&first.bytes)?;
    assert!(document.material_names().contains(&"paint.vermilion"));
    assert!(document.material_names().contains(&"glaze.green"));
    assert!(
        first.stats.meshes < first.stats.nodes / 10,
        "repeated members and tiles share glTF meshes"
    );
    Ok(())
}

pub(super) fn recipe_volume(recipe: Recipe) -> Result<f64> {
    let mut assembly = Assembly::new();
    assembly.add_recipe_part("volume", recipe)?;
    let compiled = PartCompiler::new().compile_parts(&assembly, &compile_policy())?;
    check_reports(&assembly, &compiled)?;
    let mut volume = 0.0;
    for part in compiled.parts() {
        for body in &part.bodies {
            for t in body.tri.indices.chunks_exact(3) {
                let [a, b, c] =
                    [t[0], t[1], t[2]].map(|i| body.tri.positions[i as usize].map(f64::from));
                volume += dot(a, cross(b, c)) / 6.0;
            }
        }
    }
    Ok(volume)
}

#[test]
fn separating_roof_members_reuses_geometry_and_exposes_assembled_contacts() -> Result<()> {
    let layout = Layout::resolve(Parameters::default())?;
    let roof = rafters::build(&layout)?;
    let assembled = scene::build_with_roof(&layout, &roof)?;
    let exploded = scene::build_with_separation(&layout, &roof, 0.9)?;
    let mut compiler = PartCompiler::new();
    let before = compiler.compile_parts(&assembled.assembly, &compile_policy())?;
    let counters = compiler.counters();
    let after = compiler.compile_parts(&exploded.assembly, &compile_policy())?;
    assert_eq!(compiler.counters().parts_compiled, counters.parts_compiled);
    assert_eq!(
        compiler.counters().triangles_emitted,
        counters.triangles_emitted
    );
    assert!(assembled.verify(&roof, &before)? > 0.0);
    assert!(
        exploded
            .verify(&roof, &after)
            .unwrap_err()
            .to_string()
            .contains("different exported placement")
    );
    for (before, after) in before.parts().iter().zip(after.parts()) {
        assert!(Arc::ptr_eq(before, after));
    }
    let a = flatten(&assembled.assembly, &before);
    let b = flatten(&exploded.assembly, &after);
    assert_eq!(a.triangle_count(), b.triangle_count());
    assert_eq!(a.items.len(), b.items.len());
    for (a, b) in a.items.iter().zip(&b.items) {
        assert_eq!(a.path, b.path);
        let path = a.path.to_string();
        let mut offset = [0.0, 0.0, 0.0];
        if path.starts_with("pavilion/roof/") {
            offset[2] = 0.45;
            if path.contains("/tiles/") {
                offset[2] += 3.24;
            } else if path.contains("/purlins/") {
                offset[2] += 0.9;
            } else if path.contains("/pins/") {
                offset[0] = 0.27;
                offset[2] += 2.34;
            } else if path.contains("/rafters/") {
                let segment: u32 = path
                    .split("course-")
                    .nth(1)
                    .unwrap()
                    .split('/')
                    .next()
                    .unwrap()
                    .parse()?;
                offset[0] = if segment.is_multiple_of(2) {
                    -0.09
                } else {
                    0.09
                };
                offset[1] = (f64::from(segment) - 3.5) * 0.126;
                offset[2] += 1.8 + f64::from(segment % 2) * 0.135;
            }
        }
        let expected = compose_placements(
            &Placement3::translate(offset[0], offset[1], offset[2]),
            &a.world,
        );
        for (expected, actual) in expected
            .rows
            .iter()
            .flatten()
            .zip(b.world.rows.iter().flatten())
        {
            assert!(
                (expected - actual).abs() < 1e-12,
                "wrong layer pose for {path}"
            );
        }
    }
    let export = export_glb_with_materials(
        &exploded.assembly,
        &after,
        &material,
        GltfExportOptions::z_up_to_y_up(),
    )?;
    let document = GlbDocument::parse(&export.bytes)?;
    for path in [
        "pavilion",
        "pavilion/foundation",
        "pavilion/timber",
        "pavilion/roof",
        "pavilion/roof/tiles",
    ] {
        let node = document.json()["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["extras"]["instancePath"] == path)
            .unwrap();
        assert!(node.get("mesh").is_none());
        assert!(!node["children"].as_array().unwrap().is_empty());
    }
    Ok(())
}
