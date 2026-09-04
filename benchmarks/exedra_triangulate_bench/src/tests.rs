// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Regression tests for fixture determinism and metric semantics.

use crate::fixtures::{
    FixtureRole, fixtures, incircle_scenarios, input_limited_fixture, power_of_two,
    predicate_scenarios,
};
use crate::metrics::*;
use crate::strategy_params;
use crate::timing::{MAX_BATCH_SIZE, MIN_BATCH_VERTICES, timing_batch_size};
use exedra_triangulate::predicates::{incircle_evaluated, orient2d_evaluated};
use exedra_triangulate::{TriStrategy, refine, triangulate};

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
    ("sparse_rect_dense_hole_64", 0x8e36_bc6b_28d4_242c, 84, 0),
    ("sparse_rect_dense_hole_120", 0xe2fc_297b_5429_0c4b, 148, 0),
    (
        "drill_like_collinear_midpoints",
        0x4e49_b1d2_9d73_98ea,
        12,
        0,
    ),
    ("three_hole_bridge_stress", 0x65f4_ad2d_f5ee_b08e, 42, 0),
    ("small_angle_wedge", 0xf2be_7732_47a4_d796, 19, 0),
    ("choice_scale_down_2p-200", 0xdc6f_f0ad_e1ac_1966, 0, 0),
    ("choice_scale_up_2p200", 0xb43a_6c6e_ffab_7e94, 0, 0),
];

/// Minimum angle guaranteed by the default `sqrt(2)` ratio bound:
/// `asin(1 / (2 sqrt 2))`, minus rounding slack.
const DEFAULT_BOUND_MIN_ANGLE_DEG: f64 = 20.704_811_054_635_35 - 1e-9;

fn assert_default_bound_when_complete(report: &RefineReport, fixture: &str) {
    if report.stats.remaining_bad_triangles == 0 {
        assert!(
            report.min_angle_deg >= DEFAULT_BOUND_MIN_ANGLE_DEG,
            "{fixture}: {} degrees",
            report.min_angle_deg
        );
    }
}

#[test]
fn refinement_signatures_and_work_are_pinned() {
    // The fixed corpus pins both generated geometry and work counts so a
    // refinement-order change cannot masquerade as harmless output drift.
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
    // Completed runs must meet the analytic angle bound, and every designated
    // constrained fixture must improve without regressing any corpus member.
    for fixture in fixtures() {
        let refined = analyze_refined(&fixture);
        let legalized = analyze(&fixture, TriStrategy::ConstrainedDelaunay);
        assert!(!refined.stats.budget_exhausted, "{}", fixture.name);
        // The bound is mandatory only when no bad triangles remain;
        // input-limited violations are honest exceptions caused by immutable
        // boundary geometry.
        assert_default_bound_when_complete(&refined, fixture.name);
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
    // Exact binary scaling should change coordinates only: generated order,
    // triangle indices, and all stopping/work counters must remain identical.
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
    // Keep the timing inputs honest: each named orientation scenario must
    // still reach the arithmetic path whose cost it is intended to measure.
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
    // The timing scenarios are useful only if they continue to exercise both
    // the ordinary filter and the exact dyadic fallback they are named for.
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
    // Every wind-tunnel fixture must produce usable finite metrics before its
    // values can serve as a quality or performance baseline.
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
    // Pin fixture identity, diagnostic role, and ordinary ear-clipped output
    // so an input or baseline drift requires an explicit review.
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
    // Legalized triangle order and exact flip counts are deterministic parts
    // of the wind-tunnel baseline, not unobserved implementation details.
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
    // Uniform binary scaling should preserve topology and dimensionless
    // quality, checking both the reports and the underlying triangle indices.
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
        let result = triangulate(&prepared.input(), &params).expect("scaled fixture triangulates");
        assert_eq!(
            result.triangles, base_result.triangles,
            "{}",
            transformed.name
        );
    }
}

#[test]
fn constrained_delaunay_quality_does_not_regress_and_improves_choice_cases() {
    // Edge legalization may leave constrained cases unchanged, but it must
    // never worsen their minimum angle and must improve choice-driven cases.
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
    // A triangle with analytic 45-degree angles provides a direct oracle for
    // the normalized-quality and minimum-angle formulas.
    let a = [0.0, 0.0];
    let b = [1.0, 0.0];
    let c = [0.0, 1.0];
    assert_eq!(normalized_quality(a, b, c), 0.5);
    assert_eq!(triangle_min_angle(a, b, c), std::f64::consts::FRAC_PI_4);
}

#[test]
fn first_percentile_uses_nearest_rank() {
    // Trap the empty, singleton, and one-percent boundary cases of the
    // explicitly chosen nearest-rank percentile convention.
    let values: Vec<f64> = (0..101).map(f64::from).collect();
    assert_eq!(first_percentile(&[]), 0.0);
    assert_eq!(first_percentile(&values[..1]), 0.0);
    assert_eq!(first_percentile(&values[..99]), 0.0);
    assert_eq!(first_percentile(&values[..100]), 0.0);
    assert_eq!(first_percentile(&values), 1.0);
}

#[test]
fn threshold_counts_are_strict() {
    // Values exactly on a reporting threshold do not count as violations;
    // the next representable value above it does.
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
    // Tiny fixtures are repeated to reduce timer noise, while sufficiently
    // large inputs run once so timing work remains bounded.
    assert_eq!(timing_batch_size(1), MAX_BATCH_SIZE);
    assert!(timing_batch_size(4) > 1);
    assert_eq!(timing_batch_size(MIN_BATCH_VERTICES), 1);
    assert_eq!(timing_batch_size(MIN_BATCH_VERTICES * 2), 1);
}

#[test]
fn input_limited_fixture_exercises_the_incomplete_bound_exception() {
    // This acute corner must exercise the exception side of the wind-tunnel
    // assertion: the input fixes a sub-bound angle, so it is honestly
    // reported and must not be mistaken for a completed quality pass.
    let fixture = input_limited_fixture();
    let report = analyze_refined(&fixture);
    assert!(report.stats.remaining_bad_triangles > 0);
    assert_eq!(
        report.stats.remaining_bad_triangles,
        report.stats.input_limited_triangles
    );
    assert!(!report.stats.budget_exhausted);
    assert!(report.min_angle_deg < DEFAULT_BOUND_MIN_ANGLE_DEG);
    assert_default_bound_when_complete(&report, fixture.name);
}
