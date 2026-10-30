// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Simple executable benchmark for representative `exedra_fidget` workloads.
//!
//! Run with:
//! `cargo run --release -p exedra_fidget_bench`

use std::hint::black_box;
use std::time::{Duration, Instant};

use exedra_fidget::VmField;
use exedra_implicit::{Aabb, Dual, ScalarField};
use exedra_isosurface::{DelaunayParams, delaunay_mesh};
use fidget::{context::Tree, vm::VmShape};

fn main() {
    let sphere = sphere_field();
    let toothed_torus = toothed_torus_field();
    let point_samples = sample_points(32_768);
    let gradient_samples = sample_points(16_384);

    let eval_scenarios = [
        EvalScenario {
            label: "vm_sphere_eval_points",
            field: &sphere,
            points: &point_samples,
            mode: EvalMode::Points,
            iterations: 200,
        },
        EvalScenario {
            label: "vm_toothed_torus_eval_points",
            field: &toothed_torus,
            points: &point_samples,
            mode: EvalMode::Points,
            iterations: 120,
        },
        EvalScenario {
            label: "vm_toothed_torus_eval_gradients",
            field: &toothed_torus,
            points: &gradient_samples,
            mode: EvalMode::Gradients,
            iterations: 80,
        },
    ];

    for scenario in eval_scenarios {
        run_eval_scenario(&scenario);
    }

    run_extract_scenario(&ExtractScenario {
        label: "vm_toothed_torus_delaunay",
        field: &toothed_torus,
        params: DelaunayParams::new(
            Aabb::new([-1.15, -1.15, -0.55], [1.15, 1.15, 0.55]).expect("valid bounds"),
            2.30 / 64.0,
        ),
        iterations: 4,
    });
}

#[derive(Copy, Clone)]
enum EvalMode {
    Points,
    Gradients,
}

struct EvalScenario<'a> {
    label: &'static str,
    field: &'a VmField,
    points: &'a [[f64; 3]],
    mode: EvalMode,
    iterations: usize,
}

struct ExtractScenario<'a> {
    label: &'static str,
    field: &'a VmField,
    params: DelaunayParams,
    iterations: usize,
}

fn run_eval_scenario(scenario: &EvalScenario<'_>) {
    let mut elapsed_total = Duration::ZERO;
    let mut best = Duration::MAX;
    let mut checksum = 0_u64;
    let sample_count = scenario.points.len();

    match scenario.mode {
        EvalMode::Points => {
            let mut warm = vec![0.0; sample_count];
            scenario.field.eval_points(scenario.points, &mut warm);
            checksum ^= fold_values(&warm);
            for _ in 0..scenario.iterations {
                let mut out = vec![0.0; sample_count];
                let start = Instant::now();
                scenario.field.eval_points(scenario.points, &mut out);
                black_box(());
                let elapsed = start.elapsed();
                elapsed_total += elapsed;
                best = best.min(elapsed);
                checksum ^= fold_values(&out);
            }
        }
        EvalMode::Gradients => {
            let mut warm = vec![Dual::default(); sample_count];
            scenario.field.eval_gradients(scenario.points, &mut warm);
            checksum ^= fold_gradients(&warm);
            for _ in 0..scenario.iterations {
                let mut out = vec![Dual::default(); sample_count];
                let start = Instant::now();
                scenario.field.eval_gradients(scenario.points, &mut out);
                black_box(());
                let elapsed = start.elapsed();
                elapsed_total += elapsed;
                best = best.min(elapsed);
                checksum ^= fold_gradients(&out);
            }
        }
    }

    let avg = elapsed_total / u32::try_from(scenario.iterations).expect("iterations fit in u32");
    println!(
        "{}: mode={} samples={} best={:?} avg={:?} checksum={}",
        scenario.label,
        scenario.mode.label(),
        sample_count,
        best,
        avg,
        checksum,
    );
}

