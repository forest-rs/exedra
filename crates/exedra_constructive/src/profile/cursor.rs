// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Segment-by-segment loop drawing from an explicit start point.

use alloc::vec::Vec;

use kurbo::{Point, Vec2};

use super::{Loop2, ProfileError, Seg2, SegTag};

/// Draws a [`Loop2`] one segment at a time from an explicit start point.
///
/// Each step continues from the current point. [`Self::close`] returns to the
/// start and validates the loop exactly as [`Loop2::new`] does; the start point
/// becomes the end of the final segment, so segment zero is the first one drawn.
/// Steps never round, merge or heal points.
///
/// A step that cannot be constructed (a collinear three-point arc, for example)
/// records the first error with the index of the segment it would have added.
/// Later steps are ignored and [`Self::close`] returns that error, so a chain of
/// steps needs one `?`.
///
/// ```
/// use exedra_constructive::profile::{LoopCursor, SegTag};
///
/// // A 4 x 2 slot drawn counter-clockwise: straight sides, round ends.
/// let slot = LoopCursor::new((1.0, 0.0))
///     .line_to((3.0, 0.0))
///     .arc_about((3.0, 1.0), core::f64::consts::PI)
///     .line_to((1.0, 2.0))
///     // Land exactly on the start, so closing adds no segment.
///     .arc_through((0.0, 1.0), (1.0, 0.0))
///     .tagged(SegTag(7))
///     .close()?;
/// assert_eq!(slot.segs().len(), 4);
/// assert_eq!(slot.segs()[3].tag, Some(SegTag(7)));
/// assert!(slot.signed_area() > 0.0);
/// # Ok::<(), exedra_constructive::profile::ProfileError>(())
/// ```
#[derive(Clone, Debug)]
#[must_use = "a cursor does nothing until it is closed"]
pub struct LoopCursor {
    start: Point,
    current: Point,
    segs: Vec<Seg2>,
    error: Option<ProfileError>,
}

impl LoopCursor {
    /// Starts a loop at `start`.
    pub fn new(start: impl Into<Point>) -> Self {
        let start = start.into();
        Self {
            start,
            current: start,
            segs: Vec::new(),
            error: None,
        }
    }

    /// The point the next segment starts from.
    #[must_use]
    pub fn current(&self) -> Point {
        self.current
    }

    /// Appends a straight segment to `to`.
    pub fn line_to(self, to: impl Into<Point>) -> Self {
        self.segment(Seg2::line(to))
    }

    /// Appends a circular arc to `to` with an explicit bulge, as in
    /// [`SegKind::Arc`](super::SegKind::Arc).
    pub fn arc_to(self, to: impl Into<Point>, bulge: f64) -> Self {
        self.segment(Seg2::arc(to, bulge))
    }

    /// Appends the circular arc from the current point through `via` to `to`.
    ///
    /// The three points must be distinct and not collinear; otherwise the
    /// cursor records [`ProfileError::DegenerateArc`]. Both endpoints are kept
    /// exactly; `via` determines the bulge.
    pub fn arc_through(self, via: impl Into<Point>, to: impl Into<Point>) -> Self {
        let (via, to) = (via.into(), to.into());
        match through_bulge([self.current, via, to]) {
            Some(bulge) => self.arc_to(to, bulge),
            None => {
                let seg = self.segs.len();
                self.fail(ProfileError::DegenerateArc { seg })
            }
        }
    }

    /// Appends the circular arc about `center` sweeping `sweep` radians,
    /// counter-clockwise when positive.
    ///
    /// The radius is the distance from `center` to the current point, and the
    /// endpoint is computed from it. The sweep must be finite, nonzero and
    /// less than a full turn in magnitude; a full circle needs two arcs. A
    /// current point at `center`, or an invalid sweep, records
    /// [`ProfileError::DegenerateArc`]. Close onto the start with
    /// [`Self::arc_to`] or [`Self::arc_through`] when the endpoint must be
    /// exact: a computed endpoint can differ from the start by rounding.
    pub fn arc_about(self, center: impl Into<Point>, sweep: f64) -> Self {
        let center = center.into();
        let radius = self.current - center;
        if !sweep.is_finite()
            || sweep == 0.0
            || sweep.abs() >= core::f64::consts::TAU
            || radius == Vec2::ZERO
        {
            let seg = self.segs.len();
            return self.fail(ProfileError::DegenerateArc { seg });
        }
        let (sin, cos) = (libm::sin(sweep), libm::cos(sweep));
        let to = center
            + Vec2::new(
                radius.x * cos - radius.y * sin,
                radius.x * sin + radius.y * cos,
            );
        self.arc_to(to, libm::tan(sweep / 4.0))
    }

    /// Appends a prepared segment, such as a cubic or policy-defined curve.
    pub fn segment(mut self, seg: Seg2) -> Self {
        if self.error.is_none() {
            self.current = seg.to;
            self.segs.push(seg);
        }
        self
    }

