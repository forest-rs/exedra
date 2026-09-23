// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Skeletal fork scenario: one branch fork among scattered twigs, extracted
//! in a local box with and without distance culling.

use std::time::{Duration, Instant};

use exedra_isosurface::{
    Aabb, CapsuleField, DualContourParams, EdgeSearchParams, ExtractionLimits, QefParams,
    RoundConeField, ScalarField, SmoothUnionN, SmoothUnionStats, dual_contour,
};

use crate::report;

/// One child of the fork union.
#[derive(Copy, Clone, Debug)]
enum Limb {
    Cone(RoundConeField),
    Twig(CapsuleField),
}

impl ScalarField for Limb {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        match self {
            Self::Cone(field) => field.eval_interval(bounds),
            Self::Twig(field) => field.eval_interval(bounds),
        }
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        match self {
            Self::Cone(field) => field.eval_points(points, out),
            Self::Twig(field) => field.eval_points(points, out),
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        match self {
            Self::Cone(field) => field.eval_gradients(points, out),
            Self::Twig(field) => field.eval_gradients(points, out),
        }
    }
}

impl exedra_isosurface::BoundedField for Limb {
    fn distance_bound(&self) -> exedra_isosurface::DistanceBound {
        match self {
            Self::Cone(field) => field.distance_bound(),
            Self::Twig(field) => field.distance_bound(),
        }
    }
}

const TWIGS: usize = 36;

/// A trunk forking into two limbs at the origin, plus deterministic twigs
/// scattered through a 6 m crown around it.
fn limbs() -> Vec<Limb> {
    let mut limbs = vec![
        Limb::Cone(RoundConeField {
            a: [0.0, 0.0, -1.5],
            b: [0.0, 0.0, 0.0],
            radius_a: 0.22,
            radius_b: 0.16,
        }),
        Limb::Cone(RoundConeField {
            a: [0.0, 0.0, 0.0],
            b: [0.9, 0.2, 1.1],
            radius_a: 0.13,
            radius_b: 0.07,
        }),
        Limb::Cone(RoundConeField {
            a: [0.0, 0.0, 0.0],
            b: [-0.8, -0.3, 1.2],
            radius_a: 0.12,
            radius_b: 0.06,
        }),
    ];
    let mut state = 0x5eed_u64;
    let mut unit = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1_u64 << 24) as f32
    };
    for _ in 0..TWIGS {
        let a = [unit() * 6.0 - 3.0, unit() * 6.0 - 3.0, unit() * 4.0 - 0.5];
        let b = [
            a[0] + unit() - 0.5,
            a[1] + unit() - 0.5,
            a[2] + unit() * 0.6,
        ];
        limbs.push(Limb::Twig(CapsuleField {
            a,
            b,
            radius: 0.02 + unit() * 0.04,
        }));
    }
    limbs
}

fn params(depth: u8) -> DualContourParams {
    DualContourParams {
        root_bounds: Aabb::new([-0.6, -0.6, -0.6], [0.6, 0.6, 0.6]).expect("ordered fork box"),
        max_depth: depth,
        cell_budget: None,
        limits: ExtractionLimits::default(),
        witness_limit: 16,
        vertex_merge_tolerance: 0.0,
        edge_search: EdgeSearchParams::default(),
        qef: QefParams::default(),
    }
}

struct Run {
    signature: u64,
    faces: usize,
    stats: SmoothUnionStats,
    best: Duration,
}

fn run(depth: u8, culling: bool) -> Run {
    let build = || {
        let union = SmoothUnionN::new(limbs(), 0.05);
        if culling {
            union.with_distance_culling()
        } else {
            union
        }
    };
    let field = build();
    let result = dual_contour(&field, &params(depth)).expect("fork extraction");
    let stats = field.stats();
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let field = build();
        let start = Instant::now();
        let _ = dual_contour(&field, &params(depth)).expect("fork extraction");
        best = best.min(start.elapsed());
    }
    Run {
        signature: report::extraction_signature(&result.mesh),
        faces: result.stats.faces,
        stats,
        best,
    }
}

/// Runs the fork scenario at depths 5 to 7 and prints one line per run.
pub(crate) fn run_all() {
    println!("fork.children={}", 3 + TWIGS);
    for depth in 5..=7 {
        let full = run(depth, false);
        let culled = run(depth, true);
        assert_eq!(
            full.signature, culled.signature,
            "culling must not change extraction at depth {depth}"
        );
        for (name, run) in [("full", &full), ("culled", &culled)] {
            println!(
                "fork.depth{depth}.{name}: faces={} points={} child_evals={} child_culls={} \
                 intervals={} child_intervals={} child_interval_culls={} best_ms={:.3}",
                run.faces,
                run.stats.points,
                run.stats.child_evaluations,
                run.stats.child_culls,
                run.stats.intervals,
                run.stats.child_intervals,
                run.stats.child_interval_culls,
                run.best.as_secs_f64() * 1_000.0,
            );
        }
    }
}