fn run_extract_scenario(scenario: &ExtractScenario<'_>) {
    let warm = delaunay_mesh(scenario.field, &scenario.params).expect("parameters should be valid");
    assert!(warm.report.is_complete(), "{:?}", warm.report);
    let mut elapsed_total = Duration::ZERO;
    let mut best = Duration::MAX;
    let mut checksum = 0_u64;

    for _ in 0..scenario.iterations {
        let start = Instant::now();
        let result = black_box(
            delaunay_mesh(scenario.field, &scenario.params).expect("parameters should be valid"),
        );
        let elapsed = start.elapsed();
        elapsed_total += elapsed;
        best = best.min(elapsed);
        checksum = checksum
            .wrapping_add(result.positions.len() as u64)
            .wrapping_add(result.triangles.len() as u64);
    }

    let avg = elapsed_total / u32::try_from(scenario.iterations).expect("iterations fit in u32");
    let work = warm.report.work;
    println!(
        "{}: mesh_faces={} mesh_vertices={} values={} gradients={} intervals={} best={:?} avg={:?} checksum={}",
        scenario.label,
        warm.triangles.len(),
        warm.positions.len(),
        work.values,
        work.gradients,
        work.intervals,
        best,
        avg,
        checksum,
    );
}

impl EvalMode {
    fn label(self) -> &'static str {
        match self {
            Self::Points => "points",
            Self::Gradients => "gradients",
        }
    }
}

fn sample_points(count: usize) -> Vec<[f64; 3]> {
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let t = index as f64 / count as f64;
        out.push([
            -1.15 + 2.30 * t,
            ((index * 37) % 1024) as f64 / 1024.0 * 2.30 - 1.15,
            ((index * 91) % 1024) as f64 / 1024.0 * 1.10 - 0.55,
        ]);
    }
    out
}

fn sphere_field() -> VmField {
    let x = Tree::x();
    let y = Tree::y();
    let z = Tree::z();
    VmField::new(VmShape::from(
        (x.square() + y.square() + z.square()).sqrt() - 0.82,
    ))
    .expect("shape should be axis-only")
}

fn toothed_torus_field() -> VmField {
    let x = Tree::x();
    let y = Tree::y();
    let z = Tree::z();
    let ring_radius = 0.68;
    let tube_base = 0.19;
    let theta = y.atan2(x.clone());
    let radial = (x.square() + y.square()).sqrt();
    let tube_angle = z.atan2(radial.clone() - ring_radius);
    let teeth = (theta * 12.0).cos() * 0.055;
    let fluting = (tube_angle * 3.0).cos() * 0.025;
    let tube_radius = tube_base + teeth + fluting;
    VmField::new(VmShape::from(
        ((radial - ring_radius).square() + z.square()).sqrt() - tube_radius,
    ))
    .expect("shape should be axis-only")
}

fn fold_values(values: &[f64]) -> u64 {
    values
        .iter()
        .fold(0_u64, |acc, value| acc.wrapping_add(value.to_bits()))
}

fn fold_gradients(values: &[Dual]) -> u64 {
    values.iter().fold(0_u64, |acc, row| {
        row.gradient
            .iter()
            .fold(acc.wrapping_add(row.value.to_bits()), |acc, g| {
                acc.wrapping_add(g.to_bits())
            })
    })
}

#[cfg(test)]
mod tests {
    use super::{sample_points, sphere_field, toothed_torus_field};
    use exedra_implicit::{Dual, ScalarField};

    #[test]
    fn sample_points_are_deterministic_and_non_empty() {
        let samples = sample_points(32);
        assert_eq!(samples.len(), 32);
        assert_eq!(samples[0], [-1.15, -1.15, -0.55]);
        assert_eq!(samples[31][0], -1.15 + 2.30 * (31.0 / 32.0));
    }

    #[test]
    fn benchmark_fields_evaluate_finite_values() {
        let samples = sample_points(128);
        let mut point_values = vec![0.0; samples.len()];
        let mut gradients = vec![Dual::default(); samples.len()];

        for field in [sphere_field(), toothed_torus_field()] {
            field.eval_points(&samples, &mut point_values);
            field.eval_gradients(&samples, &mut gradients);
            assert!(point_values.iter().all(|value| value.is_finite()));
            assert!(gradients.iter().all(|row| {
                row.value.is_finite() && row.gradient.iter().all(|g| g.is_finite())
            }));
        }
    }
}
