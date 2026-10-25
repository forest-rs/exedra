// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::arrangement::piece;
use super::*;
use crate::arc::{Piece, length, sub};
use crate::{Loop, LoopRef, ProfilePolicy, Segment};
use crate::{RegionOp, RegionPolicy};
use alloc::vec;
use core::f64::consts::PI;
const POLICY: ProfilePolicy = ProfilePolicy::new(1e-9);

fn rectangle(min: [f64; 2], max: [f64; 2]) -> Profile {
    Profile::new(
        Loop::new(vec![
            Segment::line([max[0], min[1]]),
            Segment::line(max),
            Segment::line([min[0], max[1]]),
            Segment::line(min),
        ])
        .unwrap(),
        vec![],
        &POLICY,
    )
    .unwrap()
}
fn cross_profile() -> Profile {
    rectangle([-20.0, -5.0], [20.0, 5.0])
        .combine(
            &rectangle([-5.0, -20.0], [5.0, 20.0]),
            RegionOp::Union,
            &RegionPolicy::new(POLICY),
        )
        .unwrap()
        .into_single()
        .unwrap()
}

#[test]
fn four_concave_cross_corners_match_independent_area_and_tangent_oracles() {
    let cross = cross_profile();
    assert_eq!(
        cross
            .corners()
            .filter(|c| c.kind == CornerKind::Concave)
            .count(),
        4
    );
    assert_eq!(
        cross
            .corners()
            .filter(|c| c.kind == CornerKind::Convex)
            .count(),
        8
    );
    let selected = cross
        .corners()
        .filter(|c| c.kind == CornerKind::Concave)
        .map(|c| c.segment);
    let rounded = cross
        .with_corners(
            selected,
            CornerTreatment::fillet(1.0).tagged(SegTag(77)),
            &POLICY,
        )
        .unwrap();
    assert_eq!(rounded.changed_corners, 4);
    assert!((rounded.profile.area() - (700.0 + 4.0 - PI)).abs() < 1e-11);
    assert!((rounded.profile.outer().length() - (160.0 - 8.0 + 2.0 * PI)).abs() < 1e-11);
    assert_eq!(rounded.profile.outer().segments().len(), 16);
    let pieces: Vec<_> = rounded.profile.outer().pieces().collect();
    for (i, piece) in pieces.iter().enumerate() {
        if let Piece::Arc(arc) = piece {
            assert!((arc.radius - 1.0).abs() < 1e-12);
            assert!(arc.sweep < 0.0);
            assert_eq!(rounded.profile.outer().segments()[i].tag, Some(SegTag(77)));
            let before = pieces[(i + pieces.len() - 1) % pieces.len()].tangent(true);
            let after = pieces[(i + 1) % pieces.len()].tangent(false);
            assert!(cross_product(before, piece.tangent(false)).abs() < 1e-12);
            assert!(cross_product(piece.tangent(true), after).abs() < 1e-12);
        }
    }
    verify_retained_sources(&cross, &rounded);
}

