// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::arrangement::point_at;
use super::*;
use crate::profile::SegTag;
use crate::profile::arc::{length, sub};
use crate::profile::{Curve, Loop, LoopRef, Segment};
use alloc::vec;
use core::f64::consts::PI;

const POLICY: RegionPolicy = RegionPolicy::new(ProfilePolicy::new(1e-9));

fn rect(min: [f64; 2], max: [f64; 2]) -> Profile {
    Profile::new(
        Loop::new(vec![
            Segment::line([max[0], min[1]]),
            Segment::line(max),
            Segment::line([min[0], max[1]]),
            Segment::line(min),
        ])
        .unwrap(),
        vec![],
        &POLICY.profile,
    )
    .unwrap()
}
fn disk(center: [f64; 2], r: f64) -> Profile {
    let a = [center[0] + r, center[1]];
    let b = [center[0] - r, center[1]];
    Profile::new(
        Loop::new(vec![Segment::arc(b, 1.0), Segment::arc(a, 1.0)]).unwrap(),
        vec![],
        &POLICY.profile,
    )
    .unwrap()
}
fn area(result: &RegionResult) -> f64 {
    result.profiles.iter().map(Profile::area).sum()
}

#[test]
fn overlapping_rectangles_form_a_cross_without_approximating_the_boundary() {
    let a = rect([-20.0, -5.0], [20.0, 5.0]);
    let b = rect([-5.0, -20.0], [5.0, 20.0]);
    let combined = a.combine(&b, RegionOp::Union, &POLICY).unwrap();
    assert_eq!(combined.profiles.len(), 1);
    assert_eq!(combined.profiles[0].outer().segments().len(), 12);
    assert_eq!(area(&combined), 700.0);
    assert_eq!(combined.report.pair_tests, 16);
    assert_eq!(
        area(&a.combine(&b, RegionOp::Intersection, &POLICY).unwrap()),
        100.0
    );
    assert_eq!(
        area(&a.combine(&b, RegionOp::Difference, &POLICY).unwrap()),
        300.0
    );
    assert_eq!(combined, a.combine(&b, RegionOp::Union, &POLICY).unwrap());
}

#[test]
fn empty_disconnected_and_nested_results_keep_their_filled_region_meaning() {
    let a = rect([-2.0, -1.0], [2.0, 1.0]);
    let b = rect([-0.5, -2.0], [0.5, 2.0]);
    let split = a.combine(&b, RegionOp::Difference, &POLICY).unwrap();
    assert_eq!(split.profiles.len(), 2);
    assert_eq!(area(&split), 6.0);
    assert_eq!(
        split.clone().into_single(),
        Err(RegionError::ExpectedOne { count: 2 })
    );
    let far = rect([20.0, 0.0], [21.0, 1.0]);
    assert!(
        a.combine(&far, RegionOp::Intersection, &POLICY)
            .unwrap()
            .profiles
            .is_empty()
    );
    assert_eq!(
        a.combine(&far, RegionOp::Union, &POLICY)
            .unwrap()
            .profiles
            .len(),
        2
    );
    let outer = rect([-5.0, -5.0], [5.0, 5.0]);
    let ring = Profile::new(
        disk([0.0, 0.0], 3.0).outer().clone(),
        vec![disk([0.0, 0.0], 1.0).outer().reversed()],
        &POLICY.profile,
    )
    .unwrap();
    let result = outer.combine(&ring, RegionOp::Difference, &POLICY).unwrap();
    assert_eq!(result.profiles.len(), 2);
    assert!((area(&result) - (100.0 - 8.0 * PI)).abs() < 1e-11);
    assert_eq!(
        result
            .profiles
            .iter()
            .map(|p| p.holes().len())
            .sum::<usize>(),
        1
    );
}

#[test]
fn circular_intersections_keep_arcs_and_agree_with_the_independent_lens_formula() {
    let a = disk([0.0, 0.0], 2.0);
    let b = disk([2.0, 0.0], 2.0);
    let lens = 8.0 * PI / 3.0 - 2.0 * libm::sqrt(3.0);
    for (op, expected) in [
        (RegionOp::Intersection, lens),
        (RegionOp::Union, 8.0 * PI - lens),
        (RegionOp::Difference, 4.0 * PI - lens),
    ] {
        let result = a.combine(&b, op, &POLICY).unwrap();
        assert!(
            (area(&result) - expected).abs() < 1e-11,
            "{op:?}: {} versus {expected}",
            area(&result)
        );
        assert!(
            result
                .profiles
                .iter()
                .flat_map(|p| p.outer().segments())
                .all(|s| matches!(s.curve, Curve::Arc { .. }))
        );
        verify_sources(&result, [&a, &b]);
    }
}

