// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Filled-region arrangement wind tunnel with geometry identity checked first.

use exedra_profile::{
    Curve, Loop, Profile, ProfilePolicy, RegionOp, RegionPolicy, RegionResult, Segment,
};
use std::{hint::black_box, time::Instant};

const POLICY: RegionPolicy = RegionPolicy::new(ProfilePolicy::new(1e-8));

fn rectangle(x: f64, y: f64) -> Profile {
    Profile::new(
        Loop::new(vec![
            Segment::line([x, -y]),
            Segment::line([x, y]),
            Segment::line([-x, y]),
            Segment::line([-x, -y]),
        ])
        .unwrap(),
        vec![],
        &POLICY.profile,
    )
    .unwrap()
}

fn radial(count: u32, offset: f64, phase: f64) -> Profile {
    let segments = (0..count)
        .map(|i| {
            let angle = std::f64::consts::TAU * f64::from(i) / f64::from(count);
            let radius = 12.0 + 1.5 * (6.0 * angle + phase).sin();
            Segment::line([offset + radius * angle.cos(), radius * angle.sin()])
        })
        .collect();
    Profile::new(Loop::new(segments).unwrap(), vec![], &POLICY.profile).unwrap()
}

fn signature(result: &RegionResult) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut add = |value: u64| {
        hash = (hash ^ value).wrapping_mul(0x0000_0100_0000_01b3);
    };
    for profile in &result.profiles {
        for loop_ in std::iter::once(profile.outer()).chain(profile.holes()) {
            add(loop_.segments().len() as u64);
            for segment in loop_.segments() {
                for coordinate in segment.to {
                    add(coordinate.to_bits());
                }
                add(match segment.curve {
                    Curve::Line => 0,
                    Curve::Arc { bulge } => bulge.to_bits(),
                    Curve::ArcCenter { center, clockwise } => {
                        center[0].to_bits()
                            ^ center[1].to_bits().rotate_left(19)
                            ^ u64::from(clockwise)
                    }
                });
                add(segment.tag.map_or(u64::MAX, |tag| u64::from(tag.0)));
            }
        }
    }
    hash
}

fn run(name: &str, left: &Profile, right: &Profile, iterations: u32) {
    let first = left.combine(right, RegionOp::Union, &POLICY).unwrap();
    let repeat = left.combine(right, RegionOp::Union, &POLICY).unwrap();
    assert_eq!(
        first, repeat,
        "geometry and correspondence must be deterministic before timing"
    );
    let signature = signature(&first);
    for _ in 0..4 {
        black_box(left.combine(right, RegionOp::Union, &POLICY).unwrap());
    }
    let mut samples = Vec::with_capacity(7);
    for _ in 0..7 {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(left.combine(right, RegionOp::Union, &POLICY).unwrap());
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / f64::from(iterations));
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "scenario={name} segments={} events={} fragments={} classifications={} output_segments={} signature={signature:016x} median_us={:.3} iterations={iterations}",
        left.outer().segments().len() + right.outer().segments().len(),
        first.report.events,
        first.report.fragments,
        first.report.classification_tests,
        first.report.retained_segments,
        samples[3]
    );
}

fn main() {
    let multiplier = std::env::args().nth(1).map_or(1, |v| {
        v.parse::<u32>().expect("positive repetition multiplier")
    });
    assert!(multiplier > 0, "repetition multiplier must be positive");
    run(
        "cross",
        &rectangle(20.0, 5.0),
        &rectangle(5.0, 20.0),
        200 * multiplier,
    );
    for (count, iterations) in [(48, 30), (192, 5), (384, 2)] {
        run(
            &format!("radial_{count}"),
            &radial(count, 0.0, 0.0),
            &radial(count, 0.35, 1.0),
            iterations * multiplier,
        );
    }
}