fn cross_product(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

#[test]
fn chamfers_and_convex_fillet_selection_have_the_expected_material_sense() {
    let cross = cross_profile();
    let selected = cross
        .corners()
        .filter(|c| c.kind == CornerKind::Concave)
        .map(|c| c.segment);
    let chamfered = cross
        .with_corners(selected, CornerTreatment::chamfer(1.0), &POLICY)
        .unwrap();
    assert_eq!(chamfered.profile.area(), 702.0);
    assert!((chamfered.profile.outer().length() - (152.0 + 4.0 * libm::sqrt(2.0))).abs() < 1e-11);
    let rectangle = rectangle([0.0, 0.0], [10.0, 6.0]);
    let rounded = rectangle
        .with_corners(
            rectangle.corners().map(|c| c.segment),
            CornerTreatment::fillet(1.0),
            &POLICY,
        )
        .unwrap();
    assert!((rounded.profile.area() - (60.0 - 4.0 + PI)).abs() < 1e-12);
    assert_eq!(rounded.profile.outer().segments().len(), 8);
    verify_retained_sources(&rectangle, &rounded);
}

#[test]
fn holes_classify_material_concavity_and_unselected_arcs_keep_their_tags_and_geometry() {
    let outer = rectangle([0.0, 0.0], [10.0, 10.0]);
    let hole = rectangle([2.0, 2.0], [6.0, 6.0]).outer().reversed();
    let profile = Profile::new(outer.outer().clone(), vec![hole], &POLICY).unwrap();
    let holes: Vec<_> = profile
        .corners()
        .filter(|c| c.segment.loop_ref == LoopRef::Hole(0))
        .collect();
    assert!(holes.iter().all(|c| c.kind == CornerKind::Concave));
    let rounded = profile
        .with_corners(
            holes.iter().map(|c| c.segment),
            CornerTreatment::fillet(0.5),
            &POLICY,
        )
        .unwrap();
    assert!((rounded.profile.area() - (84.0 + 1.0 - PI * 0.25)).abs() < 1e-12);
    let mixed = Profile::new(
        Loop::new(vec![
            Segment::line([4.0, 0.0]),
            Segment::line([4.0, 2.0]),
            Segment::arc([0.0, 2.0], 1.0).tagged(SegTag(9)),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap(),
        vec![],
        &POLICY,
    )
    .unwrap();
    let result = mixed
        .with_corners(
            [SegmentRef {
                loop_ref: LoopRef::Outer,
                segment: 0,
            }],
            CornerTreatment::fillet(0.5),
            &POLICY,
        )
        .unwrap();
    assert_eq!(
        result
            .profile
            .outer()
            .segments()
            .iter()
            .find(|s| s.tag == Some(SegTag(9))),
        Some(&mixed.outer().segments()[2])
    );
    verify_retained_sources(&mixed, &result);
}

#[test]
fn corner_refusals_name_the_dimension_junction_and_limiting_edge() {
    let profile = rectangle([0.0, 0.0], [2.0, 1.0]);
    let first = SegmentRef {
        loop_ref: LoopRef::Outer,
        segment: 0,
    };
    assert_eq!(
        profile.with_corners([first], CornerTreatment::fillet(-1.0), &POLICY),
        Err(CornerError::InvalidSize {
            shape: CornerShape::Fillet { radius: -1.0 }
        })
    );
    let unknown = SegmentRef {
        loop_ref: LoopRef::Hole(usize::MAX),
        segment: 0,
    };
    assert_eq!(
        profile.with_corners([unknown], CornerTreatment::chamfer(0.1), &POLICY),
        Err(CornerError::UnknownCorner { corner: unknown })
    );
    assert_eq!(
        profile.with_corners([first, first], CornerTreatment::fillet(0.1), &POLICY),
        Err(CornerError::RepeatedCorner { corner: first })
    );
    assert!(
        matches!(profile.with_corners(profile.corners().map(|c|c.segment),CornerTreatment::fillet(0.75),&POLICY),Err(CornerError::OverlappingTrims {length:1.0,requested,min_separation:1e-9,..}) if (requested-1.5).abs()<1e-12)
    );
    let disk = Profile::new(
        Loop::new(vec![
            Segment::arc([-1.0, 0.0], 1.0),
            Segment::arc([1.0, 0.0], 1.0),
        ])
        .unwrap(),
        vec![],
        &POLICY,
    )
    .unwrap();
    assert_eq!(
        disk.with_corners([first], CornerTreatment::fillet(0.1), &POLICY),
        Err(CornerError::SmoothCorner { corner: first })
    );
    let mixed = Profile::new(
        Loop::new(vec![
            Segment::line([4.0, 0.0]),
            Segment::line([4.0, 2.0]),
            Segment::arc([0.0, 2.0], 0.5),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap(),
        vec![],
        &POLICY,
    )
    .unwrap();
    assert!(matches!(
        mixed.with_corners(
            [SegmentRef {
                loop_ref: LoopRef::Outer,
                segment: 1
            }],
            CornerTreatment::fillet(0.1),
            &POLICY
        ),
        Err(CornerError::UnsupportedCorner { .. })
    ));
}

fn verify_retained_sources(input: &Profile, result: &CornerResult) {
    for item in &result.segments {
        if let CornerSource::Segment {
            segment,
            parameters,
        } = item.source
        {
            let source = match segment.loop_ref {
                LoopRef::Outer => input.outer(),
                LoopRef::Hole(i) => &input.holes()[i],
            };
            let output = match item.segment.loop_ref {
                LoopRef::Outer => result.profile.outer(),
                LoopRef::Hole(i) => &result.profile.holes()[i],
            };
            let before = piece(source, segment.segment);
            let after = piece(output, item.segment.segment);
            let source_point = |t| match before {
                Piece::Line(a, b) => [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])],
                Piece::Arc(a) => a.point_at(t),
            };
            assert!(length(sub(after.from(), source_point(parameters[0]))) < 1e-12);
            assert!(length(sub(after.to(), source_point(parameters[1]))) < 1e-12);
        }
    }
}
