// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Point queries: closed forms for lines and arcs, certified searches for
//! Bezier segments.
use crate::profile::Loop;
use crate::profile::arc::{Piece, turns, winding_angle};

/// Winding and nearest-boundary distance from the resolved line, arc and
/// Bezier geometry.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LoopPointQuery {
    /// Minimum distance to the boundary, in model units: closed form for
    /// lines and arcs, a curve point found by a search for Bezier segments,
    /// within [`Self::numeric_allowance`] of the true distance. This is not
    /// a sampled chord distance.
    pub boundary_distance: f64,
    /// Conservative arithmetic allowance based on the point, authored endpoints,
    /// resolved circle centers and radii, plus, for Bezier segments, how far
    /// below `boundary_distance` their search left the curve possibly
    /// coming, including the rounding its halving and reweighting may have
    /// moved the curve by, and how far the chains their winding is counted
    /// along may lie from them. This is floating-point evidence, not an
    /// interval-arithmetic certificate or a bound on external conversion
    /// error.
    pub numeric_allowance: f64,
    /// Signed whole turns around the point. Counterclockwise simple interiors
    /// have `Some(1)`, clockwise interiors `Some(-1)` and exterior points `Some(0)`.
    /// `None` means boundary distance did not exceed the arithmetic allowance;
    /// no side is assigned to an unresolved boundary point.
    /// A loop need not be a valid profile, so winding can have other values.
    pub winding: Option<i64>,
}

/// A finite point query could not be evaluated reliably in f64.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PointQueryError {
    /// Either input coordinate is nonfinite.
    NonFinitePoint,
    /// A resolved intermediate, distance or angle cannot be represented reliably.
    NumericLimit,
}
impl core::fmt::Display for PointQueryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NonFinitePoint => "profile point query requires finite coordinates",
            Self::NumericLimit => "profile point query exceeds finite arithmetic limits",
        })
    }
}
impl core::error::Error for PointQueryError {}

