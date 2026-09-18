// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::builders;

fn cubic_profile() -> Profile2 {
    Profile2::simple(
        Loop2::new(vec![
            Seg2::line((10.0, 0.0)).tagged(SegTag(0)),
            Seg2::cubic((10.0, 10.0), (14.0, 3.0), (14.0, 7.0)).tagged(SegTag(1)),
            Seg2::line((0.0, 10.0)).tagged(SegTag(2)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(3)),
        ])
        .expect("loop"),
    )
    .expect("profile")
}

fn assert_coverage(result: &OffsetResult) {
    for (index, loop_) in core::iter::once(result.profile.outer())
        .chain(result.profile.holes())
        .enumerate()
    {
        let mut end = 0;
        for run in result
            .runs
            .iter()
            .filter(|r| r.hole == index.checked_sub(1))
        {
            assert_eq!(run.result.start, end);
            assert!(run.result.end > end);
            end = run.result.end;
        }
        assert_eq!(end as usize, loop_.segs().len());
    }
    assert!(result.work.source_segments <= u64::from(result.policy.max_source_segments));
    assert!(result.work.result_segments <= u64::from(result.policy.max_result_segments));
    assert!(result.work.cubic_fits <= u64::from(result.policy.max_cubic_fits));
    assert!(result.work.check_edges <= u64::from(result.policy.max_check_edges));
    assert!(result.work.check_pairs <= result.policy.max_check_pairs);
}

#[test]
fn analytic_offsets_keep_geometry_and_charge_before_work() {
    let source = builders::rect_from_corner(40.0, 20.0).expect("rectangle");
    let corners = CornerPolicy::Miter { limit: 2.0 };
    let policy = OffsetPolicy {
        max_source_segments: 4,
        max_result_segments: 4,
        max_cubic_fits: 0,
        max_check_edges: 8,
        max_check_pairs: 18,
        ..Default::default()
    };
    let result = source
        .offset_with_policy(0.2, corners, &policy)
        .expect("exact budgets");
    assert_eq!(result.profile, source.offset(0.2, corners).expect("legacy"));
    assert_eq!(
        result.work,
        OffsetWork {
            source_segments: 4,
            result_segments: 4,
            cubic_fits: 0,
            check_edges: 8,
            check_pairs: 18
        }
    );
    assert!(
        result
            .runs
            .iter()
            .all(|r| r.method == OffsetMethod::Analytic)
    );
    assert_coverage(&result);
    for (budget, limited) in [
        (
            OffsetBudget::SourceSegments,
            OffsetPolicy {
                max_source_segments: 3,
                ..policy
            },
        ),
        (
            OffsetBudget::ResultSegments,
            OffsetPolicy {
                max_result_segments: 3,
                ..policy
            },
        ),
        (
            OffsetBudget::CheckEdges,
            OffsetPolicy {
                max_check_edges: 7,
                ..policy
            },
        ),
        (
            OffsetBudget::CheckPairs,
            OffsetPolicy {
                max_check_pairs: 17,
                ..policy
            },
        ),
    ] {
        assert!(matches!(source.offset_with_policy(0.2, corners, &limited),
            Err(ProfileError::OffsetBudgetExceeded { budget: actual, .. }) if actual == budget));
    }
}

#[test]
fn fitted_runs_keep_source_tags_and_explicit_fitting_target() {
    let source = cubic_profile();
    let coarse = OffsetPolicy::default();
    let fine = OffsetPolicy {
        fit_tolerance: 1e-8,
        ..coarse
    };
    let a = source
        .offset_with_policy(1.0, CornerPolicy::Round, &coarse)
        .expect("coarse fit");
    let b = source
        .offset_with_policy(1.0, CornerPolicy::Round, &fine)
        .expect("fine fit");
    assert!(b.profile.outer().segs().len() > a.profile.outer().segs().len());
    for result in [&a, &b] {
        assert_coverage(result);
        assert_eq!(result.work.cubic_fits, 1);
        let fitted: Vec<_> = result
            .runs
            .iter()
            .filter(|r| r.method == OffsetMethod::Fitted)
            .collect();
        assert_eq!(fitted.len(), 1);
        assert_eq!(fitted[0].source, Some(1));
        for i in fitted[0].result.clone() {
            assert_eq!(
                result.profile.outer().segs()[i as usize].tag,
                Some(SegTag(1))
            );
        }
        assert!(
            result
                .runs
                .iter()
                .any(|r| r.source.is_none() && r.method == OffsetMethod::Analytic)
        );
    }
    assert_eq!(b.policy.fit_tolerance, 1e-8);
    let limited = OffsetPolicy {
        max_cubic_fits: 0,
        ..coarse
    };
    assert!(matches!(
        source.offset_with_policy(1.0, CornerPolicy::Round, &limited),
        Err(ProfileError::OffsetBudgetExceeded {
            budget: OffsetBudget::CubicFits,
            maximum: 0
        })
    ));
}

