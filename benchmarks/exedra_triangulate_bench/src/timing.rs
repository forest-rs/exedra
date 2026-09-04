// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Wall-clock sampling for triangulation, refinement, and predicates.

use std::hint::black_box;
use std::time::{Duration, Instant};

use crate::fixtures::{Fixture, IncircleScenario, PredicateScenario};
use crate::metrics::{hash_u64, refine_params};
use crate::{FNV_OFFSET, Profile, strategy_label, strategy_params};
use exedra_triangulate::predicates::{
    IncirclePath, Orient2dPath, incircle_evaluated, orient2d_evaluated,
};
use exedra_triangulate::{TriStrategy, Triangulation, refine, triangulate, triangulate_with_stats};
pub(crate) const MIN_BATCH_VERTICES: usize = 256;
pub(crate) const MAX_BATCH_SIZE: usize = 64;

pub(crate) struct TimingReport {
    name: &'static str,
    strategy: &'static str,
    vertices: usize,
    edge_flips: usize,
    samples: usize,
    batch_size: usize,
    triangulations: usize,
    best_ns: u128,
    average_ns: u128,
    checksum: u64,
}

impl TimingReport {
    pub(crate) fn print(self) {
        println!(
            "scenario={} strategy={} vertices={} edge_flips={} samples={} batch_size={} triangulations={} best_ns={} avg_ns={} best_ns_per_vertex={:.3} avg_ns_per_vertex={:.3} checksum={:016x}",
            self.name,
            self.strategy,
            self.vertices,
            self.edge_flips,
            self.samples,
            self.batch_size,
            self.triangulations,
            self.best_ns,
            self.average_ns,
            self.best_ns as f64 / self.vertices as f64,
            self.average_ns as f64 / self.vertices as f64,
            self.checksum,
        );
    }
}

pub(crate) fn time_fixture(
    fixture: &Fixture,
    profile: Profile,
    strategy: TriStrategy,
) -> TimingReport {
    let vertices = fixture.vertex_count();
    let batch_size = timing_batch_size(vertices);
    let vertices_per_batch = vertices
        .checked_mul(batch_size)
        .expect("fixed timing batch size must fit usize");
    let samples = (profile.target_vertices() / vertices_per_batch).max(8);
    let triangulations = samples
        .checked_mul(batch_size)
        .expect("fixed timing sample count must fit usize");
    let params = strategy_params(strategy);
    let prepared = fixture.prepare();
    let input = prepared.input();
    let warmup = triangulate_with_stats(black_box(&input), &params).expect("warmup");
    let checksum = triangle_checksum(&warmup.triangulation);
    let edge_flips = warmup.stats.edge_flips;
    black_box(&warmup);

    let mut best = Duration::MAX;
    let mut total = Duration::ZERO;
    for _ in 0..samples {
        let start = Instant::now();
        for _ in 0..batch_size {
            let result = triangulate(black_box(&input), &params).expect("timed fixture");
            black_box(result);
        }
        let elapsed = start.elapsed();
        best = best.min(elapsed);
        total += elapsed;
    }
    black_box(checksum);
    TimingReport {
        name: fixture.name,
        strategy: strategy_label(strategy),
        vertices,
        edge_flips,
        samples,
        batch_size,
        triangulations,
        best_ns: best.as_nanos() / batch_size as u128,
        average_ns: total.as_nanos() / triangulations as u128,
        checksum,
    }
}

pub(crate) fn timing_batch_size(vertices: usize) -> usize {
    MIN_BATCH_VERTICES
        .div_ceil(vertices)
        .clamp(1, MAX_BATCH_SIZE)
}

