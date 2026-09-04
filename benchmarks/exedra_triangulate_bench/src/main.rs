// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic quality and timing wind tunnel for `exedra_triangulate`.
//!
//! Quality/signature reporting is completed before the timed phase so angle
//! calculation and formatting cannot contaminate the wall-clock baseline.

use std::hint::black_box;
use std::time::{Duration, Instant};

use exedra_triangulate::predicates::{
    InCircle, IncirclePath, Orient2dPath, Orientation, incircle_evaluated, orient2d_evaluated,
};
use exedra_triangulate::{
    PolygonInput, RefineParams, RefineStats, RefinedTriangulation, TriParams, TriStrategy,
    Triangulation, refine, triangulate, triangulate_with_stats,
};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const MIN_BATCH_VERTICES: usize = 256;
const MAX_BATCH_SIZE: usize = 64;

fn main() {
    let profile = Profile::from_args(std::env::args().skip(1));
    let fixtures = fixtures();
    for strategy in [TriStrategy::EarClip, TriStrategy::ConstrainedDelaunay] {
        let reports: Vec<QualityReport> = fixtures
            .iter()
            .map(|fixture| analyze(fixture, strategy))
            .collect();

        println!(
            "phase=quality profile={} strategy={} fixtures={}",
            profile.label(),
            strategy_label(strategy),
            fixtures.len()
        );
        for report in &reports {
            report.print();
        }

        println!(
            "phase=timing profile={} strategy={} fixtures={}",
            profile.label(),
            strategy_label(strategy),
            fixtures.len()
        );
        for fixture in &fixtures {
            time_fixture(fixture, profile, strategy).print();
        }
    }

    println!(
        "phase=refine_quality profile={} fixtures={}",
        profile.label(),
        fixtures.len()
    );
    for fixture in &fixtures {
        analyze_refined(fixture).print();
    }
    println!(
        "phase=refine_timing profile={} fixtures={}",
        profile.label(),
        fixtures.len()
    );
    for fixture in &fixtures {
        time_refined(fixture, profile).print();
    }

    let predicate_scenarios = predicate_scenarios();
    let incircle_scenarios = incircle_scenarios();
    println!(
        "phase=predicate_timing profile={} scenarios={}",
        profile.label(),
        predicate_scenarios.len() + incircle_scenarios.len()
    );
    for scenario in predicate_scenarios {
        time_predicate_path(scenario, profile).print();
    }
    for scenario in incircle_scenarios {
        time_incircle_path(scenario, profile).print();
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Profile {
    Quick,
    Stress,
}

impl Profile {
    fn from_args(args: impl Iterator<Item = String>) -> Self {
        let mut profile = Self::Quick;
        for arg in args {
            match arg.as_str() {
                "--quick" => profile = Self::Quick,
                "--stress" => profile = Self::Stress,
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                _ => {
                    eprintln!("unknown argument: {arg}");
                    print_help();
                    std::process::exit(2);
                }
            }
        }
        profile
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Stress => "stress",
        }
    }

    const fn target_vertices(self) -> usize {
        match self {
            Self::Quick => 12_000,
            Self::Stress => 240_000,
        }
    }

    const fn predicate_samples(self) -> usize {
        match self {
            Self::Quick => 9,
            Self::Stress => 21,
        }
    }

    const fn predicates_per_sample(self) -> usize {
        match self {
            Self::Quick => 10_000,
            Self::Stress => 100_000,
        }
    }
}

fn print_help() {
    println!(
        "exedra_triangulate_bench\n\n  --quick   run the short fixed-corpus profile (default)\n  --stress  run longer timing samples over the same corpus\n"
    );
}

#[derive(Clone, Debug)]
struct Fixture {
    name: &'static str,
    role: FixtureRole,
    outer: Vec<[f64; 2]>,
    holes: Vec<Vec<[f64; 2]>>,
}

impl Fixture {
    fn vertex_count(&self) -> usize {
        self.outer.len() + self.holes.iter().map(Vec::len).sum::<usize>()
    }

    fn prepare(&self) -> PreparedFixture<'_> {
        PreparedFixture {
            fixture: self,
            hole_refs: self.holes.iter().map(Vec::as_slice).collect(),
        }
    }

    fn points(&self) -> Vec<[f64; 2]> {
        self.outer
            .iter()
            .chain(self.holes.iter().flatten())
            .copied()
            .collect()
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum FixtureRole {
    ChoiceQuality,
    TieControl,
    InputConstraint,
}

impl FixtureRole {
    const fn label(self) -> &'static str {
        match self {
            Self::ChoiceQuality => "choice_quality",
            Self::TieControl => "tie_control",
            Self::InputConstraint => "input_constraint",
        }
    }
}

struct PreparedFixture<'fixture> {
    fixture: &'fixture Fixture,
    hole_refs: Vec<&'fixture [[f64; 2]]>,
}

