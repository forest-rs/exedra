// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::super::{OffsetBudget, OffsetPolicy};
use super::*;

fn crossing(scale: f64) -> System {
    System::Curves(
        Curve::line(Point::new(-scale, 0.0), Point::new(scale, 0.0)),
        Curve::cubic(CubicBez::new(
            (0.0, -scale),
            (scale * 0.2, -scale / 3.0),
            (-scale * 0.2, scale / 3.0),
            (0.0, scale),
        )),
    )
}
#[test]
fn isolated_crossings_at_multiple_scales() {
    for scale in [1e-6, 1.0, 1e6] {
        let mut context = OffsetContext::default();
        let roots =
            solve::roots(crossing(scale), scale * 1e-7, &mut context, None, 0).expect("crossing");
        assert_eq!(roots.len(), 1);
        assert!(roots[0].point.distance(Point::ZERO) <= roots[0].bound);
        assert!(roots[0].bound <= scale * 1e-7);
        for t in roots[0].parameters {
            assert!((t - 0.5).abs() < 1e-6);
        }
    }
}
#[test]
fn circle_crossings_are_both_reported() {
    let line = Curve::line(Point::new(-2.0, 0.0), Point::new(2.0, 0.0));
    let roots = solve::roots(
        System::Circle(line, Point::ZERO, 1.0),
        1e-6,
        &mut OffsetContext::default(),
        None,
        0,
    )
    .expect("circle roots");
    assert_eq!(roots.len(), 2);
    for r in roots {
        assert!((r.point.to_vec2().hypot() - 1.0).abs() <= r.bound);
    }
}
#[test]
fn tangencies_endpoints_and_coincidence_are_refused() {
    let horizontal = Curve::line(Point::new(-2.0, 1.0), Point::new(2.0, 1.0));
    for system in [
        System::Circle(horizontal, Point::ZERO, 1.0),
        System::Curves(horizontal, horizontal),
        System::Curves(
            horizontal,
            Curve::line(Point::new(2.0, 0.0), Point::new(2.0, 2.0)),
        ),
    ] {
        assert!(matches!(
            solve::roots(system, 1e-6, &mut OffsetContext::default(), None, 0),
            Err(ProfileError::OffsetTrimUnresolved { .. }
                | ProfileError::OffsetBudgetExceeded {
                    budget: OffsetBudget::TrimSteps,
                    ..
                })
        ));
    }
}
#[test]
fn trim_budget_is_charged_before_search() {
    let mut context = OffsetContext {
        policy: Some(OffsetPolicy {
            max_trim_steps: 0,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(matches!(
        solve::roots(crossing(1.0), 1e-6, &mut context, None, 0),
        Err(ProfileError::OffsetBudgetExceeded {
            budget: OffsetBudget::TrimSteps,
            maximum: 0
        })
    ));
}

#[test]
fn competing_roots_do_not_choose_a_nearest_join() {
    let first = Piece {
        start: Point::new(0.0, 0.0),
        end: Point::new(1.0, 0.0),
        start_normal: kurbo::Vec2::new(0.0, -1.0),
        end_normal: kurbo::Vec2::new(0.0, -1.0),
        kind: PieceKind::Line,
        tag: None,
        policy: None,
    };
    // y(t) = (t - 1/4)(t - 3/4): two transverse crossings.
    let second = Piece {
        start: Point::new(0.0, 0.1875),
        end: Point::new(1.0, 0.1875),
        kind: PieceKind::Fitted(alloc::vec![Seg2::cubic(
            (1.0, 0.1875),
            (1.0 / 3.0, 0.1875 - 1.0 / 3.0),
            (2.0 / 3.0, 0.1875 - 1.0 / 3.0)
        )]),
        ..first
    };
    assert!(matches!(
        corner(
            &first,
            &second,
            1e-6,
            None,
            4,
            &mut OffsetContext::default()
        ),
        Err(ProfileError::OffsetTrimAmbiguous { hole: None, seg: 4 })
    ));
}

#[test]
fn consumed_fitted_runs_are_refused() {
    let piece = Piece {
        start: Point::ZERO,
        end: Point::new(1.0, 0.0),
        start_normal: kurbo::Vec2::ZERO,
        end_normal: kurbo::Vec2::ZERO,
        kind: PieceKind::Fitted(alloc::vec![Seg2::line((1.0, 0.0))]),
        tag: None,
        policy: None,
    };
    assert!(matches!(
        check_cuts(
            &piece,
            Some(Cut {
                piece: 0,
                t: 0.8,
                bounds: [0.8; 2],
                evidence: 0,
                tolerance: 1e-6
            }),
            Some(Cut {
                piece: 0,
                t: 0.2,
                bounds: [0.2; 2],
                evidence: 1,
                tolerance: 1e-6
            }),
            None,
            0
        ),
        Err(ProfileError::OffsetLoopDegenerate { hole: None })
    ));
}

#[test]
fn uncertain_cut_order_does_not_turn_a_consumed_curve_into_a_sliver() {
    let make_line = |slope: f64, t: f64| {
        let center = Point::new(3.0 * t, t * t * t);
        Piece {
            start: center - kurbo::Vec2::new(2.0, 2.0 * slope),
            end: center + kurbo::Vec2::new(2.0, 2.0 * slope),
            start_normal: kurbo::Vec2::ZERO,
            end_normal: kurbo::Vec2::ZERO,
            kind: PieceKind::Line,
            tag: None,
            policy: None,
        }
    };
    let before = make_line(7.0, 0.5);
    let after = make_line(0.0, 0.49999999999);
    let middle = Piece {
        start: Point::ZERO,
        end: Point::new(3.0, 1.0),
        kind: PieceKind::Fitted(alloc::vec![Seg2::cubic((3.0, 1.0), (1.0, 0.0), (2.0, 0.0))]),
        ..before
    };
    let mut context = OffsetContext::default();
    let a = corner(&before, &middle, 0.01, None, 0, &mut context).expect("isolated start");
    let b = corner(&middle, &after, 0.01, None, 1, &mut context).expect("isolated end");
    let start = a.cuts.unwrap()[1];
    let end = b.cuts.unwrap()[0];
    assert!(matches!(
        check_cuts(&middle, Some(start), Some(end), None, 1),
        Err(ProfileError::OffsetTrimUnresolved { .. } | ProfileError::OffsetLoopDegenerate { .. })
    ));
}

fn several_fitted_pieces() -> (Piece, CubicBez) {
    let cubic = CubicBez::new((0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 1.0));
    let segments = (0..3)
        .map(|i| {
            let c = cubic.subsegment(f64::from(i) / 3.0..f64::from(i + 1) / 3.0);
            Seg2::cubic(c.p3, c.p1, c.p2)
        })
        .collect();
    (
        Piece {
            start: cubic.p0,
            end: cubic.p3,
            start_normal: kurbo::Vec2::ZERO,
            end_normal: kurbo::Vec2::ZERO,
            kind: PieceKind::Fitted(segments),
            tag: Some(crate::profile::SegTag(9)),
            policy: Some(crate::ir::PolicyId(7)),
        },
        cubic,
    )
}
fn horizontal_through(point: Point) -> Piece {
    Piece {
        start: point - kurbo::Vec2::new(4.0, 0.0),
        end: point + kurbo::Vec2::new(4.0, 0.0),
        start_normal: kurbo::Vec2::ZERO,
        end_normal: kurbo::Vec2::ZERO,
        kind: PieceKind::Line,
        tag: None,
        policy: None,
    }
}
#[test]
fn trimming_discards_consumed_pieces_and_preserves_the_remaining_curve() {
    let (piece, cubic) = several_fitted_pieces();
    let mut context = OffsetContext::default();
    let a = corner(
        &horizontal_through(cubic.eval(0.45)),
        &piece,
        1e-8,
        None,
        0,
        &mut context,
    )
    .unwrap();
    let b = corner(
        &piece,
        &horizontal_through(cubic.eval(0.9)),
        1e-8,
        None,
        1,
        &mut context,
    )
    .unwrap();
    let start = a.cuts.unwrap()[1];
    let end = b.cuts.unwrap()[0];
    assert_eq!((start.piece, end.piece), (1, 2));
    check_cuts(&piece, Some(start), Some(end), None, 1).unwrap();
    let PieceKind::Fitted(original) = &piece.kind else {
        unreachable!()
    };
    let output = emit(
        &piece,
        original,
        Some(start),
        Some(end),
        a.start,
        b.end,
        None,
        &mut context,
    )
    .unwrap();
    assert_eq!(output.len(), 2);
    let mut from = a.start;
    for (segment, (lo, hi)) in output.iter().zip([(0.45, 2.0 / 3.0), (2.0 / 3.0, 0.9)]) {
        assert_eq!(segment.tag, piece.tag);
        assert!(matches!(
            segment.kind,
            SegKind::PolicyTo {
                policy: crate::ir::PolicyId(7),
                ..
            }
        ));
        let SegKind::Cubic { c1, c2 } = unwrapped(&segment.kind) else {
            panic!("retained cubic")
        };
        let emitted = CubicBez::new(from, *c1, *c2, segment.to);
        for i in 0..=20 {
            let t = f64::from(i) / 20.0;
            assert!(emitted.eval(t).distance(cubic.eval(lo + (hi - lo) * t)) < 2e-8);
        }
        from = segment.to;
    }
}
#[test]
fn an_intersection_at_an_internal_fitted_knot_is_explicitly_unresolved() {
    let (piece, _) = several_fitted_pieces();
    let PieceKind::Fitted(segments) = &piece.kind else {
        unreachable!()
    };
    let crossing = horizontal_through(segments[0].to);
    assert!(matches!(
        corner(
            &crossing,
            &piece,
            1e-6,
            None,
            0,
            &mut OffsetContext::default()
        ),
        Err(ProfileError::OffsetTrimUnresolved { .. }
            | ProfileError::OffsetBudgetExceeded {
                budget: OffsetBudget::TrimSteps,
                ..
            })
    ));
}