pub(crate) fn triangle_checksum(result: &Triangulation) -> u64 {
    let mut hash = FNV_OFFSET;
    for triangle in &result.triangles {
        for &index in triangle {
            hash_u64(&mut hash, u64::from(index));
        }
    }
    hash
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct PredicateTimingReport {
    name: &'static str,
    path: &'static str,
    samples: usize,
    predicates_per_sample: usize,
    best_ns: u128,
    average_ns: u128,
}

impl PredicateTimingReport {
    pub(crate) fn print(self) {
        println!(
            "scenario={} path={} samples={} predicates_per_sample={} best_ns={} avg_ns={}",
            self.name,
            self.path,
            self.samples,
            self.predicates_per_sample,
            self.best_ns,
            self.average_ns,
        );
    }
}

pub(crate) fn time_predicate_path(
    scenario: PredicateScenario,
    profile: Profile,
) -> PredicateTimingReport {
    let [a, b, c] = scenario.points;
    let evaluated = orient2d_evaluated(a, b, c);
    assert_eq!(
        evaluated.orientation, scenario.orientation,
        "{}",
        scenario.name
    );
    assert_eq!(evaluated.path, scenario.path, "{}", scenario.name);

    let samples = profile.predicate_samples();
    let predicates_per_sample = profile.predicates_per_sample();
    let mut best = Duration::MAX;
    let mut total = Duration::ZERO;
    for _ in 0..samples {
        let started = Instant::now();
        for _ in 0..predicates_per_sample {
            black_box(orient2d_evaluated(black_box(a), black_box(b), black_box(c)));
        }
        let elapsed = started.elapsed();
        best = best.min(elapsed);
        total += elapsed;
    }
    PredicateTimingReport {
        name: scenario.name,
        path: orient2d_path_label(scenario.path),
        samples,
        predicates_per_sample,
        best_ns: best.as_nanos() / predicates_per_sample as u128,
        average_ns: total.as_nanos()
            / samples
                .checked_mul(predicates_per_sample)
                .expect("fixed predicate sample count must fit usize") as u128,
    }
}

pub(crate) fn time_incircle_path(
    scenario: IncircleScenario,
    profile: Profile,
) -> PredicateTimingReport {
    let [a, b, c, d] = scenario.points;
    let evaluated = incircle_evaluated(a, b, c, d);
    assert_eq!(evaluated.position, scenario.position, "{}", scenario.name);
    assert_eq!(evaluated.path, scenario.path, "{}", scenario.name);

    let samples = profile.predicate_samples();
    let predicates_per_sample = profile.predicates_per_sample();
    let mut best = Duration::MAX;
    let mut total = Duration::ZERO;
    for _ in 0..samples {
        let started = Instant::now();
        for _ in 0..predicates_per_sample {
            black_box(incircle_evaluated(
                black_box(a),
                black_box(b),
                black_box(c),
                black_box(d),
            ));
        }
        let elapsed = started.elapsed();
        best = best.min(elapsed);
        total += elapsed;
    }
    PredicateTimingReport {
        name: scenario.name,
        path: incircle_path_label(scenario.path),
        samples,
        predicates_per_sample,
        best_ns: best.as_nanos() / predicates_per_sample as u128,
        average_ns: total.as_nanos()
            / samples
                .checked_mul(predicates_per_sample)
                .expect("fixed predicate sample count must fit usize") as u128,
    }
}

const fn orient2d_path_label(path: Orient2dPath) -> &'static str {
    match path {
        Orient2dPath::Filter => "orient2d.filter",
        Orient2dPath::NormalizedExpansion => "orient2d.normalized_expansion",
        Orient2dPath::Dyadic => "orient2d.dyadic",
        Orient2dPath::NonFiniteInput => "orient2d.nonfinite",
        _ => "orient2d.unknown",
    }
}

const fn incircle_path_label(path: IncirclePath) -> &'static str {
    match path {
        IncirclePath::Filter => "incircle.filter",
        IncirclePath::Dyadic => "incircle.dyadic",
        IncirclePath::NonFiniteInput => "incircle.nonfinite",
        _ => "incircle.unknown",
    }
}

pub(crate) fn time_refined(fixture: &Fixture, profile: Profile) -> TimingReport {
    let vertices = fixture.vertex_count();
    let batch_size = timing_batch_size(vertices);
    let vertices_per_batch = vertices
        .checked_mul(batch_size)
        .expect("fixed timing batch size must fit usize");
    let samples = (profile.target_vertices() / vertices_per_batch).max(8);
    let triangulations = samples
        .checked_mul(batch_size)
        .expect("fixed timing sample count must fit usize");
    let params = refine_params();
    let prepared = fixture.prepare();
    let input = prepared.input();
    let warmup = refine(black_box(&input), &params).expect("warmup");
    let checksum = triangle_checksum(&Triangulation {
        triangles: warmup.triangles.clone(),
    });
    let edge_flips = warmup.stats.edge_flips;
    black_box(&warmup);

    let mut best = Duration::MAX;
    let mut total = Duration::ZERO;
    for _ in 0..samples {
        let start = Instant::now();
        for _ in 0..batch_size {
            let result = refine(black_box(&input), &params).expect("timed fixture");
            black_box(result);
        }
        let elapsed = start.elapsed();
        best = best.min(elapsed);
        total += elapsed;
    }
    black_box(checksum);
    TimingReport {
        name: fixture.name,
        strategy: "Refined",
        vertices,
        edge_flips,
        samples,
        batch_size,
        triangulations,
        best_ns: best.as_nanos() / batch_size as u128,
        average_ns: total.as_nanos() / triangulations as u128,
        checksum,
    }
}
