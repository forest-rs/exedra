// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::rc::Rc;

use exedra_constructive::ir::Recipe;
use exedra_gltf::GlbDocument;
use exedra_math::{cross, dot, norm, sub};

use super::*;

pub(crate) mod solid;

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
            let assembly = scene::build_with_roof(&l, &roof_frame)?;
            let compiled = PartCompiler::new().compile_parts(&assembly, &compile_policy())?;
            check_reports(&assembly, &compiled)?;
            assert!(roof_frame.verify(&assembly, &compiled)? > 0.0);
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
        assert!(Rc::ptr_eq(a, b), "the compiled allocation itself is reused");
    }
    let draw = flatten(&assembly, &after);
    let export = || {
        export_glb_with_materials(
            &assembly,
            &after,
            &draw,
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
