// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::{array::from_fn, f64::consts::TAU};

use exedra_assembly::{Assembly, CompiledPart, CompiledParts, PartCompiler};
use joiner::{ContactMeaning, compose, instance_path, measure_contact_geometry};

use super::{WIDTH, build, purlin_key, study};
use crate::joinery::FittedConstruction;
use crate::layout::{Layout, Parameters};
use crate::tests::solid::Solid;
use crate::{Result, check_reports, compile_policy};

#[test]
fn repeated_lanes_have_identical_local_cut_recipes() -> Result<()> {
    let frame = build(&Layout::resolve(Parameters {
        bays: 5,
        ..Parameters::default()
    })?)?;
    for segment in 0..8 {
        let family = format!("roof-rafter-{segment}");
        let mut recipes = frame
            .families
            .iter()
            .filter(|(_, f)| f == &family)
            .map(|(key, _)| {
                compose(
                    &frame.construction,
                    frame.construction.element(key).unwrap(),
                )
                .unwrap()
                .recipe_fingerprint()
            });
        let first = recipes.next().unwrap();
        assert!(
            recipes.all(|recipe| recipe == first),
            "{family} changed with its lane placement"
        );
    }
    Ok(())
}

struct Fixture {
    frame: FittedConstruction,
    assembly: Assembly,
    compiled: CompiledParts,
}

impl Fixture {
    fn new(depth_mm: u32) -> Result<Self> {
        let layout = Layout::resolve(Parameters {
            depth_mm,
            ..Parameters::default()
        })?;
        let frame = study(&layout)?;
        let assembly = frame.lower()?;
        let compiled = PartCompiler::new().compile_parts(&assembly, &compile_policy())?;
        check_reports(&assembly, &compiled)?;
        Ok(Self {
            frame,
            assembly,
            compiled,
        })
    }

    fn part(&self, key: &str) -> &CompiledPart {
        let instance = self.assembly.resolve_path(&instance_path(key)).unwrap();
        self.compiled
            .part(self.assembly.instance(instance).unwrap().part().unwrap())
            .unwrap()
    }

    fn solid(&self, key: &str) -> Solid {
        let element = self.frame.construction.element(key).unwrap();
        Solid::new(self.part(key), element.extent.placement())
    }
}

#[test]
fn every_rafter_has_two_full_bearings_and_each_lap_has_real_side_contact() -> Result<()> {
    for depth in [2_400, 3_601, 5_400] {
        let fixture = Fixture::new(depth)?;
        assert!(
            fixture
                .frame
                .verify(&fixture.assembly, &fixture.compiled, |key| fixture
                    .assembly
                    .resolve_path(&instance_path(key)))?
                > 0.0
        );
        let mut bearings = 0;
        let mut laps = 0;
        for contact in fixture
            .frame
            .construction
            .contacts()
            .iter()
            .filter(|c| c.key.starts_with("rafter-"))
        {
            let measured = measure_contact_geometry(
                &fixture.frame.construction,
                contact,
                fixture.part(&contact.carried.element),
                fixture.part(&contact.carrier.element),
                1.0e-5,
            )?;
            assert!(
                measured.is_covered(),
                "depth={depth}, {}: {measured:?}",
                contact.key
            );
            match contact.meaning {
                ContactMeaning::Bearing => bearings += 1,
                ContactMeaning::SideFit => laps += 1,
                _ => panic!("unexpected rafter contact"),
            }
        }
        assert_eq!(
            (bearings, laps),
            (16, 7),
            "two bearings per rafter, one lap per pitch change"
        );
    }
    Ok(())
}