impl PreparedFixture<'_> {
    fn input(&self) -> PolygonInput<'_> {
        PolygonInput {
            outer: &self.fixture.outer,
            holes: &self.hole_refs,
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct CircleStep {
    cos: f64,
    sin: f64,
}

impl CircleStep {
    const N16: Self = Self {
        cos: 0.923_879_532_511_286_7,
        sin: 0.382_683_432_365_089_8,
    };
    const N64: Self = Self {
        cos: 0.995_184_726_672_196_9,
        sin: 0.098_017_140_329_560_6,
    };
    const N120: Self = Self {
        cos: 0.998_629_534_754_573_8,
        sin: 0.052_335_956_242_943_835,
    };
}

fn fixtures() -> Vec<Fixture> {
    let choice = vec![[4.9, 4.9], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
    vec![
        Fixture {
            name: "choice_driven_quad",
            role: FixtureRole::ChoiceQuality,
            outer: choice.clone(),
            holes: Vec::new(),
        },
        Fixture {
            name: "exact_cocircular_quad",
            role: FixtureRole::TieControl,
            outer: vec![[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]],
            holes: Vec::new(),
        },
        near_circle_fixture("near_circle_16", 16, CircleStep::N16),
        near_circle_fixture("near_circle_64", 64, CircleStep::N64),
        near_circle_fixture("near_circle_120", 120, CircleStep::N120),
        sparse_hole_fixture("sparse_rect_dense_hole_16", 16, CircleStep::N16),
        sparse_hole_fixture("sparse_rect_dense_hole_64", 64, CircleStep::N64),
        sparse_hole_fixture("sparse_rect_dense_hole_120", 120, CircleStep::N120),
        drill_collinear_fixture(),
        bridge_stress_fixture(),
        Fixture {
            name: "small_angle_wedge",
            role: FixtureRole::InputConstraint,
            outer: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.001, 0.01]],
            holes: Vec::new(),
        },
        Fixture {
            name: "choice_scale_down_2p-200",
            role: FixtureRole::ChoiceQuality,
            outer: scale_points(&choice, power_of_two(-200)),
            holes: Vec::new(),
        },
        Fixture {
            name: "choice_scale_up_2p200",
            role: FixtureRole::ChoiceQuality,
            outer: scale_points(&choice, power_of_two(200)),
            holes: Vec::new(),
        },
    ]
}

fn near_circle_fixture(name: &'static str, count: usize, step: CircleStep) -> Fixture {
    Fixture {
        name,
        role: FixtureRole::TieControl,
        outer: near_circle(count, 10.0, [0.0, 0.0], step),
        holes: Vec::new(),
    }
}

fn sparse_hole_fixture(name: &'static str, count: usize, step: CircleStep) -> Fixture {
    let mut hole = near_circle(count, 4.0, [0.0, 0.0], step);
    hole.reverse();
    Fixture {
        name,
        role: FixtureRole::InputConstraint,
        outer: vec![[-10.0, -6.0], [10.0, -6.0], [10.0, 6.0], [-10.0, 6.0]],
        holes: vec![hole],
    }
}

fn near_circle(count: usize, radius: f64, center: [f64; 2], step: CircleStep) -> Vec<[f64; 2]> {
    let mut points = Vec::with_capacity(count);
    let (mut x, mut y) = (radius, 0.0);
    for _ in 0..count {
        points.push([center[0] + x, center[1] + y]);
        (x, y) = (x * step.cos - y * step.sin, x * step.sin + y * step.cos);
    }
    points
}

fn drill_collinear_fixture() -> Fixture {
    let ring = [
        [13.0, 6.0],
        [12.0, 8.0],
        [10.0, 9.0],
        [8.0, 8.0],
        [7.0, 6.0],
        [8.0, 4.0],
        [10.0, 3.0],
        [12.0, 4.0],
    ];
    let mut hole = Vec::with_capacity(ring.len() * 2);
    for (index, &point) in ring.iter().enumerate() {
        let next = ring[(index + 1) % ring.len()];
        hole.push(point);
        hole.push([(point[0] + next[0]) / 2.0, (point[1] + next[1]) / 2.0]);
    }
    hole.reverse();
    Fixture {
        name: "drill_like_collinear_midpoints",
        role: FixtureRole::InputConstraint,
        outer: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 12.0], [0.0, 12.0]],
        holes: vec![hole],
    }
}

fn bridge_stress_fixture() -> Fixture {
    let mut circle = near_circle(16, 1.5, [15.0, 7.5], CircleStep::N16);
    circle.reverse();
    Fixture {
        name: "three_hole_bridge_stress",
        role: FixtureRole::TieControl,
        outer: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 15.0], [0.0, 15.0]],
        holes: vec![
            vec![[5.0, 5.0], [5.0, 8.0], [8.0, 8.0], [8.0, 5.0]],
            circle,
            vec![[22.0, 5.0], [24.0, 9.0], [26.0, 5.0]],
        ],
    }
}

fn scale_points(points: &[[f64; 2]], scale: f64) -> Vec<[f64; 2]> {
    points
        .iter()
        .map(|point| [point[0] * scale, point[1] * scale])
        .collect()
}