#[test]
fn coincident_runs_record_both_sources_and_prefer_the_left_label() {
    let labelled = |tag| {
        let p = rect([0.0, 0.0], [2.0, 1.0]);
        let outer = Loop::new(
            p.outer()
                .segments()
                .iter()
                .cloned()
                .map(|s| s.tagged(SegTag(tag)))
                .collect(),
        )
        .unwrap();
        Profile::new(outer, vec![], &POLICY.profile).unwrap()
    };
    let a = labelled(10);
    let b = labelled(20);
    for op in [RegionOp::Union, RegionOp::Intersection] {
        let result = a.combine(&b, op, &POLICY).unwrap();
        assert_eq!(area(&result), 2.0);
        assert!(
            result
                .segments
                .iter()
                .all(|s| s.sources.iter().all(Option::is_some))
        );
        assert!(
            result.profiles[0]
                .outer()
                .segments()
                .iter()
                .all(|s| s.tag == Some(SegTag(10)))
        );
        verify_sources(&result, [&a, &b]);
    }
    assert!(
        a.combine(&b, RegionOp::Difference, &POLICY)
            .unwrap()
            .profiles
            .is_empty()
    );
    let adjacent = rect([2.0, 0.0], [4.0, 1.0]);
    assert_eq!(
        area(&a.combine(&adjacent, RegionOp::Union, &POLICY).unwrap()),
        4.0
    );
}

#[test]
fn coincident_circular_seams_and_opposite_material_sides_are_resolved() {
    let a = disk([0.0, 0.0], 1.0);
    let major = Profile::new(
        Loop::new(vec![
            Segment::arc([0.0, -1.0], libm::tan(3.0 * PI / 8.0)),
            Segment::arc([1.0, 0.0], libm::tan(PI / 8.0)),
        ])
        .unwrap(),
        vec![],
        &POLICY.profile,
    )
    .unwrap();
    let union = a.combine(&major, RegionOp::Union, &POLICY).unwrap();
    assert!((area(&union) - PI).abs() < 1e-12);
    assert!(
        union
            .segments
            .iter()
            .all(|s| s.sources.iter().all(Option::is_some))
    );
    verify_sources(&union, [&a, &major]);
    let ring = Profile::new(
        disk([0.0, 0.0], 2.0).outer().clone(),
        vec![a.outer().reversed()],
        &POLICY.profile,
    )
    .unwrap();
    assert!((area(&a.combine(&ring, RegionOp::Union, &POLICY).unwrap()) - 4.0 * PI).abs() < 1e-12);
    assert!(
        a.combine(&ring, RegionOp::Intersection, &POLICY)
            .unwrap()
            .profiles
            .is_empty()
    );
    assert!((area(&a.combine(&ring, RegionOp::Difference, &POLICY).unwrap()) - PI).abs() < 1e-12);
}

#[test]
fn a_caller_can_raise_the_reported_event_budget_until_the_operation_completes() {
    let a = rect([0.0, 0.0], [2.0, 1.0]);
    let mut policy = RegionPolicy {
        max_events: 1,
        ..POLICY
    };
    let mut result = None;
    for _ in 0..64 {
        match a.combine(&a, RegionOp::Union, &policy) {
            Err(RegionError::Budget {
                resource: RegionBudget::Events,
                required,
                ..
            }) => policy.max_events = u32::try_from(required).unwrap(),
            Ok(value) => {
                result = Some(value);
                break;
            }
            other => panic!("unexpected recovery result: {other:?}"),
        }
    }
    assert_eq!(
        area(&result.expect("reported budgets permit bounded recovery")),
        2.0
    );
    let zero = RegionPolicy {
        max_events: 0,
        ..POLICY
    };
    assert_eq!(
        a.combine(&a, RegionOp::Union, &zero),
        Err(RegionError::InvalidBudget {
            resource: RegionBudget::Events,
            maximum: 0
        })
    );
}