#[test]
fn all_pins_clear_both_lap_halves_and_leave_wood_around_the_bores() -> Result<()> {
    for depth in [2_400, 3_601, 5_400] {
        let fixture = Fixture::new(depth)?;
        for joint in 1..8 {
            let pin_key = format!("rafter-pin-0-{joint}");
            let pin_element = fixture.frame.construction.element(&pin_key).unwrap();
            let center = pin_element
                .extent
                .anchor(pin_element.extent.size.map(|v| v * 0.5));
            let pin = fixture.solid(&pin_key);
            let halves = [
                fixture.solid(&format!("rafter-0-{}", joint - 1)),
                fixture.solid(&format!("rafter-0-{joint}")),
            ];
            for x in [-0.03, -0.01, 0.01, 0.03] {
                for angle in 0..16 {
                    let theta = (f64::from(angle) + 0.37) * TAU / 16.0;
                    let point = |radius| {
                        [
                            center[0] + x,
                            center[1] + radius * theta.cos(),
                            center[2] + radius * theta.sin(),
                        ]
                    };
                    assert!(
                        pin.contains(point(0.005)),
                        "pin must actually span both halves"
                    );
                    let clearance = point(0.0062);
                    assert!(
                        !pin.contains(clearance) && halves.iter().all(|h| !h.contains(clearance)),
                        "depth={depth}, joint={joint}: pin clearance blocked at {clearance:?}"
                    );
                    let surround = point(0.008);
                    assert!(
                        halves[usize::from(x > 0.0)].contains(surround),
                        "depth={depth}, joint={joint}: bore broke out of its lap at {surround:?}"
                    );
                    assert!(
                        !halves[usize::from(x < 0.0)].contains(surround),
                        "opposite lap half interferes at {surround:?}"
                    );
                }
            }
            for x in [-WIDTH * 0.5 - 0.004, WIDTH * 0.5 + 0.004] {
                let point = [center[0] + x, center[1], center[2]];
                assert!(
                    pin.contains(point) && halves.iter().all(|h| !h.contains(point)),
                    "pin ends must protrude outside the timber"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn fitted_rafters_do_not_interpenetrate_their_neighbors_or_purlins() -> Result<()> {
    for depth in [2_400, 3_601, 5_400] {
        let fixture = Fixture::new(depth)?;
        let rafters: Vec<_> = (0..8)
            .map(|i| fixture.solid(&format!("rafter-0-{i}")))
            .collect();
        let purlins: Vec<_> = (0..9).map(|i| fixture.solid(&purlin_key(i))).collect();
        let mut sampled_pairs = 0;
        for (i, rafter) in rafters.iter().enumerate() {
            for (j, purlin) in purlins.iter().enumerate() {
                sampled_pairs += sample_disjoint(
                    rafter,
                    purlin,
                    &format!("depth={depth}, rafter={i}, purlin={j}"),
                );
            }
            for (j, other) in rafters.iter().enumerate().skip(i + 1) {
                sampled_pairs +=
                    sample_disjoint(rafter, other, &format!("depth={depth}, rafters={i}/{j}"));
            }
        }
        assert!(
            sampled_pairs >= 7,
            "exercise at least all seven side-lap overlaps"
        );
    }
    Ok(())
}

/// Finite sampling catches volume interference; the separate surface-coverage
/// test checks complete contact rectangles. These samples are not a collision proof.
fn sample_disjoint(a: &Solid, b: &Solid, context: &str) -> usize {
    let min: [f64; 3] = from_fn(|i| a.bounds.min[i].max(b.bounds.min[i]));
    let max: [f64; 3] = from_fn(|i| a.bounds.max[i].min(b.bounds.max[i]));
    if (0..3).any(|i| max[i] - min[i] < 1.0e-6) {
        return 0;
    }
    for x in 0..5 {
        for y in 0..11 {
            for z in 0..9 {
                let fractions = [
                    (f64::from(x) + 0.37) / 5.0,
                    (f64::from(y) + 0.41) / 11.0,
                    (f64::from(z) + 0.43) / 9.0,
                ];
                let point = from_fn(|i| min[i] + (max[i] - min[i]) * fractions[i]);
                assert!(
                    !(a.contains(point) && b.contains(point)),
                    "{context}: interference at {point:?}"
                );
            }
        }
    }
    1
}