fn power_of_two(exponent: i32) -> f64 {
    assert!(
        (-1022..=1023).contains(&exponent),
        "benchmark power must be a normal finite f64"
    );
    let biased = u64::try_from(exponent + 1023).expect("biased exponent is nonnegative");
    f64::from_bits(biased << 52)
}

#[derive(Copy, Clone, Debug)]
struct PredicateScenario {
    name: &'static str,
    points: [[f64; 2]; 3],
    orientation: Orientation,
    path: Orient2dPath,
}

fn predicate_scenarios() -> [PredicateScenario; 3] {
    [
        PredicateScenario {
            name: "orient2d_filter",
            points: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            orientation: Orientation::Ccw,
            path: Orient2dPath::Filter,
        },
        PredicateScenario {
            name: "orient2d_normalized_expansion",
            points: [[0.0, 0.0], [1e-300, 0.0], [0.0, 1e-300]],
            orientation: Orientation::Ccw,
            path: Orient2dPath::NormalizedExpansion,
        },
        PredicateScenario {
            name: "orient2d_dyadic",
            points: [[f64::from_bits(1), 0.0], [0.5, 0.5], [1.0, 1.0]],
            orientation: Orientation::Cw,
            path: Orient2dPath::Dyadic,
        },
    ]
}

#[derive(Copy, Clone, Debug)]
struct IncircleScenario {
    name: &'static str,
    points: [[f64; 2]; 4],
    position: InCircle,
    path: IncirclePath,
}

fn incircle_scenarios() -> [IncircleScenario; 2] {
    [
        IncircleScenario {
            name: "incircle_filter",
            points: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.25, 0.25]],
            position: InCircle::Inside,
            path: IncirclePath::Filter,
        },
        IncircleScenario {
            name: "incircle_dyadic",
            points: [[0.0, 0.0], [1e100, 0.0], [0.0, 1e100], [1e100, 1e100]],
            position: InCircle::Cocircular,
            path: IncirclePath::Dyadic,
        },
    ]
}

#[derive(Clone, Debug)]
struct QualityReport {
    name: &'static str,
    strategy: &'static str,
    role: FixtureRole,
    vertices: usize,
    holes: usize,
    triangles: usize,
    signature: u64,
    min_angle_deg: f64,
    p01_angle_deg: f64,
    worst_quality: f64,
    below_1deg: usize,
    below_5deg: usize,
    below_10deg: usize,
    edge_flips: usize,
}

impl QualityReport {
    fn print(&self) {
        println!(
            "scenario={} strategy={} role={} vertices={} holes={} triangles={} signature={:016x} min_angle_deg={:.9} p01_angle_deg={:.9} worst_quality={:.9e} below_1deg={} below_5deg={} below_10deg={} edge_flips={}",
            self.name,
            self.strategy,
            self.role.label(),
            self.vertices,
            self.holes,
            self.triangles,
            self.signature,
            self.min_angle_deg,
            self.p01_angle_deg,
            self.worst_quality,
            self.below_1deg,
            self.below_5deg,
            self.below_10deg,
            self.edge_flips,
        );
    }
}

fn analyze(fixture: &Fixture, strategy: TriStrategy) -> QualityReport {
    let params = strategy_params(strategy);
    let prepared = fixture.prepare();
    let input = prepared.input();
    let first = triangulate_with_stats(&input, &params).expect("fixed fixture must triangulate");
    let second =
        triangulate_with_stats(&input, &params).expect("fixed fixture repeat must triangulate");
    assert_eq!(
        first, second,
        "{} must produce byte-identical triangles",
        fixture.name
    );
    let result = &first.triangulation;

    let points = fixture.points();
    validate_cover(fixture, &points, result);
    let mut minimum_angles = Vec::with_capacity(result.triangles.len());
    let mut worst_quality = f64::INFINITY;
    for &triangle in &result.triangles {
        let [a, b, c] = triangle_points(&points, triangle);
        minimum_angles.push(triangle_min_angle(a, b, c));
        worst_quality = worst_quality.min(normalized_quality(a, b, c));
    }
    minimum_angles.sort_by(f64::total_cmp);
    let min_angle = minimum_angles.first().copied().unwrap_or(0.0);
    let p01_angle = first_percentile(&minimum_angles);
    let degrees = 180.0 / std::f64::consts::PI;

    QualityReport {
        name: fixture.name,
        strategy: strategy_label(strategy),
        role: fixture.role,
        vertices: fixture.vertex_count(),
        holes: fixture.holes.len(),
        triangles: result.triangles.len(),
        signature: signature(fixture, result),
        min_angle_deg: min_angle * degrees,
        p01_angle_deg: p01_angle * degrees,
        worst_quality,
        below_1deg: count_below(&minimum_angles, 1.0 / degrees),
        below_5deg: count_below(&minimum_angles, 5.0 / degrees),
        below_10deg: count_below(&minimum_angles, 10.0 / degrees),
        edge_flips: first.stats.edge_flips,
    }
}

