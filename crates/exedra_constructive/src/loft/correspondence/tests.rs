// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{
    builders,
    ir::{CapMode, LoftPolicy, Placement3},
    tessellate::{EvalPolicy, tessellate_loft},
};
const POLICY: CorrespondencePolicy = CorrespondencePolicy::new(ProfilePolicy::new(1e-9));

fn check_sources(input: &Profile2, output: &MatchedProfile) {
    let source = input.to_line_arc(&POLICY.profile).unwrap();
    let prepared = output.profile.to_line_arc(&POLICY.profile).unwrap();
    assert!(
        (source.area() - prepared.area()).abs() < 1e-9,
        "exact split preserves area"
    );
    for (loop_id, spans) in output.segments.iter().enumerate() {
        let original = ring(&source, loop_id);
        let result = ring(&prepared, loop_id);
        assert_eq!(spans.len(), result.segments().len());
        for ((from, segment), span) in result.iter_with_starts().zip(spans) {
            let n = original.segments().len();
            let start = original.segments()[(span.source.segment + n - 1) % n].to;
            let original_segment = &original.segments()[span.source.segment];
            let point = |t| {
                original.arc(span.source.segment).map_or_else(
                    || core::array::from_fn(|k| start[k] + t * (original_segment.to[k] - start[k])),
                    |a| a.point_at(t),
                )
            };
            for (actual, expected) in [
                (from, point(span.parameter[0])),
                (segment.to, point(span.parameter[1])),
            ] {
                assert!(
                    libm::hypot(actual[0] - expected[0], actual[1] - expected[1]) < 1e-10,
                    "source interval {span:?}"
                );
            }
            assert_eq!(segment.tag, original_segment.tag);
        }
        for segment in 0..original.segments().len() {
            let mut intervals: Vec<_> = spans
                .iter()
                .filter(|s| s.source.segment == segment)
                .map(|s| s.parameter)
                .collect();
            intervals.sort_by(|a, b| a[0].total_cmp(&b[0]));
            assert_eq!(intervals[0][0], 0.0);
            assert_eq!(intervals.last().unwrap()[1], 1.0);
            assert!(
                intervals.windows(2).all(|p| p[0][1] == p[1][0]),
                "original segment covered exactly once"
            );
        }
    }
}

#[test]
fn normalized_perimeters_keep_original_shapes_corners_tags_and_intervals() {
    let circle = builders::circle(10.0).unwrap();
    let rectangle = builders::rect_centered(24.0, 16.0).unwrap();
    let result = correspond_profiles(
        &[
            CorrespondenceProfile::new(&circle),
            CorrespondenceProfile::new(&rectangle),
        ],
        &POLICY,
    )
    .unwrap();
    assert_eq!(
        result.profiles[0].profile.outer().segs().len(),
        result.profiles[1].profile.outer().segs().len()
    );
    check_sources(&circle, &result.profiles[0]);
    check_sources(&rectangle, &result.profiles[1]);
    assert_eq!(result.report.input_segments, 6);
    for original in rectangle.outer().segs() {
        assert!(
            result.profiles[1]
                .profile
                .outer()
                .segs()
                .iter()
                .any(|s| s.to == original.to),
            "every corner remains exact"
        );
    }
}

#[test]
fn explicit_landmarks_rotate_seams_and_feed_closed_ruled_and_smooth_lofts() {
    let a = builders::circle(10.0).unwrap();
    let b = builders::rect_centered(24.0, 16.0).unwrap();
    let c = builders::circle(8.0).unwrap();
    let circle_marks = [
        BoundaryPoint::new(0, 0.25),
        BoundaryPoint::new(0, 0.75),
        BoundaryPoint::new(1, 0.25),
        BoundaryPoint::new(1, 0.75),
    ];
    let rect_marks = core::array::from_fn::<_, 4, _>(|i| BoundaryPoint::new(i, 0.0));
    let result = correspond_profiles(
        &[
            CorrespondenceProfile::new(&a).with_landmarks(LoopRef::Outer, circle_marks),
            CorrespondenceProfile::new(&b).with_landmarks(LoopRef::Outer, rect_marks),
            CorrespondenceProfile::new(&c).with_landmarks(LoopRef::Outer, circle_marks),
        ],
        &POLICY,
    )
    .unwrap();
    assert_eq!(
        result.profiles[0].profile.outer().segs().len(),
        6,
        "four landmarks and both original circle seams are retained"
    );
    for (input, output) in [&a, &b, &c].into_iter().zip(&result.profiles) {
        check_sources(input, output);
    }
    let sections: Vec<_> = result
        .profiles
        .iter()
        .enumerate()
        .map(|(i, p)| (Placement3::translate(0.0, 0.0, 25.0 * i as f64), &p.profile))
        .collect();
    for policy in [LoftPolicy::Ruled, LoftPolicy::Smooth] {
        let output =
            tessellate_loft(&sections, policy, CapMode::Both, &EvalPolicy::default()).unwrap();
        assert!(output.mesh.validate_deep().is_empty());
        assert!(output.mesh.boundary_loops().unwrap().is_empty());
    }
}

#[test]
fn roundoff_close_knots_keep_each_sections_exact_corners() {
    let a = builders::rect_centered(10.0, 10.0).unwrap();
    let b = builders::rect_centered(10.0, 10.0 + 1e-13).unwrap();
    let result = correspond_profiles(
        &[
            CorrespondenceProfile::new(&a),
            CorrespondenceProfile::new(&b),
        ],
        &POLICY,
    )
    .unwrap();
    assert_eq!(result.profiles[0].profile, a);
    assert_eq!(result.profiles[1].profile, b);
    assert!(result.report.max_knot_adjustment > 0.0);
}

