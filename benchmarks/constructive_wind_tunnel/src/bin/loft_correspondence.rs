// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact correspondence preparation, separate from subsequent loft meshing.

use exedra_constructive::{
    builders,
    loft::{CorrespondencePolicy, CorrespondenceProfile, correspond_profiles},
};
use exedra_profile::ProfilePolicy;
use std::{hint::black_box, time::Instant};

fn main() {
    let profiles = [
        builders::circle(35.0).unwrap(),
        builders::rounded_rect_centered(86.0, 60.0, 6.0).unwrap(),
        builders::rect_centered(80.0, 56.0).unwrap(),
    ];
    let inputs: Vec<_> = profiles.iter().map(CorrespondenceProfile::new).collect();
    let policy = CorrespondencePolicy::new(ProfilePolicy::new(1e-7));
    let warm = correspond_profiles(&inputs, &policy).unwrap();
    assert_eq!(
        warm,
        correspond_profiles(&inputs, &policy).unwrap(),
        "preparation must be deterministic before timing"
    );
    let mut samples = Vec::with_capacity(101);
    for _ in 0..101 {
        let start = Instant::now();
        let output = correspond_profiles(&inputs, &policy).unwrap();
        black_box(&output);
        let elapsed = start.elapsed().as_nanos();
        drop(output);
        samples.push(elapsed);
    }
    println!(
        "{}",
        serde_json::json!({"input_segments":warm.report.input_segments,"output_segments":warm.report.output_segments,"loops":warm.report.loops,"max_knot_adjustment":warm.report.max_knot_adjustment,"samples_ns":samples})
    );
}
