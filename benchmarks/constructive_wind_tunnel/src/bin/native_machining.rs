// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic retained-native through-cylinder machining wind tunnel.
use exedra_analytic::{
    extrusion::Extrusion,
    machining::{CylinderTool, MachiningPolicy, subtract_through_cylinder_with_roles},
};
use exedra_math::Placement3;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
use std::{hint::black_box, time::Instant};

fn main() {
    let repetitions = std::env::args()
        .nth(1)
        .map(|s| s.parse::<u32>().expect("repetition count"))
        .unwrap_or(1000);
    let outline = Loop::new(vec![
        Segment::line([100.0, -50.0]),
        Segment::line([100.0, 50.0]),
        Segment::line([-100.0, 50.0]),
        Segment::line([-100.0, -50.0]),
    ])
    .unwrap();
    let source = Extrusion::new(
        Profile::new(outline, vec![], &ProfilePolicy::new(1e-9)).unwrap(),
        Placement3::IDENTITY,
        20.0,
    )
    .unwrap();
    let tools = (-3..=3)
        .map(|x| CylinderTool::new([f64::from(x) * 22.0, 0.0, 0.0], [0.1, 0.0, 1.0], 4.0).unwrap())
        .collect::<Vec<_>>();
    let policy = MachiningPolicy::default();
    let start = Instant::now();
    let mut checksum = 0.0;
    let mut faces = 0;
    for _ in 0..repetitions {
        let mut body = source.body().clone();
        for tool in &tools {
            let result = subtract_through_cylinder_with_roles(
                black_box(&body),
                black_box(tool),
                &policy,
                source.body().faces()[0].role,
                source.body().edges()[0].role,
            )
            .unwrap();
            checksum += result.report.removed_volume;
            body = result.body;
        }
        assert!(
            body.is_closed(),
            "each machining iteration must retain closed incidence"
        );
        faces = body.faces().len();
        black_box(body);
    }
    println!(
        "workload=native_7_oblique_bores repetitions={repetitions} operations={} elapsed_ms={:.3} ns_per_operation={:.1} final_faces={faces} removed_volume_checksum={checksum:.9}",
        repetitions * 7,
        start.elapsed().as_secs_f64() * 1000.0,
        start.elapsed().as_secs_f64() * 1e9 / f64::from(repetitions * 7)
    );
}
