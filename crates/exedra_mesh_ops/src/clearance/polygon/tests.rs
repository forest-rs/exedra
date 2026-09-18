// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::clearance::{BoundaryStats, PatchLoop};
use exedra_math::Placement3;

fn patch(outer: &[[f64; 2]], holes: &[&[[f64; 2]]]) -> PlanarPatch<u32> {
    let loops: Vec<_> = core::iter::once(outer)
        .chain(holes.iter().copied())
        .enumerate()
        .map(|(i, points)| PatchLoop {
            points: points.to_vec(),
            sources: (0..points.len()).map(|j| index(i * 100 + j)).collect(),
        })
        .collect();
    PlanarPatch {
        frame: Placement3::IDENTITY,
        max_plane_deviation: 0.0,
        stats: BoundaryStats::default(),
        loops,
    }
}
fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> [[f64; 2]; 4] {
    [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}
fn support() -> PlanarPatch<u32> {
    patch(
        &rectangle(0.0, 0.0, 10.0, 10.0),
        &[&[[4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0]]],
    )
}
#[test]
fn contains_filled_polygons_and_distinguishes_hole_coverage_from_distance() {
    let support = support();
    let policy = PolygonClearancePolicy::default();
    let fit = support
        .polygon_clearance(&rectangle(1.0, 1.0, 3.0, 3.0), &policy)
        .unwrap();
    assert!(fit.is_contained());
    assert_eq!(fit.boundary_distance, 1.0);
    assert_eq!(
        fit.classify(0.5, 0.01).unwrap(),
        ClearanceDecision::Satisfied
    );
    assert_eq!(
        fit.classify(1.0, 0.01).unwrap(),
        ClearanceDecision::WithinTolerance
    );
    assert_eq!(
        fit.classify(2.0, 0.01).unwrap(),
        ClearanceDecision::Violated
    );
    let hole = support
        .polygon_clearance(&rectangle(3.0, 3.0, 7.0, 7.0), &policy)
        .unwrap();
    assert_eq!(hole.boundary_distance, 1.0);
    assert!(matches!(
        hole.violation,
        Some(PolygonViolation::CoveredHole {
            boundary: BoundaryWitness {
                loop_index: 1,
                source: 100,
                ..
            }
        })
    ));
    assert_eq!(
        hole.classify(0.0, 100.0).unwrap(),
        ClearanceDecision::Violated,
        "decision tolerance cannot fill a hole"
    );
    let crossing = support
        .polygon_clearance(&rectangle(3.0, 5.0, 7.0, 7.0), &policy)
        .unwrap();
    assert_eq!(crossing.boundary_distance, 0.0);
    assert!(matches!(
        crossing.violation,
        Some(PolygonViolation::BoundaryCrossing { .. })
    ));
    let outside = support
        .polygon_clearance(&rectangle(11.0, 2.0, 12.0, 3.0), &policy)
        .unwrap();
    assert_eq!(outside.boundary_distance, 1.0);
    assert!(matches!(
        outside.violation,
        Some(PolygonViolation::Outside { .. })
    ));
}
#[test]
fn contact_is_contained_but_coincident_hole_coverage_is_not() {
    let support = support();
    let policy = PolygonClearancePolicy::default();
    for mut points in [rectangle(0.0, 0.0, 2.0, 2.0), rectangle(2.0, 4.0, 4.0, 6.0)] {
        for _ in 0..2 {
            let result = support.polygon_clearance(&points, &policy).unwrap();
            assert!(result.is_contained(), "{result:?}");
            assert_eq!(result.boundary_distance, 0.0);
            assert_eq!(
                result.classify(0.0, 0.0).unwrap(),
                ClearanceDecision::WithinTolerance
            );
            points.reverse();
        }
    }
    let mut hole = rectangle(4.0, 4.0, 6.0, 6.0);
    for _ in 0..2 {
        let result = support.polygon_clearance(&hole, &policy).unwrap();
        assert!(matches!(
            result.violation,
            Some(PolygonViolation::CoveredHole { .. })
        ));
        assert_eq!(
            result.classify(0.0, 0.0).unwrap(),
            ClearanceDecision::Violated
        );
        hole.reverse();
    }
}
#[test]
fn concave_footprints_fit_while_notch_crossings_fail_even_at_boundary_vertices() {
    let policy = PolygonClearancePolicy::default();
    let points = [
        [1.0, 1.0],
        [9.0, 1.0],
        [9.0, 3.0],
        [3.0, 3.0],
        [3.0, 9.0],
        [1.0, 9.0],
    ];
    assert!(
        support()
            .polygon_clearance(&points, &policy)
            .unwrap()
            .is_contained()
    );
    let notch = patch(
        &[
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [7.0, 10.0],
            [7.0, 6.0],
            [6.0, 4.0],
            [4.0, 4.0],
            [3.0, 6.0],
            [3.0, 10.0],
            [0.0, 10.0],
        ],
        &[],
    );
    let points = rectangle(1.0, 2.0, 9.0, 6.0);
    assert!(
        points.iter().all(|&p| contains(notch.loops[0].points(), p)),
        "all corners fit"
    );
    let result = notch.polygon_clearance(&points, &policy).unwrap();
    assert!(
        matches!(
            result.violation,
            Some(PolygonViolation::Outside {
                footprint: FootprintWitness {
                    segment_index: 2,
                    point: [5.0, 6.0]
                }
            })
        ),
        "{result:?}"
    );
    assert_eq!(result.boundary_distance, 0.0);
}
#[test]
fn validation_and_whole_query_budgets_are_explicit() {
    let support = support();
    let policy = PolygonClearancePolicy::default();
    let points = rectangle(1.0, 1.0, 3.0, 3.0);
    let result = support.polygon_clearance(&points, &policy).unwrap();
    assert_eq!(
        support
            .polygon_clearance(
                &points,
                &PolygonClearancePolicy {
                    max_pair_checks: result.pair_checks,
                    ..policy
                }
            )
            .unwrap(),
        result
    );
    for limited in [
        PolygonClearancePolicy {
            max_pair_checks: 0,
            ..policy
        },
        PolygonClearancePolicy {
            max_pair_checks: result.pair_checks - 1,
            ..policy
        },
        PolygonClearancePolicy {
            max_vertices: 3,
            ..policy
        },
    ] {
        assert_eq!(
            support.polygon_clearance(&points, &limited),
            Err(PolygonClearanceError::BudgetExceeded)
        );
    }
    assert!(matches!(
        support.polygon_clearance(&[[1.0, 1.0], [3.0, 3.0], [1.0, 3.0], [3.0, 1.0]], &policy),
        Err(PolygonClearanceError::FootprintContact {
            first: 0,
            second: 2
        })
    ));
    assert_eq!(
        support.polygon_clearance(&[[1.0, 1.0], [3.0, 1.0], [2.0, 1.0]], &policy),
        Err(PolygonClearanceError::InvalidFootprint)
    );
    assert_eq!(
        support.polygon_clearance(&[[f64::NAN, 1.0], [3.0, 1.0], [2.0, 3.0]], &policy),
        Err(PolygonClearanceError::InvalidInput)
    );
    assert_eq!(
        result.classify(-1.0, 0.0),
        Err(PolygonClearanceError::InvalidInput)
    );
}
#[test]
fn oblique_shared_intervals_and_collinear_stations_keep_contact_semantics() {
    let transform = |p: [f64; 2]| {
        [
            3.0 + 0.6 * p[0] - 0.8 * p[1],
            -2.0 + 0.8 * p[0] + 0.6 * p[1],
        ]
    };
    // Dyadic coordinates make the authored shared edges exactly collinear.
    let points = [[0.0, 0.0], [2.0, 1.0], [4.0, 2.0], [2.0, 6.0], [-2.0, 4.0]];
    let support = patch(&points, &[]);
    for p in [points.to_vec(), points.into_iter().rev().collect()] {
        assert!(
            support
                .polygon_clearance(&p, &PolygonClearancePolicy::default())
                .unwrap()
                .is_contained()
        );
    }
    let outer = rectangle(0.0, 0.0, 10.0, 10.0).map(transform);
    let footprint = rectangle(2.0, 2.0, 4.0, 4.0).map(transform);
    let result = patch(&outer, &[])
        .polygon_clearance(&footprint, &PolygonClearancePolicy::default())
        .unwrap();
    assert!(result.is_contained());
    assert!((result.boundary_distance - 2.0).abs() < 1e-12);
}

#[test]
fn rectangle_grid_matches_independent_material_overlap_oracle() {
    let support = support();
    let policy = PolygonClearancePolicy::default();
    for x in -1..=10 {
        for y in -1..=10 {
            for width in [1, 2, 4] {
                for height in [1, 2, 4] {
                    let (x1, y1) = (x + width, y + height);
                    let expected = x >= 0
                        && y >= 0
                        && x1 <= 10
                        && y1 <= 10
                        && !(x < 6 && x1 > 4 && y < 6 && y1 > 4);
                    let points =
                        rectangle(f64::from(x), f64::from(y), f64::from(x1), f64::from(y1));
                    let result = support.polygon_clearance(&points, &policy).unwrap();
                    assert_eq!(result.is_contained(), expected, "{points:?}: {result:?}");
                }
            }
        }
    }
}

#[test]
fn unrepresentable_interval_interiors_are_refused() {
    let points = [
        [1.0, 1.0],
        [1.0 + f64::EPSILON, 1.0],
        [1.0 + f64::EPSILON, 2.0],
        [1.0, 2.0],
    ];
    let policy = PolygonClearancePolicy {
        min_separation: 1e-18,
        ..Default::default()
    };
    assert_eq!(
        support().polygon_clearance(&points, &policy),
        Err(PolygonClearanceError::NumericLimit)
    );
}

#[test]
fn signed_zero_collinear_stations_preserve_hole_side_for_either_winding() {
    let outer = rectangle(-4.0, -2.0, 4.0, 4.0);
    for (hole, contained) in [
        ([[0.0, 0.0], [0.0, 2.0], [2.0, 2.0], [2.0, 0.0]], false),
        ([[-2.0, 0.0], [-2.0, 2.0], [0.0, 2.0], [0.0, 0.0]], true),
    ] {
        let support = patch(&outer, &[&hole]);
        for zero in [0.0, -0.0] {
            let mut footprint = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [zero, 1.0]];
            for _ in 0..2 {
                for _ in 0..footprint.len() {
                    let result = support
                        .polygon_clearance(&footprint, &PolygonClearancePolicy::default())
                        .unwrap();
                    assert_eq!(
                        result.is_contained(),
                        contained,
                        "{footprint:?}: {result:?}"
                    );
                    if !contained {
                        assert!(matches!(
                            result.violation,
                            Some(PolygonViolation::CoveredHole { .. })
                        ));
                    }
                    footprint.rotate_left(1);
                }
                footprint.reverse();
            }
        }
    }
}

#[test]
fn cancellation_in_crossing_witness_is_refused() {
    let support = patch(&[[0.40625, 0.40625], [0.4375, 0.4375], [0.40625, 0.5]], &[]);
    let e = f64::EPSILON;
    let footprint = [[0.0, 0.75 * e], [1.0, 1.0 - e], [1.0, 2.0], [0.0, 1.0]];
    assert_eq!(
        support.polygon_clearance(&footprint, &PolygonClearancePolicy::default()),
        Err(PolygonClearanceError::NumericLimit)
    );
}