#[test]
fn tangential_branching_and_work_budget_refusals_are_explicit() {
    let a = rect([0.0, 0.0], [1.0, 1.0]);
    let b = rect([1.0, 1.0], [2.0, 2.0]);
    assert!(matches!(
        a.combine(&b, RegionOp::Union, &POLICY),
        Err(RegionError::NonManifold { .. })
    ));
    assert!(
        a.combine(&b, RegionOp::Intersection, &POLICY)
            .unwrap()
            .profiles
            .is_empty()
    );
    let policy = RegionPolicy {
        max_pair_tests: 1,
        ..POLICY
    };
    assert_eq!(
        a.combine(&b, RegionOp::Union, &policy),
        Err(RegionError::Budget {
            resource: RegionBudget::PairTests,
            maximum: 1,
            required: 2
        })
    );
    let policy = RegionPolicy {
        max_events: 1,
        ..POLICY
    };
    assert!(matches!(
        a.combine(&a, RegionOp::Union, &policy),
        Err(RegionError::Budget {
            resource: RegionBudget::Events,
            ..
        })
    ));
    let policy = RegionPolicy {
        max_fragments: 1,
        ..POLICY
    };
    assert!(matches!(
        a.combine(&a, RegionOp::Union, &policy),
        Err(RegionError::Budget {
            resource: RegionBudget::Fragments,
            ..
        })
    ));
    let policy = RegionPolicy {
        max_classification_tests: 1,
        ..POLICY
    };
    assert_eq!(
        a.combine(&a, RegionOp::Union, &policy),
        Err(RegionError::Budget {
            resource: RegionBudget::ClassificationTests,
            maximum: 1,
            required: 2
        })
    );
}

#[test]
fn line_arc_crossings_major_arcs_and_rescaled_coordinates_keep_the_same_region() {
    let disk = disk([0.0, 0.0], 2.0);
    let half = rect([0.0, -3.0], [3.0, 3.0]);
    let result = disk
        .combine(&half, RegionOp::Intersection, &POLICY)
        .unwrap();
    assert!((area(&result) - 2.0 * PI).abs() < 1e-12);
    verify_sources(&result, [&disk, &half]);
    let major = Loop::new(vec![
        Segment::arc([0.0, -1.0], libm::tan(3.0 * PI / 8.0)),
        Segment::arc([1.0, 0.0], libm::tan(PI / 8.0)),
    ])
    .unwrap();
    let major = Profile::new(major, vec![], &POLICY.profile).unwrap();
    let result = major
        .combine(&half, RegionOp::Intersection, &POLICY)
        .unwrap();
    assert!((area(&result) - 0.5 * PI).abs() < 1e-12);
    verify_sources(&result, [&major, &half]);
    for scale in [1e-90, 1e-6, 1.0, 1e6, 1e90] {
        let tx = 1000.0 * scale;
        let ty = -2000.0 * scale;
        let transformed = |p: &Profile| {
            let outer = Loop::new(
                p.outer()
                    .segments()
                    .iter()
                    .map(|s| Segment {
                        to: [tx + s.to[0] * scale, ty + s.to[1] * scale],
                        ..s.clone()
                    })
                    .collect(),
            )
            .unwrap();
            Profile::new(outer, vec![], &ProfilePolicy::new(1e-9 * scale)).unwrap()
        };
        let a = transformed(&rect([-20.0, -5.0], [20.0, 5.0]));
        let b = transformed(&rect([-5.0, -20.0], [5.0, 20.0]));
        let result = a
            .combine(
                &b,
                RegionOp::Union,
                &RegionPolicy::new(ProfilePolicy::new(1e-9 * scale)),
            )
            .unwrap();
        assert!((area(&result) / (scale * scale) - 700.0).abs() < 1e-8);
    }
}

fn verify_sources(result: &RegionResult, inputs: [&Profile; 2]) {
    for correspondence in &result.segments {
        let p = &result.profiles[correspondence.profile];
        let loop_ = match correspondence.segment.loop_ref {
            LoopRef::Outer => p.outer(),
            LoopRef::Hole(i) => &p.holes()[i],
        };
        let piece = loop_.pieces().nth(correspondence.segment.segment).unwrap();
        for (operand, source) in correspondence.sources.iter().enumerate() {
            if let Some(source) = source {
                let profile = inputs[operand];
                let loop_ = match source.segment.loop_ref {
                    LoopRef::Outer => profile.outer(),
                    LoopRef::Hole(i) => &profile.holes()[i],
                };
                let original = loop_.pieces().nth(source.segment.segment).unwrap();
                for i in 0..=16 {
                    let t = f64::from(i) / 16.0;
                    let expected = point_at(
                        &original,
                        source.parameters[0] + t * (source.parameters[1] - source.parameters[0]),
                    );
                    assert!(length(sub(point_at(&piece, t), expected)) < 1e-11);
                }
            }
        }
    }
}

