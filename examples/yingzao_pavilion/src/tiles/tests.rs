// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_assembly::{Assembly, CompiledPart, PartCompiler};
use exedra_math::{cross, dot, norm, sub};

use super::*;
use crate::layout::Parameters;
use crate::tests::solid::Solid;
use crate::{check_reports, compile_policy};

/// Sample just inside the actual tessellated faces, including near each corner.
/// This exercises clay thickness, rather than testing ideal circular surfaces.
fn clay_samples(part: &CompiledPart) -> Vec<[f64; 3]> {
    let own = Solid::new(part, Placement3::IDENTITY);
    let mut samples = Vec::new();
    for body in &part.bodies {
        for t in body.tri.indices.chunks_exact(3) {
            let [a, b, c] =
                [t[0], t[1], t[2]].map(|i| body.tri.positions[i as usize].map(f64::from));
            let n = cross(sub(b, a), sub(c, a));
            let length = norm(n);
            assert!(length > 1.0e-12);
            for weights in [
                [1.0 / 3.0; 3],
                [0.6, 0.2, 0.2],
                [0.2, 0.6, 0.2],
                [0.2, 0.2, 0.6],
            ] {
                let point = core::array::from_fn(|i| {
                    a[i] * weights[0] + b[i] * weights[1] + c[i] * weights[2]
                        - 0.00015 * n[i] / length
                });
                assert!(
                    own.contains(point),
                    "outward, closed tile shell at {point:?}"
                );
                samples.push(point);
            }
        }
    }
    samples
}

fn placed(samples: &[[f64; 3]], placement: Placement3) -> Vec<[f64; 3]> {
    samples
        .iter()
        .map(|&p| placement.rows.map(|r| dot([r[0], r[1], r[2]], p) + r[3]))
        .collect()
}