impl Loop {
    /// Queries the resolved boundary: lines and arcs in closed form, in one
    /// allocation-free traversal; Bezier segments by subdivision, which
    /// allocates, bounding their distance from both sides and counting
    /// their crossings of the ray leftward from the point.
    ///
    /// The boundary distance uses bounded circular arcs, including major arcs,
    /// and the winding uses their complete angular contributions. No polyline is
    /// created. On or numerically near the boundary, distance remains available
    /// while winding is `None`. Querying does not certify loop simplicity.
    ///
    /// A consumer converting another geometry representation must independently
    /// bound its conversion error and require sufficient distance beyond both
    /// that gap and [`LoopPointQuery::numeric_allowance`] before trusting the side.
    ///
    /// # Errors
    /// Refuses nonfinite input or unrepresentable intermediate arithmetic.
    pub fn query_point(&self, point: [f64; 2]) -> Result<LoopPointQuery, PointQueryError> {
        if !point.into_iter().all(f64::is_finite) {
            return Err(PointQueryError::NonFinitePoint);
        }
        let mut magnitude = point
            .into_iter()
            .map(f64::abs)
            .fold(f64::MIN_POSITIVE, f64::max);
        for piece in self.pieces() {
            magnitude = piece
                .from()
                .into_iter()
                .chain(piece.to())
                .map(f64::abs)
                .fold(magnitude, f64::max);
            if let Piece::Bezier(bezier) = &piece {
                magnitude = bezier
                    .bezier
                    .interior()
                    .iter()
                    .flatten()
                    .map(|v| v.abs())
                    .fold(magnitude, f64::max);
            }
            if let Piece::Arc(arc) = piece {
                magnitude = arc
                    .center
                    .into_iter()
                    .chain([arc.radius])
                    .map(f64::abs)
                    .fold(magnitude, f64::max);
            }
        }
        // Existing predicates contain products of coordinate differences.
        // Refuse before infinities could masquerade as an atan2 quadrant.
        if !magnitude.is_finite()
            || magnitude > libm::sqrt(f64::MAX) / 8.0
            || magnitude < libm::sqrt(f64::MIN_POSITIVE) * 8.0
        {
            return Err(PointQueryError::NumericLimit);
        }
        let rounding = 512.0 * f64::EPSILON * magnitude;
        let mut boundary_distance = f64::INFINITY;
        // How far below `boundary_distance` a Bezier search left the curve
        // possibly coming.
        let mut unsettled = 0.0_f64;
        let mut angle = 0.0;
        let mut compensation = 0.0;
        let mut undecided = false;
        // How far a Bezier piece's winding chain may lie from its curve.
        let mut chained = 0.0_f64;
        for piece in self.pieces() {
            let (distance, contribution) = match &piece {
                Piece::Bezier(bezier) => {
                    let (distance, lower) = bezier.distance_bounds(point);
                    unsettled = unsettled.max(distance - lower);
                    let (contribution, drift) = bezier.winding_angle(point, rounding);
                    chained = chained.max(drift);
                    (distance, contribution)
                }
                _ => (
                    piece.distance_to(point),
                    winding_angle(&piece, point, rounding),
                ),
            };
            if !distance.is_finite() {
                return Err(PointQueryError::NumericLimit);
            }
            boundary_distance = boundary_distance.min(distance);
            // A Bezier piece crossing the ray too near the point to tell
            // which side leaves the winding undecided.
            if matches!(piece, Piece::Bezier(_)) && contribution.is_nan() {
                undecided = true;
                continue;
            }
            if !contribution.is_finite() {
                return Err(PointQueryError::NumericLimit);
            }
            let adjusted = contribution - compensation;
            let next = angle + adjusted;
            compensation = (next - angle) - adjusted;
            angle = next;
        }
        // A winding counted along chains within `chained` of the curve
        // holds for points farther than that from it.
        let numeric_allowance = rounding + unsettled + chained;
        if !boundary_distance.is_finite() || !numeric_allowance.is_finite() || !angle.is_finite() {
            return Err(PointQueryError::NumericLimit);
        }
        Ok(LoopPointQuery {
            boundary_distance,
            numeric_allowance,
            winding: (!undecided && boundary_distance > numeric_allowance).then(|| turns(angle)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Segment;
    use alloc::vec;

    fn disk() -> Loop {
        Loop::new(vec![
            Segment::arc([2.0, 0.0], 1.0),
            Segment::arc([-2.0, 0.0], 1.0),
        ])
        .unwrap()
    }
    #[test]
    fn circle_distance_and_winding_cover_both_sides_of_diameter_chords() {
        let disk = disk();
        for (point, distance, winding) in [
            ([0.0, 0.0], 2.0, 1),
            ([0.0, 1.0], 1.0, 1),
            ([0.0, -1.0], 1.0, 1),
            ([0.0, 3.0], 1.0, 0),
        ] {
            let query = disk.query_point(point).unwrap();
            assert_eq!(query.winding, Some(winding));
            assert!((query.boundary_distance - distance).abs() < 1e-14);
            assert!(query.numeric_allowance > 0.0);
        }
        assert_eq!(
            disk.reversed().query_point([0.0; 2]).unwrap().winding,
            Some(-1)
        );
    }
    #[test]
    fn boundary_and_near_boundary_never_receive_an_arbitrary_side() {
        for p in [
            [2.0, 0.0],
            [0.0, 2.0],
            [0.0, -2.0],
            [2.0 + f64::EPSILON, 0.0],
        ] {
            let query = disk().query_point(p).unwrap();
            assert_eq!(query.winding, None);
            assert!(query.boundary_distance <= query.numeric_allowance);
        }
    }
    #[test]
    fn mixed_major_arc_and_line_use_bounded_curves() {
        let loop_ = Loop::new(vec![
            Segment::arc([0.0, -1.0], 1.0 + libm::sqrt(2.0)),
            Segment::line([1.0, 0.0]),
        ])
        .unwrap();
        assert_eq!(loop_.query_point([0.0, 0.0]).unwrap().winding, Some(1));
        assert_eq!(loop_.query_point([0.8, -0.8]).unwrap().winding, Some(0));
        let query = loop_.query_point([0.9, -0.9]).unwrap();
        assert!((query.boundary_distance - libm::sqrt(0.32)).abs() < 1e-12);
    }
    #[test]
    fn nonfinite_inputs_and_product_overflow_are_typed_refusals() {
        assert_eq!(
            disk().query_point([f64::NAN, 0.0]),
            Err(PointQueryError::NonFinitePoint)
        );
        assert_eq!(
            disk().query_point([f64::INFINITY, 0.0]),
            Err(PointQueryError::NonFinitePoint)
        );
        assert_eq!(
            disk().query_point([f64::MAX, 0.0]),
            Err(PointQueryError::NumericLimit)
        );
        let tiny = Loop::new(vec![
            Segment::arc([1e-200, 0.0], 1.0),
            Segment::arc([-1e-200, 0.0], 1.0),
        ])
        .unwrap();
        assert_eq!(
            tiny.query_point([0.0; 2]),
            Err(PointQueryError::NumericLimit)
        );
    }
}
