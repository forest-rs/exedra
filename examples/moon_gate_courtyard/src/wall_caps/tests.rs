// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::policy;
use exedra_assembly::PartCompiler;
use exedra_constructive::evaluate::Severity;

type Triangle = [[f64; 3]; 3];

#[test]
fn emitted_caps_respect_their_square_and_mitred_boundaries() -> Result<()> {
    for (length, ends) in [
        (1.031, Ends::SQUARE),
        (
            9.31,
            Ends {
                start: 0.0,
                end: -1.0,
            },
        ),
        (
            9.31,
            Ends {
                start: 0.0,
                end: 1.0,
            },
        ),
        (
            10.68,
            Ends {
                start: -1.0,
                end: 1.0,
            },
        ),
    ] {
        let mut scene = Scene::default();
        build(&mut scene, "cap", length, Placement3::IDENTITY, ends)?;
        let compiled = PartCompiler::new().compile_parts(&scene.assembly, &policy())?;
        for instance in scene.assembly.instances() {
            let id = instance.part().unwrap();
            let part = compiled.part(id).unwrap();
            assert!(
                part.triangle_count() > 0,
                "empty cap tile: {}",
                instance.key()
            );
            assert!(compiled.report(id).unwrap().clean_at(Severity::Warning));
            for body in &part.bodies {
                for p in &body.tri.positions {
                    let [x, y, z] = placed(p.map(f64::from), instance.placement());
                    assert!(
                        x >= ends.start * y - 2e-6 && x <= length + ends.end * y + 2e-6,
                        "{} crosses its corner boundary at {x}, {y}",
                        instance.key()
                    );
                    assert!((-0.031..=0.331).contains(&z));
                }
            }
        }
    }
    Ok(())
}

#[test]
fn pan_cover_and_ridge_cover_the_coping_without_clay_interference() -> Result<()> {
    let mut scene = Scene::default();
    build(&mut scene, "cap", 1.0, Placement3::IDENTITY, Ends::SQUARE)?;
    let compiled = PartCompiler::new().compile_parts(&scene.assembly, &policy())?;
    let mut solids = Vec::new();
    for instance in scene.assembly.instances() {
        for body in &compiled.part(instance.part().unwrap()).unwrap().bodies {
            let triangles: Vec<Triangle> = body
                .tri
                .indices
                .chunks_exact(3)
                .map(|t| {
                    [t[0], t[1], t[2]].map(|i| {
                        placed(
                            body.tri.positions[i as usize].map(f64::from),
                            instance.placement(),
                        )
                    })
                })
                .collect();
            solids.push((instance.key(), triangles));
        }
    }
    for side in [-1.0, 1.0] {
        for i in 0..32 {
            for j in 0..16 {
                let p = [
                    0.1123 + f64::from(i) * 0.0137,
                    side * (0.0731 + f64::from(j) * 0.0187),
                ];
                let mut intervals = Vec::new();
                for (key, triangles) in &solids {
                    let mut hits: Vec<_> = triangles
                        .iter()
                        .filter_map(|t| vertical_hit(t, p))
                        .collect();
                    hits.sort_by(f64::total_cmp);
                    hits.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
                    assert!(
                        hits.len().is_multiple_of(2),
                        "open shell at {key}, {p:?}: {hits:?}"
                    );
                    for pair in hits.chunks_exact(2) {
                        intervals.push((pair[0], pair[1], key));
                    }
                }
                intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
                assert!(intervals.len() >= 2, "uncovered coping at {p:?}");
                for pair in intervals.windows(2) {
                    assert!(
                        pair[0].1 <= pair[1].0 + 1e-6,
                        "solid overlap at {p:?}: {:?}",
                        pair
                    );
                }
            }
        }
    }
    Ok(())
}

fn placed(p: [f64; 3], frame: &Placement3) -> [f64; 3] {
    frame
        .rows
        .map(|r| r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + r[3])
}
fn vertical_hit(t: &Triangle, p: [f64; 2]) -> Option<f64> {
    let cross = |a: [f64; 3], b: [f64; 3], c: [f64; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let area = cross(t[0], t[1], [t[2][0], t[2][1]]);
    if area.abs() < 1e-12 {
        return None;
    }
    let u = cross(t[1], t[2], p) / area;
    let v = cross(t[2], t[0], p) / area;
    let w = 1.0 - u - v;
    (u >= 0.0 && v >= 0.0 && w >= 0.0).then(|| u * t[0][2] + v * t[1][2] + w * t[2][2])
}