/// Quality and work of one budgeted refinement run under the default bound.
struct RefineReport {
    name: &'static str,
    role: FixtureRole,
    vertices: usize,
    triangles: usize,
    signature: u64,
    min_angle_deg: f64,
    p01_angle_deg: f64,
    worst_quality: f64,
    below_10deg: usize,
    below_20deg: usize,
    stats: RefineStats,
}

impl RefineReport {
    fn print(&self) {
        let stats = self.stats;
        println!(
            "scenario={} strategy=Refined role={} vertices={} triangles={} signature={:016x} min_angle_deg={:.9} p01_angle_deg={:.9} worst_quality={:.9e} below_10deg={} below_20deg={} steiner_points={} boundary_splits={} interior_insertions={} declined_insertions={} remaining_bad={} input_limited={} budget_exhausted={} edge_flips={}",
            self.name,
            self.role.label(),
            self.vertices,
            self.triangles,
            self.signature,
            self.min_angle_deg,
            self.p01_angle_deg,
            self.worst_quality,
            self.below_10deg,
            self.below_20deg,
            stats.steiner_points,
            stats.boundary_splits,
            stats.interior_insertions,
            stats.declined_insertions,
            stats.remaining_bad_triangles,
            stats.input_limited_triangles,
            stats.budget_exhausted,
            stats.edge_flips,
        );
    }
}

fn refine_params() -> RefineParams {
    RefineParams::default()
}

fn analyze_refined(fixture: &Fixture) -> RefineReport {
    let params = refine_params();
    let prepared = fixture.prepare();
    let input = prepared.input();
    let first = refine(&input, &params).expect("fixed fixture must refine");
    let second = refine(&input, &params).expect("fixed fixture repeat must refine");
    assert_eq!(
        first, second,
        "{} must produce byte-identical refinement",
        fixture.name
    );
    validate_refined_cover(fixture, &first);

    let mut minimum_angles = Vec::with_capacity(first.triangles.len());
    let mut worst_quality = f64::INFINITY;
    for &triangle in &first.triangles {
        let [a, b, c] = triangle_points(&first.points, triangle);
        minimum_angles.push(triangle_min_angle(a, b, c));
        worst_quality = worst_quality.min(normalized_quality(a, b, c));
    }
    minimum_angles.sort_by(f64::total_cmp);
    let degrees = 180.0 / std::f64::consts::PI;
    RefineReport {
        name: fixture.name,
        role: fixture.role,
        vertices: first.points.len(),
        triangles: first.triangles.len(),
        signature: refined_signature(fixture, &first),
        min_angle_deg: minimum_angles.first().copied().unwrap_or(0.0) * degrees,
        p01_angle_deg: first_percentile(&minimum_angles) * degrees,
        worst_quality,
        below_10deg: count_below(&minimum_angles, 10.0 / degrees),
        below_20deg: count_below(&minimum_angles, 20.0 / degrees),
        stats: first.stats,
    }
}

/// Refined covers add rounded boundary midpoints, so the area check allows
/// rounding-scale slivers; indices, orientation, and provenance are exact.
fn validate_refined_cover(fixture: &Fixture, result: &RefinedTriangulation) {
    let input_count = result.input_vertex_count as usize;
    assert_eq!(input_count, fixture.vertex_count(), "{}", fixture.name);
    assert_eq!(
        result.points.len(),
        input_count + result.steiner.len(),
        "{}",
        fixture.name
    );
    assert_eq!(
        &result.points[..input_count],
        fixture.points(),
        "{}",
        fixture.name
    );
    let mut triangle_area2 = 0.0;
    for &triangle in &result.triangles {
        for index in triangle {
            assert!(
                (index as usize) < result.points.len(),
                "{} emitted out-of-range index {index}",
                fixture.name
            );
        }
        let [a, b, c] = triangle_points(&result.points, triangle);
        let area2 = cross(a, b, c);
        assert!(
            area2 > 0.0,
            "{} emitted non-CCW refined triangle {triangle:?}",
            fixture.name
        );
        triangle_area2 += area2;
    }
    let expected_area2 = signed_area2(&fixture.outer)
        + fixture
            .holes
            .iter()
            .map(|hole| signed_area2(hole))
            .sum::<f64>();
    let scale = expected_area2.abs().max(f64::MIN_POSITIVE);
    assert!(
        (triangle_area2 - expected_area2).abs() <= scale * 1e-10,
        "{} refined area mismatch: triangles={triangle_area2:e} polygon={expected_area2:e}",
        fixture.name
    );
}

/// FNV-1a over the fixture, every generated coordinate in insertion order,
/// and every emitted triangle index.
fn refined_signature(fixture: &Fixture, result: &RefinedTriangulation) -> u64 {
    let mut hash = FNV_OFFSET;
    hash_bytes(&mut hash, fixture.name.as_bytes());
    hash_u64(&mut hash, fixture.outer.len() as u64);
    for point in &fixture.outer {
        hash_point(&mut hash, *point);
    }
    hash_u64(&mut hash, fixture.holes.len() as u64);
    for hole in &fixture.holes {
        hash_u64(&mut hash, hole.len() as u64);
        for point in hole {
            hash_point(&mut hash, *point);
        }
    }
    hash_u64(&mut hash, result.steiner.len() as u64);
    for point in &result.points[result.input_vertex_count as usize..] {
        hash_point(&mut hash, *point);
    }
    for triangle in &result.triangles {
        for &index in triangle {
            hash_u64(&mut hash, u64::from(index));
        }
    }
    hash
}