#[test]
fn checking_accuracy_changes_work_without_refitting_analytic_geometry() {
    let source = builders::rect_from_corner(40.0, 20.0).expect("rectangle");
    let coarse = OffsetPolicy::default();
    let fine = OffsetPolicy {
        check_tolerance: 0.001,
        ..coarse
    };
    let a = source
        .offset_with_policy(1.0, CornerPolicy::Round, &coarse)
        .expect("coarse checks");
    let b = source
        .offset_with_policy(1.0, CornerPolicy::Round, &fine)
        .expect("fine checks");
    assert_eq!(a.profile, b.profile);
    assert!(b.work.check_edges > a.work.check_edges);
    assert!(b.work.check_pairs > a.work.check_pairs);
    assert_coverage(&b);
}

#[test]
fn zero_distance_records_identity_and_validates_policy_first() {
    let source = cubic_profile();
    let policy = OffsetPolicy {
        max_cubic_fits: 0,
        max_check_edges: 0,
        max_check_pairs: 0,
        ..Default::default()
    };
    let result = source
        .offset_with_policy(0.0, CornerPolicy::Round, &policy)
        .expect("identity");
    assert_eq!(result.profile, source);
    assert_eq!(result.work.check_edges, 0);
    assert!(
        result
            .runs
            .iter()
            .all(|r| r.method == OffsetMethod::Identity)
    );
    assert_coverage(&result);
    for bad in [
        OffsetPolicy {
            fit_tolerance: 0.0,
            ..policy
        },
        OffsetPolicy {
            check_tolerance: f64::NAN,
            ..policy
        },
        OffsetPolicy {
            undercut_slack: 0.0,
            ..policy
        },
        OffsetPolicy {
            undercut_slack: f64::INFINITY,
            ..policy
        },
    ] {
        assert_eq!(
            source
                .offset_with_policy(0.0, CornerPolicy::Round, &bad)
                .unwrap_err(),
            ProfileError::InvalidOffsetPolicy
        );
    }
    assert!(matches!(
        source.offset_with_policy(
            0.0,
            CornerPolicy::Round,
            &OffsetPolicy {
                max_result_segments: 0,
                ..policy
            }
        ),
        Err(ProfileError::OffsetBudgetExceeded {
            budget: OffsetBudget::ResultSegments,
            maximum: 0
        })
    ));
}

#[test]
fn collapse_and_unsupported_cubic_trimming_remain_failures() {
    let policy = OffsetPolicy::default();
    let rect = builders::rect_from_corner(40.0, 20.0).expect("rectangle");
    assert!(
        rect.offset_with_policy(-11.0, CornerPolicy::Miter { limit: 2.0 }, &policy)
            .is_err()
    );
    assert!(matches!(
        cubic_profile().offset_with_policy(-1.0, CornerPolicy::Round, &policy),
        Err(ProfileError::OffsetCornerUnsupported { .. })
    ));
}

#[test]
fn hole_checks_share_the_operation_budget() {
    let source = tests::square_hole_profile();
    let policy = OffsetPolicy::default();
    let result = source
        .offset_with_policy(1.0, CornerPolicy::Miter { limit: 2.0 }, &policy)
        .expect("hole offset");
    assert_coverage(&result);
    assert_eq!(result.work.source_segments, 8);
    assert_eq!(result.work.result_segments, 8);
    assert_eq!(result.work.check_edges, 24); // both source/result pairs plus separation sampling
    assert_eq!(result.work.check_pairs, 56); // two local checks plus outer/hole separation
    let limited = OffsetPolicy {
        max_check_pairs: 55,
        ..policy
    };
    assert!(matches!(
        source.offset_with_policy(1.0, CornerPolicy::Miter { limit: 2.0 }, &limited),
        Err(ProfileError::OffsetBudgetExceeded {
            budget: OffsetBudget::CheckPairs,
            maximum: 55
        })
    ));
}

