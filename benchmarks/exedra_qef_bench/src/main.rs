// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Simple executable benchmark for representative `exedra_qef` solve sizes.

use exedra_qef::{PlaneConstraint, QefBounds, QefParams, QefSolver};
use std::time::Instant;

fn main() {
    let bounds = QefBounds::new([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]).expect("valid bounds");
    let params = QefParams::default();
    let scenarios = [
        ("4 constraints", scenario(4)),
        ("8 constraints", scenario(8)),
        ("12 constraints", scenario(12)),
    ];

    for (label, constraints) in scenarios {
        let start = Instant::now();
        let mut checksum = 0.0_f32;
        for _ in 0..100_000 {
            let mut solver = QefSolver::new();
            solver.add_constraints(&constraints);
            let result = solver.solve(bounds, &params).expect("benchmark solve");
            checksum += result.position[0] + result.position[1] + result.position[2];
        }
        let elapsed = start.elapsed();
        println!("{label}: {elapsed:?} checksum={checksum}");
    }
}

fn scenario(count: usize) -> Vec<PlaneConstraint> {
    (0..count)
        .map(|index| {
            let t = index as f32;
            PlaneConstraint {
                position: [
                    ((0.17 * t).sin() * 0.5).clamp(-0.9, 0.9),
                    ((0.29 * t).cos() * 0.5).clamp(-0.9, 0.9),
                    ((0.11 * t).sin() * 0.35).clamp(-0.9, 0.9),
                ],
                normal: [
                    0.5 + (0.37 * t).cos(),
                    -0.25 + (0.23 * t).sin(),
                    0.75 + (0.19 * t).cos(),
                ],
            }
        })
        .collect()
}