#[test]
fn many_circle_pairs_match_independent_lens_areas_and_point_membership() {
    let mut state = 12345_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        f64::from((state >> 32) as u32) / f64::from(u32::MAX)
    };
    for _ in 0..80 {
        let ca = [4.0 * next() - 2.0, 4.0 * next() - 2.0];
        let cb = [4.0 * next() - 2.0, 4.0 * next() - 2.0];
        let (ra, rb) = (0.25 + 2.0 * next(), 0.25 + 2.0 * next());
        let a = disk(ca, ra);
        let b = disk(cb, rb);
        let d = length(sub(cb, ca));
        let overlap = if d >= ra + rb {
            0.0
        } else if d <= (ra - rb).abs() {
            PI * ra.min(rb).powi(2)
        } else {
            let alpha = libm::acos(((d * d + ra * ra - rb * rb) / (2.0 * d * ra)).clamp(-1.0, 1.0));
            let beta = libm::acos(((d * d + rb * rb - ra * ra) / (2.0 * d * rb)).clamp(-1.0, 1.0));
            ra * ra * alpha + rb * rb * beta
                - 0.5 * libm::sqrt((-d + ra + rb) * (d + ra - rb) * (d - ra + rb) * (d + ra + rb))
        };
        for (op, expected) in [
            (RegionOp::Intersection, overlap),
            (RegionOp::Union, PI * (ra * ra + rb * rb) - overlap),
            (RegionOp::Difference, PI * ra * ra - overlap),
        ] {
            let result = a.combine(&b, op, &POLICY).unwrap();
            assert!(
                (area(&result) - expected).abs() < 1e-9,
                "{op:?}: {ca:?}/{ra} {cb:?}/{rb}"
            );
            verify_sources(&result, [&a, &b]);
            for x in -10..=10 {
                for y in -10..=10 {
                    let point = [f64::from(x) * 0.4, f64::from(y) * 0.4];
                    if (length(sub(point, ca)) - ra).abs() < 1e-7
                        || (length(sub(point, cb)) - rb).abs() < 1e-7
                    {
                        continue;
                    }
                    let expected =
                        op.contains(length(sub(point, ca)) < ra, length(sub(point, cb)) < rb);
                    let actual = result.profiles.iter().any(|p| {
                        p.outer().winding(point) != 0
                            && p.holes().iter().all(|h| h.winding(point) == 0)
                    });
                    assert_eq!(actual, expected, "{op:?} at {point:?}");
                }
            }
        }
    }
}

#[test]
fn center_defined_arcs_keep_their_center_and_sources_when_split_by_regions() {
    let centered_disk = |center: [f64; 2]| {
        Profile::new(
            Loop::new(vec![
                Segment::arc_centered([center[0] - 2.0, center[1]], center, false)
                    .tagged(SegTag(23)),
                Segment::arc_centered([center[0] + 2.0, center[1]], center, false)
                    .tagged(SegTag(23)),
            ])
            .unwrap(),
            vec![],
            &POLICY.profile,
        )
        .unwrap()
    };
    let a = centered_disk([0.0; 2]);
    let b = centered_disk([2.0, 0.0]);
    let lens = 8.0 * PI / 3.0 - 2.0 * libm::sqrt(3.0);
    for (op, expected) in [
        (RegionOp::Intersection, lens),
        (RegionOp::Union, 8.0 * PI - lens),
        (RegionOp::Difference, 4.0 * PI - lens),
    ] {
        let result = a.combine(&b, op, &POLICY).unwrap();
        assert!((area(&result) - expected).abs() < 1e-11);
        for profile in &result.profiles {
            for segment in profile.outer().segments() {
                let Curve::ArcCenter { center, .. } = segment.curve else {
                    panic!("region replaced an authored center with derived bulge");
                };
                assert!(center == [0.0; 2] || center == [2.0, 0.0]);
                assert_eq!(segment.tag, Some(SegTag(23)));
            }
        }
        verify_sources(&result, [&a, &b]);
    }
}