#[test]
fn hole_pair_checks_are_bounded_and_contacts_are_refused() {
    fn square(x: f64, y: f64, size: f64) -> Loop2 {
        Loop2::new(vec![
            Seg2::line((x + size, y)),
            Seg2::line((x + size, y + size)),
            Seg2::line((x, y + size)),
            Seg2::line((x, y)),
        ])
        .expect("square")
    }
    let source = Profile2::new(
        square(0.0, 0.0, 40.0),
        vec![
            square(8.0, 8.0, 4.0).reversed(),
            square(14.0, 8.0, 4.0).reversed(),
        ],
    )
    .expect("two holes");
    let corners = CornerPolicy::Miter { limit: 2.0 };
    let policy = OffsetPolicy::default();
    let result = source
        .offset_with_policy(-0.5, corners, &policy)
        .expect("separated holes");
    assert_coverage(&result);
    assert_eq!(result.work.check_edges, 44);
    assert_eq!(result.work.check_pairs, 118);
    assert!(matches!(
        source.offset_with_policy(
            -0.5,
            corners,
            &OffsetPolicy {
                max_check_pairs: 117,
                ..policy
            }
        ),
        Err(ProfileError::OffsetBudgetExceeded {
            budget: OffsetBudget::CheckPairs,
            maximum: 117
        })
    ));
    assert!(matches!(
        source.offset_with_policy(-1.0, corners, &policy),
        Err(ProfileError::OverlappingHoles {
            first: 0,
            second: 1
        })
    ));

    let tiny = Profile2::new(
        square(0.0, 0.0, 40.0),
        vec![square(8.0, 8.0, 0.01).reversed()],
    )
    .expect("tiny hole");
    assert!(
        matches!(
            tiny.offset_with_policy(0.1, corners, &policy),
            Err(ProfileError::OffsetLoopDegenerate { hole: Some(0) })
        ),
        "a consumed line must not reverse even when clearance slack exceeds the feature width"
    );
}

#[test]
fn explicit_units_scale_geometry_and_charges_together() {
    let reference = OffsetPolicy::default();
    let base = builders::rect_from_corner(40.0, 20.0)
        .expect("rectangle")
        .offset_with_policy(1.0, CornerPolicy::Round, &reference)
        .expect("reference");
    for units in [0.001, 1000.0] {
        let policy = OffsetPolicy {
            fit_tolerance: reference.fit_tolerance * units,
            check_tolerance: reference.check_tolerance * units,
            undercut_slack: reference.undercut_slack * units,
            ..reference
        };
        let scaled = builders::rect_from_corner(40.0 * units, 20.0 * units)
            .expect("rectangle")
            .offset_with_policy(units, CornerPolicy::Round, &policy)
            .expect("scaled");
        assert_eq!(base.work, scaled.work);
        assert_eq!(base.runs, scaled.runs);
        for (a, b) in base
            .profile
            .outer()
            .segs()
            .iter()
            .zip(scaled.profile.outer().segs())
        {
            assert!((a.to.x - b.to.x / units).abs() < 1e-10);
            assert!((a.to.y - b.to.y / units).abs() < 1e-10);
        }
    }
}

#[test]
fn fitted_curve_meets_requested_target_at_independent_normal_samples() {
    use kurbo::ParamCurveNearest;
    let source = cubic_profile();
    let cubic = CubicBez::new((10.0, 0.0), (14.0, 3.0), (14.0, 7.0), (10.0, 10.0));
    for tolerance in [0.001, 1e-8] {
        let policy = OffsetPolicy {
            fit_tolerance: tolerance,
            ..Default::default()
        };
        let result = source
            .offset_with_policy(1.0, CornerPolicy::Round, &policy)
            .expect("fit");
        let output = result.profile.outer().to_bez_path();
        for i in 0..=128 {
            let t = f64::from(i) / 128.0;
            let d = cubic.deriv().eval(t).to_vec2();
            let length = libm::hypot(d.x, d.y);
            let exact = cubic.eval(t) + Vec2::new(d.y / length, -d.x / length);
            let error_squared = output
                .segments()
                .map(|s| s.nearest(exact, 1e-12).distance_sq)
                .fold(f64::INFINITY, f64::min);
            assert!(
                libm::sqrt(error_squared) <= tolerance + 1e-10,
                "independent normal sample {t} exceeds fitting target {tolerance}"
            );
        }
    }
}

#[test]
fn unresolvable_check_accuracy_and_nonpositive_clearance_floor_fail() {
    let source = builders::rect_from_corner(40.0, 20.0).expect("rectangle");
    let policy = OffsetPolicy {
        check_tolerance: 1e-15,
        fit_tolerance: 1e-16,
        max_check_edges: 64,
        ..Default::default()
    };
    assert!(
        matches!(
            source.offset_with_policy(1.0, CornerPolicy::Round, &policy),
            Err(ProfileError::OffsetCheckSampling(_))
        ),
        "an accuracy request must not silently clamp to the edge budget"
    );
    assert_eq!(
        source
            .offset_with_policy(0.001, CornerPolicy::Round, &OffsetPolicy::default())
            .unwrap_err(),
        ProfileError::InvalidOffsetPolicy,
        "slack must not erase the entire requested clearance"
    );
}