#[test]
fn tile_laps_clear_the_clay_and_decking_across_pitch_changes() -> Result<()> {
    for depth_mm in [2_400, 2_700, 3_601, 4_500, 5_400] {
        let section = RoofSection::new(&Layout::resolve(Parameters {
            depth_mm,
            ..Parameters::default()
        })?);
        let mut assembly = Assembly::new();
        let cap = assembly.add_recipe_part("cap", shell(true, CAP_RADIUS, LENGTH)?)?;
        let pan = assembly.add_recipe_part("pan", shell(false, PAN_RADIUS, LENGTH)?)?;
        let deck = assembly.add_recipe_part("deck", section.decking(0.8)?)?;
        let compiled = PartCompiler::new().compile_parts(&assembly, &compile_policy())?;
        check_reports(&assembly, &compiled)?;
        let deck = Solid::new(compiled.part(deck).unwrap(), section.placement(-1.0));
        let mut lanes = Vec::new();
        for (name, part, spacing, lift, x, projection) in [
            ("cap", cap, 0.245, CAP_LIFT, 0.0, CAP_EAVE_PROJECTION),
            // Tightest column spacing near the accepted minimum roof width.
            ("pan", pan, 0.230, PAN_LIFT, 0.106, PAN_EAVE_PROJECTION),
        ] {
            let part = compiled.part(part).unwrap();
            let samples = clay_samples(part);
            let courses = courses(&section, spacing, projection);
            let mut solids = Vec::new();
            for (i, course) in courses.iter().enumerate() {
                let placement = course.world(-1.0, section.run(), x, lift);
                let samples = placed(&samples, placement);
                assert!(
                    samples.iter().all(|&p| !deck.contains(p)),
                    "depth={depth_mm}, {name} course {i} penetrates decking"
                );
                solids.push((Solid::new(part, placement), samples));
            }
            for (i, pair) in solids.windows(2).enumerate() {
                for (a, b) in [(&pair[0], &pair[1]), (&pair[1], &pair[0])] {
                    for &p in &a.1 {
                        assert!(
                            !b.0.contains(p),
                            "depth={depth_mm}, {name} lap {i} intersects clay at {p:?}"
                        );
                    }
                }
                assert!(
                    pair[0].0.bounds.max[1] > pair[1].0.bounds.min[1] + 0.07,
                    "real lap length, not separated tiles"
                );
            }
            lanes.push(solids);
        }
        // A cover sits over the edges of the adjacent drainage channels.
        for (i, cap) in lanes[0].iter().enumerate() {
            for (j, pan) in lanes[1].iter().enumerate() {
                for (a, b) in [(cap, pan), (pan, cap)] {
                    for &p in &a.1 {
                        assert!(
                            !b.0.contains(p),
                            "depth={depth_mm}, cap {i}/pan {j} interference at {p:?}"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn courses_finish_at_the_ridge_and_do_not_restart_at_purlins() -> Result<()> {
    let section = RoofSection::new(&Layout::resolve(Parameters::default())?);
    for (spacing, projection) in [(0.230, PAN_EAVE_PROJECTION), (0.245, CAP_EAVE_PROJECTION)] {
        let courses = courses(&section, spacing, projection);
        let last = courses.last().unwrap();
        let end = [
            last.start[0] + LENGTH * last.normal[1],
            last.start[1] - LENGTH * last.normal[0],
        ];
        let ridge = section.layer(DECK_TOP)[4];
        assert!((end[0] - ridge[0]).hypot(end[1] - ridge[1]) < 1.0e-12);
        for joint in &section.layer(DECK_TOP)[1..4] {
            assert!(
                courses
                    .iter()
                    .all(|c| (c.start[0] - joint[0]).abs() > 0.001)
            );
            assert!(
                courses
                    .iter()
                    .any(|c| c.start[0] < joint[0] && c.start[0] + LENGTH * c.normal[1] > joint[0])
            );
        }
    }
    Ok(())
}

#[test]
fn ridge_shells_nest_and_clear_the_last_roof_courses() -> Result<()> {
    let section = RoofSection::new(&Layout::resolve(Parameters::default())?);
    let mut assembly = Assembly::new();
    let ridge = assembly.add_recipe_part("ridge", shell(true, RIDGE_RADIUS, LENGTH)?)?;
    let cap = assembly.add_recipe_part("cap", shell(true, CAP_RADIUS, LENGTH)?)?;
    let pan = assembly.add_recipe_part("pan", shell(false, PAN_RADIUS, LENGTH)?)?;
    let compiled = PartCompiler::new().compile_parts(&assembly, &compile_policy())?;
    check_reports(&assembly, &compiled)?;
    let ridge = compiled.part(ridge).unwrap();
    let ridge_samples = clay_samples(ridge);
    let z = section.layer(DECK_TOP)[4][1] + RIDGE_LIFT;
    let ridges: Vec<_> = [0.0, 0.24]
        .into_iter()
        .map(|x| {
            let placement = Placement3::from_axes(
                [0.0, 1.0, 0.0],
                [-1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [x, 0.0, z],
            );
            (
                Solid::new(ridge, placement),
                placed(&ridge_samples, placement),
            )
        })
        .collect();
    for (a, b) in [(&ridges[0], &ridges[1]), (&ridges[1], &ridges[0])] {
        assert!(
            a.1.iter().all(|&p| !b.0.contains(p)),
            "ridge tile lap intersects"
        );
    }
    for side in [-1.0, 1.0] {
        for (part, lift, spacing, projection) in [
            (cap, CAP_LIFT, 0.245, CAP_EAVE_PROJECTION),
            (pan, PAN_LIFT, 0.230, PAN_EAVE_PROJECTION),
        ] {
            let part = compiled.part(part).unwrap();
            let course = *courses(&section, spacing, projection).last().unwrap();
            let placement = course.world(side, section.run(), 0.0, lift);
            let roof = Solid::new(part, placement);
            let samples = placed(&clay_samples(part), placement);
            for ridge in &ridges {
                for &p in &samples {
                    assert!(
                        !ridge.0.contains(p),
                        "roof clay penetrates ridge at {p:?}, side={side}, lift={lift}"
                    );
                }
                for &p in &ridge.1 {
                    assert!(
                        !roof.contains(p),
                        "ridge clay penetrates roof at {p:?}, side={side}, lift={lift}"
                    );
                }
            }
        }
    }
    Ok(())
}