    /// Tags the most recently added segment.
    ///
    /// Before the first segment this records [`ProfileError::NoSegmentToTag`].
    pub fn tagged(mut self, tag: SegTag) -> Self {
        if self.error.is_none() {
            match self.segs.last_mut() {
                Some(seg) => seg.tag = Some(tag),
                None => return self.fail(ProfileError::NoSegmentToTag),
            }
        }
        self
    }

    /// Closes the loop at the start point.
    ///
    /// A cursor whose current point is exactly the start closes without
    /// another segment; otherwise a straight segment returns to the start.
    ///
    /// # Errors
    ///
    /// Returns the first recorded step error, then any [`Loop2::new`]
    /// validation error.
    pub fn close(self) -> Result<Loop2, ProfileError> {
        let start = self.start;
        let cursor = if self.current == start {
            self
        } else {
            self.line_to(start)
        };
        match cursor.error {
            Some(error) => Err(error),
            None => Loop2::new(cursor.segs),
        }
    }

    fn fail(mut self, error: ProfileError) -> Self {
        self.error.get_or_insert(error);
        self
    }
}

/// The bulge of the arc from `start` through `via` to `end`, or `None` when the
/// points determine no circle.
///
/// The inscribed angle `phi` at `via` gives a sweep of `2 * pi - 2 * phi`, so
/// the bulge `tan(sweep / 4)` is `cot(phi / 2)`. With `u = start - via`,
/// `v = end - via`, `n = |u| |v|`, `d = u . v` and `c = u x v`, that is both
/// `(n + d) / |c|` and `|c| / (n - d)`, since `n^2 = d^2 + c^2`. Each form is
/// evaluated where it does not cancel: shallow arcs have `d` near `-n`. The
/// sign is the orientation of the three points, the opposite of `c`. The
/// vectors are first scaled by a power of two, which changes no bits of the
/// ratio, so neither large nor small coordinates overflow or underflow.
fn through_bulge([start, via, end]: [Point; 3]) -> Option<f64> {
    let finite = |w: Vec2| w.x.is_finite() && w.y.is_finite();
    let (mut u, mut v) = (start - via, end - via);
    if !finite(u) || !finite(v) {
        // Differences of finite points can exceed the range; halving every
        // point first keeps them finite.
        let half = |p: Point| Point::new(0.5 * p.x, 0.5 * p.y);
        (u, v) = (half(start) - half(via), half(end) - half(via));
    }
    let largest = u.x.abs().max(u.y.abs()).max(v.x.abs()).max(v.y.abs());
    if !largest.is_finite() || largest == 0.0 {
        return None;
    }
    let exponent = libm::ilogb(largest);
    let scale = |w: Vec2| Vec2::new(libm::scalbn(w.x, -exponent), libm::scalbn(w.y, -exponent));
    let (u, v) = (scale(u), scale(v));
    // One rounding where the squared product is normal; it underflows only
    // when one vector is shorter than the other by about 1e-154.
    let product = u.hypot2() * v.hypot2();
    let n = if product >= f64::MIN_POSITIVE {
        libm::sqrt(product)
    } else {
        libm::hypot(u.x, u.y) * libm::hypot(v.x, v.y)
    };
    let (d, c) = (u.dot(v), u.cross(v));
    if c == 0.0 {
        return None;
    }
    let bulge = if d <= 0.0 { -c / (n - d) } else { -(n + d) / c };
    (bulge.is_finite() && bulge != 0.0).then_some(bulge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{SegKind, bulge_arc_center_radius};

    #[test]
    fn three_point_arcs_take_the_side_of_their_via_point() {
        for (via, bulge) in [((0.0, 1.0), 1.0), ((0.0, -1.0), -1.0)] {
            let cursor = LoopCursor::new((1.0, 0.0)).arc_through(via, (-1.0, 0.0));
            assert_eq!(cursor.segs[0].kind, SegKind::Arc { bulge });
        }
        // A quarter arc: bulge tan(pi / 8) for a 90 degree sweep.
        let cursor = LoopCursor::new((1.0, 0.0)).arc_through(
            (
                core::f64::consts::FRAC_1_SQRT_2,
                core::f64::consts::FRAC_1_SQRT_2,
            ),
            (0.0, 1.0),
        );
        let SegKind::Arc { bulge } = cursor.segs[0].kind else {
            panic!("expected an arc");
        };
        assert!((bulge - libm::tan(core::f64::consts::FRAC_PI_8)).abs() < 1e-15);
        // A major arc passing through a point behind its chord.
        let cursor = LoopCursor::new((1.0, 0.0)).arc_through((-1.0, 0.0), (0.0, -1.0));
        let SegKind::Arc { bulge } = cursor.segs[0].kind else {
            panic!("expected an arc");
        };
        let (center, radius) =
            bulge_arc_center_radius((1.0, 0.0).into(), (0.0, -1.0).into(), bulge);
        assert!(center.distance(Point::ORIGIN) < 1e-15 && (radius - 1.0).abs() < 1e-15);
        assert!(bulge > 1.0, "three quarters of a turn counter-clockwise");
    }

    #[test]
    fn shallow_and_scaled_arcs_keep_their_bulge() {
        // Through the midpoint of a chord of length 2, the bulge is the
        // sagitta: an arc bowing left of its chord direction is negative.
        for sagitta in [0.5, 1e-4, 1e-8, 1e-9, 1e-12, 1e-150] {
            for side in [1.0, -1.0] {
                let height = side * sagitta;
                let bulge = through_bulge([
                    Point::new(0.0, 0.0),
                    Point::new(1.0, height),
                    Point::new(2.0, 0.0),
                ])
                .unwrap();
                assert!(
                    (bulge + height).abs() <= 4.0 * f64::EPSILON * height.abs(),
                    "sagitta {height}: bulge {bulge}"
                );
            }
        }
        // Semicircles and quarter arcs at any finite scale.
        let quarter = libm::tan(core::f64::consts::FRAC_PI_8);
        for scale in [1e-300, 1e-80, 1.0, 1e80, 1e300, f64::MAX] {
            let semicircle = through_bulge([
                Point::new(scale, 0.0),
                Point::new(0.0, scale),
                Point::new(-scale, 0.0),
            ]);
            assert_eq!(semicircle, Some(1.0), "semicircle at {scale}");
            let diagonal = core::f64::consts::FRAC_1_SQRT_2 * scale;
            let arc = through_bulge([
                Point::new(scale, 0.0),
                Point::new(diagonal, diagonal),
                Point::new(0.0, scale),
            ])
            .unwrap();
            assert!(
                (arc - quarter).abs() <= 4.0 * f64::EPSILON,
                "quarter at {scale}"
            );
        }
        // A via point beside the start: the squared lengths underflow, and the
        // upper semicircle from left to right is still a clockwise half turn.
        let beside = through_bulge([
            Point::new(0.0, 0.0),
            Point::new(0.0, 1e-170),
            Point::new(2.0, 0.0),
        ]);
        assert_eq!(beside, Some(-1.0));
        // The cursor accepts what the raw arc constructor accepts.
        let shallow = LoopCursor::new((0.0, 0.0))
            .arc_through((1.0, 1e-9), (2.0, 0.0))
            .close()
            .unwrap();
        let SegKind::Arc { bulge } = shallow.segs()[0].kind else {
            panic!("expected an arc");
        };
        assert!((bulge + 1e-9).abs() <= 4.0 * f64::EPSILON * 1e-9);
    }

    #[test]
    fn center_arcs_compute_their_endpoint_from_the_radius() {
        let cursor =
            LoopCursor::new((2.0, 1.0)).arc_about((1.0, 1.0), -core::f64::consts::FRAC_PI_2);
        let end = cursor.current();
        assert!(end.distance(Point::new(1.0, 0.0)) < 1e-15);
        let SegKind::Arc { bulge } = cursor.segs[0].kind else {
            panic!("expected an arc");
        };
        assert_eq!(bulge, libm::tan(-core::f64::consts::FRAC_PI_8));
    }

    #[test]
    fn construction_failures_name_the_segment_and_stop_drawing() {
        let collinear = LoopCursor::new((0.0, 0.0))
            .line_to((1.0, 0.0))
            .arc_through((2.0, 0.0), (3.0, 0.0))
            .line_to((3.0, 3.0));
        assert_eq!(collinear.segs.len(), 1, "later steps are ignored");
        assert_eq!(
            collinear.close(),
            Err(ProfileError::DegenerateArc { seg: 1 })
        );
        for sweep in [0.0, core::f64::consts::TAU, f64::NAN] {
            assert_eq!(
                LoopCursor::new((1.0, 0.0))
                    .arc_about((0.0, 0.0), sweep)
                    .close(),
                Err(ProfileError::DegenerateArc { seg: 0 })
            );
        }
        assert_eq!(
            LoopCursor::new((1.0, 0.0))
                .arc_about((1.0, 0.0), 1.0)
                .close(),
            Err(ProfileError::DegenerateArc { seg: 0 })
        );
        assert_eq!(
            LoopCursor::new((0.0, 0.0))
                .tagged(SegTag(1))
                .line_to((1.0, 0.0))
                .close(),
            Err(ProfileError::NoSegmentToTag)
        );
        // Validation stays with `Loop2::new`: a lone segment cannot close.
        assert_eq!(
            LoopCursor::new((0.0, 0.0)).close(),
            Err(ProfileError::TooFewSegments { count: 0 })
        );
    }

    #[test]
    fn closure_adds_a_line_only_away_from_the_start() {
        let open = LoopCursor::new((0.0, 0.0))
            .line_to((2.0, 0.0))
            .line_to((2.0, 1.0))
            .close()
            .unwrap();
        assert_eq!(open.segs().len(), 3);
        assert_eq!(open.segs()[2], Seg2::line((0.0, 0.0)));
        let landed = LoopCursor::new((0.0, 0.0))
            .line_to((2.0, 0.0))
            .arc_through((1.0, 1.0), (0.0, 0.0))
            .close()
            .unwrap();
        assert_eq!(landed.segs().len(), 2);
        assert_eq!(
            landed,
            Loop2::new(alloc::vec![
                Seg2::line((2.0, 0.0)),
                Seg2::arc((0.0, 0.0), 1.0)
            ])
            .unwrap()
        );
    }
}