fn time_refined(fixture: &Fixture, profile: Profile) -> TimingReport {
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

fn validate_cover(fixture: &Fixture, points: &[[f64; 2]], result: &Triangulation) {
    let mut triangle_area2 = 0.0;
    for &triangle in &result.triangles {
        for index in triangle {
            assert!(
                (index as usize) < points.len(),
                "{} emitted out-of-range input index {index}",
                fixture.name
            );
        }
        let [a, b, c] = triangle_points(points, triangle);
        let area2 = cross(a, b, c);
        assert!(
            area2 > 0.0,
            "{} emitted non-CCW triangle {triangle:?}",
            fixture.name
        );
        triangle_area2 += area2;
    }

    let expected_area2 = signed_area2(&fixture.outer)
        + fixture
            .holes
            .iter()
            .map(|hole| signed_area2(hole))
            .sum::<f64>();
    let scale = expected_area2.abs().max(f64::MIN_POSITIVE);
    assert!(
        (triangle_area2 - expected_area2).abs() <= scale * 1e-10,
        "{} area mismatch: triangles={triangle_area2:e} polygon={expected_area2:e}",
        fixture.name
    );
}

fn triangle_points(points: &[[f64; 2]], triangle: [u32; 3]) -> [[f64; 2]; 3] {
    [
        points[triangle[0] as usize],
        points[triangle[1] as usize],
        points[triangle[2] as usize],
    ]
}

fn signed_area2(loop_points: &[[f64; 2]]) -> f64 {
    let mut area2 = 0.0;
    for (index, &a) in loop_points.iter().enumerate() {
        let b = loop_points[(index + 1) % loop_points.len()];
        area2 += a[0] * b[1] - a[1] * b[0];
    }
    area2
}

fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn edge_sq(a: [f64; 2], b: [f64; 2]) -> f64 {
    let x = b[0] - a[0];
    let y = b[1] - a[1];
    x * x + y * y
}

fn normalized_quality(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let longest = edge_sq(a, b).max(edge_sq(b, c)).max(edge_sq(c, a));
    cross(a, b, c).abs() / longest
}

fn triangle_min_angle(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    angle(b, a, c).min(angle(a, b, c)).min(angle(a, c, b))
}

fn angle(a: [f64; 2], vertex: [f64; 2], b: [f64; 2]) -> f64 {
    let u = [a[0] - vertex[0], a[1] - vertex[1]];
    let v = [b[0] - vertex[0], b[1] - vertex[1]];
    let cross = (u[0] * v[1] - u[1] * v[0]).abs();
    let dot = u[0] * v[0] + u[1] * v[1];
    cross.atan2(dot)
}

fn count_below(values: &[f64], threshold: f64) -> usize {
    values.partition_point(|&value| value < threshold)
}

fn first_percentile(sorted_values: &[f64]) -> f64 {
    let index = sorted_values.len().div_ceil(100).saturating_sub(1);
    sorted_values.get(index).copied().unwrap_or(0.0)
}

fn signature(fixture: &Fixture, result: &Triangulation) -> u64 {
    let mut hash = FNV_OFFSET;
    hash_bytes(&mut hash, fixture.name.as_bytes());
    hash_u64(&mut hash, fixture.outer.len() as u64);
    for point in &fixture.outer {
        hash_point(&mut hash, *point);
    }
    hash_u64(&mut hash, fixture.holes.len() as u64);
    for hole in &fixture.holes {
        hash_u64(&mut hash, hole.len() as u64);
        for point in hole {
            hash_point(&mut hash, *point);
        }
    }
    for triangle in &result.triangles {
        for &index in triangle {
            hash_u64(&mut hash, u64::from(index));
        }
    }
    hash
}

fn hash_point(hash: &mut u64, point: [f64; 2]) {
    hash_u64(hash, point[0].to_bits());
    hash_u64(hash, point[1].to_bits());
}

fn hash_u64(hash: &mut u64, value: u64) {
    hash_bytes(hash, &value.to_le_bytes());
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

#[derive(Copy, Clone, Debug)]
struct TimingReport {
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
    fn print(self) {
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

fn time_fixture(fixture: &Fixture, profile: Profile, strategy: TriStrategy) -> TimingReport {
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

fn timing_batch_size(vertices: usize) -> usize {
    MIN_BATCH_VERTICES
        .div_ceil(vertices)
        .clamp(1, MAX_BATCH_SIZE)
}

fn triangle_checksum(result: &Triangulation) -> u64 {
    let mut hash = FNV_OFFSET;
    for triangle in &result.triangles {
        for &index in triangle {
            hash_u64(&mut hash, u64::from(index));
        }
    }
    hash
}

#[derive(Copy, Clone, Debug)]
struct PredicateTimingReport {
    name: &'static str,
    path: &'static str,
    samples: usize,
    predicates_per_sample: usize,
    best_ns: u128,
    average_ns: u128,
}

impl PredicateTimingReport {
    fn print(self) {
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

fn time_predicate_path(scenario: PredicateScenario, profile: Profile) -> PredicateTimingReport {
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

fn time_incircle_path(scenario: IncircleScenario, profile: Profile) -> PredicateTimingReport {
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

fn strategy_params(strategy: TriStrategy) -> TriParams {
    let mut params = TriParams::default();
    params.strategy = strategy;
    params
}

const fn strategy_label(strategy: TriStrategy) -> &'static str {
    match strategy {
        TriStrategy::EarClip => "EarClip",
        TriStrategy::ConstrainedDelaunay => "ConstrainedDelaunay",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORPUS_PINS: [(&str, FixtureRole, u64); 13] = [
        (
            "choice_driven_quad",
            FixtureRole::ChoiceQuality,
            0x9e27_621e_5c12_35b7,
        ),
        (
            "exact_cocircular_quad",
            FixtureRole::TieControl,
            0x6f01_fe1c_ee8a_2602,
        ),
        (
            "near_circle_16",
            FixtureRole::TieControl,
            0x6036_d8f5_1e65_536c,
        ),
        (
            "near_circle_64",
            FixtureRole::TieControl,
            0x8c7e_9310_3fef_ef5f,
        ),
        (
            "near_circle_120",
            FixtureRole::TieControl,
            0x7883_2136_483a_dbf9,
        ),
        (
            "sparse_rect_dense_hole_16",
            FixtureRole::InputConstraint,
            0x1aac_7386_2e82_d721,
        ),
        (
            "sparse_rect_dense_hole_64",
            FixtureRole::InputConstraint,
            0x6e4d_3f6c_516c_412b,
        ),
        (
            "sparse_rect_dense_hole_120",
            FixtureRole::InputConstraint,
            0x4eed_246c_6ae1_f30b,
        ),
        (
            "drill_like_collinear_midpoints",
            FixtureRole::InputConstraint,
            0x689e_8b17_9902_c6a2,
        ),
        (
            "three_hole_bridge_stress",
            FixtureRole::TieControl,
            0xf54c_563e_80e1_b944,
        ),
        (
            "small_angle_wedge",
            FixtureRole::InputConstraint,
            0x3fc7_7c22_3b03_009d,
        ),
        (
            "choice_scale_down_2p-200",
            FixtureRole::ChoiceQuality,
            0x49c5_f33e_7b05_4306,
        ),
        (
            "choice_scale_up_2p200",
            FixtureRole::ChoiceQuality,
            0x9bf9_dca4_e4b6_8d74,
        ),
    ];

    const DELAUNAY_PINS: [(&str, u64, usize); 13] = [
        ("choice_driven_quad", 0x61e1_e303_41fc_2357, 1),
        ("exact_cocircular_quad", 0xfb69_ec33_ed2e_9fe2, 1),
        ("near_circle_16", 0x429d_e374_81fd_242a, 23),
        ("near_circle_64", 0x5c4e_0ae6_2a32_8660, 116),
        ("near_circle_120", 0x581c_c8f0_19d7_428f, 96),
        ("sparse_rect_dense_hole_16", 0x5691_c23f_23f3_30c3, 4),
        ("sparse_rect_dense_hole_64", 0xc1b6_ec9a_ceb5_fc37, 11),
        ("sparse_rect_dense_hole_120", 0xec57_fe5b_ca27_46fd, 21),
        ("drill_like_collinear_midpoints", 0x0fb3_38c6_3e40_b363, 1),
        ("three_hole_bridge_stress", 0xdcff_9d39_e6f3_51fc, 27),
        ("small_angle_wedge", 0x9210_97b6_21ea_dcdd, 0),
        ("choice_scale_down_2p-200", 0xd62d_e155_79a9_bce6, 1),
        ("choice_scale_up_2p200", 0xd83f_5bbf_fecc_9fd4, 1),
    ];

    /// `(name, signature, steiner_points, remaining_bad)` under the default
    /// refinement bound and budget.
    const REFINE_PINS: [(&str, u64, u32, usize); 13] = [
        ("choice_driven_quad", 0x97c7_b807_45c5_17f7, 0, 0),
        ("exact_cocircular_quad", 0x68b8_8d88_f0ae_33e2, 0, 0),
        ("near_circle_16", 0x247d_2a72_dd61_6d18, 1, 0),
        ("near_circle_64", 0x2835_df33_b8ee_65c2, 45, 0),
        ("near_circle_120", 0xdf31_dc03_b25e_1b67, 99, 0),
        ("sparse_rect_dense_hole_16", 0x85cf_200e_8c38_dc93, 16, 0),
        ("sparse_rect_dense_hole_64", 0xe542_3a1d_f417_e323, 84, 0),
        ("sparse_rect_dense_hole_120", 0xe2fc_297b_5429_0c4b, 148, 0),
        (
            "drill_like_collinear_midpoints",
            0x4e49_b1d2_9d73_98ea,
            12,
            0,
        ),
        ("three_hole_bridge_stress", 0xcc43_0d26_62a3_de73, 43, 0),
        ("small_angle_wedge", 0x675d_421c_11d2_3fbc, 20, 0),
        ("choice_scale_down_2p-200", 0xdc6f_f0ad_e1ac_1966, 0, 0),
        ("choice_scale_up_2p200", 0xb43a_6c6e_ffab_7e94, 0, 0),
    ];

    /// Minimum angle guaranteed by the default `sqrt(2)` ratio bound:
    /// `asin(1 / (2 sqrt 2))`, minus rounding slack.
    const DEFAULT_BOUND_MIN_ANGLE_DEG: f64 = 20.704_811_054_635_35 - 1e-9;

    #[test]
    fn refinement_signatures_and_work_are_pinned() {
        let fixtures = fixtures();
        assert_eq!(fixtures.len(), REFINE_PINS.len());
        for (fixture, &(expected_name, expected_signature, expected_steiner, expected_bad)) in
            fixtures.iter().zip(&REFINE_PINS)
        {
            assert_eq!(fixture.name, expected_name);
            let report = analyze_refined(fixture);
            assert_eq!(report.signature, expected_signature, "{expected_name}");
            assert_eq!(
                report.stats.steiner_points, expected_steiner,
                "{expected_name}"
            );
            assert_eq!(
                report.stats.remaining_bad_triangles, expected_bad,
                "{expected_name}"
            );
        }
    }

    #[test]
    fn refinement_meets_the_default_bound_and_improves_constrained_fixtures() {
        for fixture in fixtures() {
            let refined = analyze_refined(&fixture);
            let legalized = analyze(&fixture, TriStrategy::ConstrainedDelaunay);
            assert!(!refined.stats.budget_exhausted, "{}", fixture.name);
            if refined.stats.remaining_bad_triangles == refined.stats.input_limited_triangles {
                assert!(
                    refined.min_angle_deg >= DEFAULT_BOUND_MIN_ANGLE_DEG,
                    "{}: {} degrees",
                    fixture.name,
                    refined.min_angle_deg
                );
            }
            assert!(
                refined.min_angle_deg >= legalized.min_angle_deg,
                "{}: refinement regressed the minimum angle",
                fixture.name
            );
            if fixture.role == FixtureRole::InputConstraint {
                assert!(
                    refined.min_angle_deg > legalized.min_angle_deg * 2.0,
                    "{}: refinement must materially improve a constrained fixture",
                    fixture.name
                );
            }
        }
    }

    #[test]
    fn power_of_two_fixtures_refine_to_exactly_scaled_results() {
        let fixtures = fixtures();
        let base = refine(&fixtures[0].prepare().input(), &refine_params()).expect("base");
        for (fixture, exponent) in [(&fixtures[11], -200), (&fixtures[12], 200)] {
            let scaled = refine(&fixture.prepare().input(), &refine_params()).expect("scaled");
            assert_eq!(scaled.triangles, base.triangles, "{}", fixture.name);
            assert_eq!(scaled.stats, base.stats, "{}", fixture.name);
            let scale = power_of_two(exponent);
            for (got, expected) in scaled.points.iter().zip(&base.points) {
                assert_eq!(
                    *got,
                    [expected[0] * scale, expected[1] * scale],
                    "{}",
                    fixture.name
                );
            }
        }
    }

    #[test]
    fn predicate_scenarios_reach_each_typed_path() {
        let scenarios = predicate_scenarios();
        assert_eq!(scenarios.len(), 3);
        for scenario in scenarios {
            let [a, b, c] = scenario.points;
            let evaluated = orient2d_evaluated(a, b, c);
            assert_eq!(
                evaluated.orientation, scenario.orientation,
                "{}",
                scenario.name
            );
            assert_eq!(evaluated.path, scenario.path, "{}", scenario.name);
        }
    }

    #[test]
    fn incircle_scenarios_reach_each_typed_path() {
        let scenarios = incircle_scenarios();
        assert_eq!(scenarios.len(), 2);
        for scenario in scenarios {
            let [a, b, c, d] = scenario.points;
            let evaluated = incircle_evaluated(a, b, c, d);
            assert_eq!(evaluated.position, scenario.position, "{}", scenario.name);
            assert_eq!(evaluated.path, scenario.path, "{}", scenario.name);
        }
    }

    #[test]
    fn fixed_corpus_triangulates_and_reports_finite_quality() {
        for fixture in fixtures() {
            let report = analyze(&fixture, TriStrategy::EarClip);
            assert!(report.triangles > 0, "{}", fixture.name);
            assert!(report.min_angle_deg.is_finite(), "{}", fixture.name);
            assert!(report.p01_angle_deg.is_finite(), "{}", fixture.name);
            assert!(report.worst_quality.is_finite(), "{}", fixture.name);
            assert!(report.worst_quality > 0.0, "{}", fixture.name);
        }
    }

    #[test]
    fn corpus_signatures_are_pinned() {
        let fixtures = fixtures();
        assert_eq!(fixtures.len(), CORPUS_PINS.len());
        for (fixture, &(expected_name, expected_role, expected_signature)) in
            fixtures.iter().zip(&CORPUS_PINS)
        {
            assert_eq!(fixture.name, expected_name);
            assert_eq!(fixture.role, expected_role, "{expected_name}");
            assert_eq!(
                analyze(fixture, TriStrategy::EarClip).signature,
                expected_signature,
                "{expected_name}"
            );
        }
    }

    #[test]
    fn constrained_delaunay_signatures_and_work_are_pinned() {
        let fixtures = fixtures();
        assert_eq!(fixtures.len(), DELAUNAY_PINS.len());
        for (fixture, &(expected_name, expected_signature, expected_flips)) in
            fixtures.iter().zip(&DELAUNAY_PINS)
        {
            assert_eq!(fixture.name, expected_name);
            let report = analyze(fixture, TriStrategy::ConstrainedDelaunay);
            assert_eq!(report.signature, expected_signature, "{expected_name}");
            assert_eq!(report.edge_flips, expected_flips, "{expected_name}");
        }
    }

    #[test]
    fn power_of_two_fixtures_preserve_indices_and_every_quality_metric() {
        let fixtures = fixtures();
        let reports: Vec<QualityReport> = fixtures
            .iter()
            .map(|fixture| analyze(fixture, TriStrategy::EarClip))
            .collect();
        let base = &reports[0];
        for transformed in &reports[11..=12] {
            assert_eq!(transformed.triangles, base.triangles);
            assert_eq!(transformed.min_angle_deg, base.min_angle_deg);
            assert_eq!(transformed.p01_angle_deg, base.p01_angle_deg);
            assert_eq!(transformed.worst_quality, base.worst_quality);
            assert_eq!(transformed.below_1deg, base.below_1deg);
            assert_eq!(transformed.below_5deg, base.below_5deg);
            assert_eq!(transformed.below_10deg, base.below_10deg);
        }

        let params = strategy_params(TriStrategy::EarClip);
        let base_prepared = fixtures[0].prepare();
        let base_result =
            triangulate(&base_prepared.input(), &params).expect("base fixture triangulates");
        for transformed in &fixtures[11..=12] {
            let prepared = transformed.prepare();
            let result =
                triangulate(&prepared.input(), &params).expect("scaled fixture triangulates");
            assert_eq!(
                result.triangles, base_result.triangles,
                "{}",
                transformed.name
            );
        }
    }

    #[test]
    fn constrained_delaunay_quality_does_not_regress_and_improves_choice_cases() {
        for fixture in fixtures() {
            let ear_clip = analyze(&fixture, TriStrategy::EarClip);
            let delaunay = analyze(&fixture, TriStrategy::ConstrainedDelaunay);
            assert_eq!(delaunay.triangles, ear_clip.triangles, "{}", fixture.name);
            assert!(
                delaunay.min_angle_deg >= ear_clip.min_angle_deg,
                "{}: {} < {}",
                fixture.name,
                delaunay.min_angle_deg,
                ear_clip.min_angle_deg
            );
            if fixture.role == FixtureRole::ChoiceQuality {
                assert!(
                    delaunay.min_angle_deg > ear_clip.min_angle_deg,
                    "{} should improve when diagonal choice is causal",
                    fixture.name
                );
                assert!(delaunay.edge_flips > 0, "{}", fixture.name);
            }
        }
    }

    #[test]
    fn element_metrics_match_a_right_isosceles_triangle() {
        let a = [0.0, 0.0];
        let b = [1.0, 0.0];
        let c = [0.0, 1.0];
        assert_eq!(normalized_quality(a, b, c), 0.5);
        assert_eq!(triangle_min_angle(a, b, c), std::f64::consts::FRAC_PI_4);
    }

    #[test]
    fn first_percentile_uses_nearest_rank() {
        let values: Vec<f64> = (0..101).map(f64::from).collect();
        assert_eq!(first_percentile(&[]), 0.0);
        assert_eq!(first_percentile(&values[..1]), 0.0);
        assert_eq!(first_percentile(&values[..99]), 0.0);
        assert_eq!(first_percentile(&values[..100]), 0.0);
        assert_eq!(first_percentile(&values), 1.0);
    }

    #[test]
    fn threshold_counts_are_strict() {
        let values = [1.0, 5.0, 10.0];
        assert_eq!(count_below(&values, 1.0), 0);
        assert_eq!(
            count_below(&values, f64::from_bits(1.0_f64.to_bits() + 1)),
            1
        );
        assert_eq!(count_below(&values, 5.0), 1);
        assert_eq!(count_below(&values, 10.0), 2);
    }

    #[test]
    fn tiny_fixture_timings_are_batched() {
        assert_eq!(timing_batch_size(1), MAX_BATCH_SIZE);
        assert!(timing_batch_size(4) > 1);
        assert_eq!(timing_batch_size(MIN_BATCH_VERTICES), 1);
        assert_eq!(timing_batch_size(MIN_BATCH_VERTICES * 2), 1);
    }
}