#[test]
fn invalid_landmarks_and_budgets_name_the_section_and_loop() {
    let a = builders::circle(10.0).unwrap();
    let b = builders::rect_centered(24.0, 16.0).unwrap();
    for points in [
        vec![BoundaryPoint::new(0, f64::NAN)],
        vec![
            BoundaryPoint::new(0, 0.5),
            BoundaryPoint::new(0, 0.4),
            BoundaryPoint::new(1, 0.0),
        ],
    ] {
        let err = correspond_profiles(
            &[
                CorrespondenceProfile::new(&a),
                CorrespondenceProfile::new(&b).with_landmarks(LoopRef::Outer, points),
            ],
            &POLICY,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CorrespondenceError::Landmarks {
                section: 1,
                loop_ref: LoopRef::Outer,
                ..
            }
        ));
    }
    let policy = CorrespondencePolicy {
        max_total_segments: 6,
        ..POLICY
    };
    assert!(matches!(
        correspond_profiles(
            &[
                CorrespondenceProfile::new(&a),
                CorrespondenceProfile::new(&b)
            ],
            &policy
        ),
        Err(CorrespondenceError::Budget {
            storage: "output segments",
            ..
        })
    ));
}

#[test]
fn holes_and_mixed_rounded_profiles_keep_exact_source_geometry() {
    let hole = builders::circle(3.0).unwrap();
    let a = builders::rect_centered(60.0, 40.0)
        .unwrap()
        .with_cutouts_at(&hole, [(-15.0, 0.0), (15.0, 0.0)])
        .unwrap();
    let b = builders::rounded_rect_centered(70.0, 44.0, 4.0)
        .unwrap()
        .with_cutouts_at(&hole, [(-15.0, 0.0), (15.0, 0.0)])
        .unwrap();
    let result = correspond_profiles(
        &[
            CorrespondenceProfile::new(&a),
            CorrespondenceProfile::new(&b),
        ],
        &POLICY,
    )
    .unwrap();
    assert_eq!(result.report.loops, 3);
    check_sources(&a, &result.profiles[0]);
    check_sources(&b, &result.profiles[1]);
    let sections = [
        (Placement3::IDENTITY, &result.profiles[0].profile),
        (
            Placement3::translate(0.0, 0.0, 20.0),
            &result.profiles[1].profile,
        ),
    ];
    let output = tessellate_loft(
        &sections,
        LoftPolicy::Ruled,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert!(output.mesh.validate_deep().is_empty());
    assert!(output.mesh.boundary_loops().unwrap().is_empty());
}

#[test]
fn close_corners_report_original_sources_and_can_be_aligned_explicitly() {
    let a = builders::rect_centered(10.0, 10.0).unwrap();
    let b = builders::rect_centered(10.0, 10.0 + 1e-7).unwrap();
    let policy = CorrespondencePolicy::new(ProfilePolicy::new(1e-5));
    let error = correspond_profiles(
        &[
            CorrespondenceProfile::new(&a),
            CorrespondenceProfile::new(&b),
        ],
        &policy,
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            CorrespondenceError::OutputValidation {
                sources: Some(_),
                ..
            }
        ),
        "{error:?}"
    );
    let marks = core::array::from_fn::<_, 4, _>(|i| BoundaryPoint::new(i, 0.0));
    let result = correspond_profiles(
        &[
            CorrespondenceProfile::new(&a).with_landmarks(LoopRef::Outer, marks),
            CorrespondenceProfile::new(&b).with_landmarks(LoopRef::Outer, marks),
        ],
        &policy,
    )
    .unwrap();
    assert_eq!(result.profiles[0].profile, a);
    assert_eq!(result.profiles[1].profile, b);
}

#[test]
fn cubics_and_missing_holes_keep_authored_refusal_context() {
    let a = builders::circle(10.0).unwrap();
    let cubic = Profile2::simple(
        Loop2::new(vec![
            Seg2::line((2.0, 0.0)),
            Seg2::cubic((2.0, 2.0), (3.0, 0.0), (3.0, 2.0)),
            Seg2::line((0.0, 2.0)),
            Seg2::line((0.0, 0.0)),
        ])
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        correspond_profiles(
            &[
                CorrespondenceProfile::new(&a),
                CorrespondenceProfile::new(&cubic)
            ],
            &POLICY
        ),
        Err(CorrespondenceError::Profile {
            section: 1,
            error: LineArcError::Unsupported { hole: None, seg: 1 }
        })
    ));
    let b = CorrespondenceProfile::new(&a)
        .with_landmarks(LoopRef::Hole(usize::MAX), [BoundaryPoint::new(0, 0.0)]);
    assert!(matches!(
        correspond_profiles(&[CorrespondenceProfile::new(&a), b], &POLICY),
        Err(CorrespondenceError::Landmarks {
            section: 1,
            reason: LandmarkError::MissingLoop,
            ..
        })
    ));
}

#[test]
fn normalized_resolution_never_discards_a_distinct_source_corner() {
    let a = builders::rect_centered(10.0, 1e-13).unwrap();
    let b = builders::rect_centered(10.0, 1.0).unwrap();
    let policy = CorrespondencePolicy::new(ProfilePolicy::new(1e-16));
    let error = correspond_profiles(
        &[
            CorrespondenceProfile::new(&a),
            CorrespondenceProfile::new(&b),
        ],
        &policy,
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            CorrespondenceError::Numeric {
                section: 0,
                points: Some(_),
                ..
            }
        ),
        "{error:?}"
    );
}
