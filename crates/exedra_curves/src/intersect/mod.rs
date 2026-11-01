// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Certified intersections of planar curve pieces.
//!
//! Two Bezier pieces over `[0, 1]`, polynomial or rational, or a piece and
//! a circle, meet where a system of two equations in two parameters
//! vanishes. [`crossings`] isolates every such root inside the open unit
//! square by interval subdivision with outward-rounded arithmetic, and
//! proves each one unique and contained with the Krawczyk test (as Tucker
//! and Wilczak state it in "A rigorous lower bound for the stability
//! regions of the quadratic map", Physica D 238, 2009, section 2.3,
//! equation (10) and theorem 6;
//! <https://ww2.ii.uj.edu.pl/~wilczak/papers/logmap/logmap.pdf>). A crossing is
//! reported only with a positional bound no larger than the caller's
//! tolerance; anything the arithmetic cannot settle (a tangency, a root on
//! a piece's end, an overlapping run, a box that will not contract) is a
//! typed refusal rather than a guess. The work is charged one step at a
//! time to the caller, which may stop it.
//!
//! These are intersections of the pieces as given: a caller whose pieces
//! approximate other curves answers for that approximation.

mod enclosure;
pub(crate) mod interval;
pub use enclosure::{BezierEnclosure, HullSeparation};

use alloc::vec::Vec;

use crate::vector::WeightedPoint;
use interval::I;

/// Pieces of more controls than this are refused rather than evaluated.
pub const MAX_ORDER: usize = 16;

/// A Bezier piece over `[0, 1]` in the plane.
#[derive(Clone, Copy, Debug)]
pub enum Piece<'a> {
    /// A polynomial piece with these controls, 2 to [`MAX_ORDER`] of them.
    Polynomial(&'a [[f64; 2]]),
    /// A rational piece with these weighted controls, 2 to [`MAX_ORDER`]
    /// of them, its weights positive and finite.
    Rational(&'a [WeightedPoint<2>]),
}

/// What [`crossings`] solves for.
#[derive(Clone, Copy, Debug)]
pub enum System<'a> {
    /// Where two pieces meet: parameters on the first and the second.
    Pieces(Piece<'a>, Piece<'a>),
    /// Where a piece meets the circle about `center` of `radius`: the
    /// first parameter is the piece's, the second is held at `0.5`.
    Circle(Piece<'a>, [f64; 2], f64),
}

/// One isolated crossing.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Crossing {
    /// The parameters on the two pieces, the middles of `enclosure`.
    pub parameters: [f64; 2],
    /// The middle of the box holding both pieces' positions over
    /// `enclosure`, within `bound` of the true crossing.
    pub point: [f64; 2],
    /// A bound on how far the true crossing lies from `point`: the L1
    /// diameter of the box holding both pieces' positions over
    /// `enclosure`.
    pub bound: f64,
    /// Parameter intervals proven to hold exactly one crossing.
    pub enclosure: [[f64; 2]; 2],
    /// The larger box in which no other crossing lies.
    domain: [[f64; 2]; 2],
}

/// Why [`crossings`] stopped without an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntersectError {
    /// The arithmetic could not settle a candidate: a tangency, a root on
    /// a piece's end, an overlapping run, or a non-finite value; or a piece
    /// had too few or too many controls, or a weight not positive and
    /// finite.
    Unresolved,
    /// The caller's step budget ran out.
    Budget,
}

impl core::fmt::Display for IntersectError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unresolved => write!(f, "an intersection could not be isolated"),
            Self::Budget => write!(f, "the intersection step budget ran out"),
        }
    }
}

impl core::error::Error for IntersectError {}

impl Piece<'_> {
    fn order(self) -> usize {
        match self {
            Self::Polynomial(c) => c.len(),
            Self::Rational(c) => c.len(),
        }
    }

    /// Whether the piece has an order the search evaluates and, if
    /// rational, positive finite weights.
    fn evaluable(self) -> bool {
        (2..=MAX_ORDER).contains(&self.order())
            && match self {
                Self::Polynomial(_) => true,
                Self::Rational(c) => c.iter().all(|h| h.weight > 0.0 && h.weight.is_finite()),
            }
    }

    /// The position and derivative over `t`, or `None` where a rational
    /// weight's interval reaches zero.
    fn eval(self, t: I) -> Option<([I; 2], [I; 2])> {
        match self {
            Self::Polynomial(c) => {
                let controls: [[I; 2]; MAX_ORDER] =
                    core::array::from_fn(|i| c.get(i).map_or([I::ZERO; 2], |p| p.map(I::p)));
                let n = c.len();
                let point = bezier(&controls[..n], t);
                let mut d = [[I::ZERO; 2]; MAX_ORDER];
                for i in 0..n - 1 {
                    for k in 0..2 {
                        d[i][k] = I::p((n - 1) as f64) * (controls[i + 1][k] - controls[i][k]);
                    }
                }
                Some((point, bezier(&d[..n - 1], t)))
            }
            Self::Rational(c) => {
                let n = c.len();
                let controls: [[I; 3]; MAX_ORDER] = core::array::from_fn(|i| {
                    c.get(i).map_or([I::ZERO; 3], |h| {
                        [I::p(h.weighted[0]), I::p(h.weighted[1]), I::p(h.weight)]
                    })
                });
                let a = bezier(&controls[..n], t);
                let mut d = [[I::ZERO; 3]; MAX_ORDER];
                for i in 0..n - 1 {
                    for k in 0..3 {
                        d[i][k] = I::p((n - 1) as f64) * (controls[i + 1][k] - controls[i][k]);
                    }
                }
                let da = bezier(&d[..n - 1], t);
                // C = A / w, C' = (A' w - A w') / w^2. On `[0, 1]` the
                // weight is a convex combination of the controls', which
                // interval arithmetic, losing that `1 - t` and `t` are
                // linked, widens toward zero: the controls' range bounds it.
                let (lightest, heaviest) =
                    c.iter().fold((f64::INFINITY, 0.0_f64), |(lo, hi), h| {
                        (lo.min(h.weight), hi.max(h.weight))
                    });
                let w = a[2].intersect(I {
                    lo: lightest,
                    hi: heaviest,
                })?;
                if !w.excludes_zero() {
                    return None;
                }
                let inverse = w.recip();
                let point = [a[0] * inverse, a[1] * inverse];
                let square = inverse * inverse;
                let first = core::array::from_fn(|k| (da[k] * w - a[k] * da[2]) * square);
                Some((point, first))
            }
        }
    }

    fn point(self, t: I) -> Option<[I; 2]> {
        self.eval(t).map(|(p, _)| p)
    }

    /// The piece's point at `t` in `[0, 1]`, as [`crossings`] evaluates
    /// it: the middle of its outward-rounded interval there. `None` for `t`
    /// outside `[0, 1]`, a piece [`crossings`] refuses, or arithmetic that
    /// over- or underflows on the way.
    #[must_use]
    pub fn point_at(self, t: f64) -> Option<[f64; 2]> {
        if !(0.0..=1.0).contains(&t) || !self.evaluable() {
            return None;
        }
        let point = self.point(I::p(t))?;
        point.iter().all(|v| v.valid()).then(|| point.map(I::mid))
    }
}

/// De Casteljau's scheme on interval controls.
fn bezier<const K: usize>(control: &[[I; K]], t: I) -> [I; K] {
    let mut p = [[I::ZERO; K]; MAX_ORDER];
    p[..control.len()].copy_from_slice(control);
    for n in (1..control.len()).rev() {
        for i in 0..n {
            p[i] = core::array::from_fn(|k| (I::ONE - t) * p[i][k] + t * p[i + 1][k]);
        }
    }
    p[0]
}

impl System<'_> {
    fn evaluable(self) -> bool {
        match self {
            Self::Pieces(a, b) => a.evaluable() && b.evaluable(),
            Self::Circle(a, _, _) => a.evaluable(),
        }
    }

    /// The equations and their Jacobian over `x`.
    fn eval(self, x: [I; 2]) -> Option<([I; 2], [[I; 2]; 2])> {
        match self {
            Self::Pieces(a, b) => {
                let (p, dp) = a.eval(x[0])?;
                let (q, dq) = b.eval(x[1])?;
                Some((
                    [p[0] - q[0], p[1] - q[1]],
                    [[dp[0], -dq[0]], [dp[1], -dq[1]]],
                ))
            }
            Self::Circle(c, center, radius) => {
                let (p, d) = c.eval(x[0])?;
                let p = [p[0] - I::p(center[0]), p[1] - I::p(center[1])];
                Some((
                    [
                        p[0].square() + p[1].square() - I::p(radius).square(),
                        x[1] - I::p(0.5),
                    ],
                    [
                        [I::p(2.0) * (p[0] * d[0] + p[1] * d[1]), I::ZERO],
                        [I::ZERO, I::ONE],
                    ],
                ))
            }
        }
    }

    fn positions(self, x: [I; 2]) -> Option<[I; 2]> {
        match self {
            Self::Pieces(a, b) => {
                let (p, q) = (a.point(x[0])?, b.point(x[1])?);
                Some([p[0].union(q[0]), p[1].union(q[1])])
            }
            Self::Circle(c, _, _) => c.point(x[0]),
        }
    }
}

struct Node {
    x: [I; 2],
    depth: u32,
}

fn includes(a: [I; 2], b: [I; 2]) -> bool {
    (0..2).all(|i| a[i].contains(b[i]))
}

/// Every crossing of `system` inside the open unit square, each isolated
/// to a positional `bound` no larger than `tolerance`, or why there is no
/// answer. `step` is called once per box examined and stops the search by
/// returning `false`.
///
/// # Errors
///
/// [`IntersectError::Unresolved`] where a candidate cannot be settled,
/// [`IntersectError::Budget`] where `step` refused.
pub fn crossings(
    system: System<'_>,
    tolerance: f64,
    mut step: impl FnMut() -> bool,
) -> Result<Vec<Crossing>, IntersectError> {
    let unresolved = IntersectError::Unresolved;
    if !system.evaluable() {
        return Err(unresolved);
    }
    let mut stack = alloc::vec![Node {
        x: [I::UNIT; 2],
        depth: 0
    }];
    let mut roots: Vec<Crossing> = Vec::new();
    'nodes: while let Some(Node { mut x, depth }) = stack.pop() {
        let mut domain = None;
        let mut expanded = false;
        for _ in 0..64 {
            if !step() {
                return Err(IntersectError::Budget);
            }
            if roots.iter().any(|r| includes(boxed(r.domain), x)) {
                continue 'nodes;
            }
            let (f, j) = system.eval(x).ok_or(unresolved)?;
            if f.iter().chain(j.iter().flatten()).any(|v| !v.valid()) {
                return Err(unresolved);
            }
            if f.iter().any(|v| v.excludes_zero()) {
                continue 'nodes;
            }
            let mid = x.map(I::mid);
            let (f0, j0) = system.eval(mid.map(I::p)).ok_or(unresolved)?;
            let a = j0.map(|r| r.map(I::mid));
            let determinant = a[0][0] * a[1][1] - a[0][1] * a[1][0];
            if determinant.is_finite() && determinant != 0.0 {
                let c = [
                    [a[1][1] / determinant, -a[0][1] / determinant],
                    [-a[1][0] / determinant, a[0][0] / determinant],
                ];
                let det_c = I::p(c[0][0]) * I::p(c[1][1]) - I::p(c[0][1]) * I::p(c[1][0]);
                if c.iter().flatten().all(|v| v.is_finite())
                    && det_c.valid()
                    && det_c.excludes_zero()
                {
                    let mut k = [I::ZERO; 2];
                    for i in 0..2 {
                        k[i] = I::p(mid[i]) - I::p(c[i][0]) * f0[0] - I::p(c[i][1]) * f0[1];
                        for n in 0..2 {
                            let identity = if i == n { I::ONE } else { I::ZERO };
                            let factor =
                                identity - I::p(c[i][0]) * j[0][n] - I::p(c[i][1]) * j[1][n];
                            k[i] = k[i] + factor * (x[n] - I::p(mid[n]));
                        }
                    }
                    if k.iter().any(|v| !v.valid()) {
                        return Err(unresolved);
                    }
                    let (Some(first), Some(second)) = (x[0].intersect(k[0]), x[1].intersect(k[1]))
                    else {
                        continue 'nodes;
                    };
                    if domain.is_none() && (0..2).all(|i| x[i].interior(k[i])) {
                        domain = Some(x);
                    }
                    let next = [first, second];
                    if let Some(domain) = domain {
                        let position = system.positions(next).ok_or(unresolved)?;
                        if position.iter().any(|v| !v.valid()) {
                            return Err(unresolved);
                        }
                        let point = [position[0].mid(), position[1].mid()];
                        // Full box L1 diameter covers midpoint and evaluation rounding.
                        let bound = (position[0].width().next_up() + position[1].width().next_up())
                            .next_up();
                        if bound <= tolerance {
                            let root = Crossing {
                                parameters: next.map(I::mid),
                                point,
                                bound,
                                enclosure: next.map(|v| [v.lo, v.hi]),
                                domain: domain.map(|v| [v.lo, v.hi]),
                            };
                            if !roots.iter().any(|old| {
                                includes(boxed(old.domain), next)
                                    || includes(domain, boxed(old.enclosure))
                            }) {
                                roots.push(root);
                            }
                            continue 'nodes;
                        }
                    }
                    if next[0].width() + next[1].width() < 0.8 * (x[0].width() + x[1].width()) {
                        x = next;
                        continue;
                    }
                    if domain.is_some() {
                        return Err(unresolved);
                    }
                }
            }
            // Near a rounded root, a contracted box may have the root on its
            // boundary. Prove inclusion in a slightly larger finite-domain box;
            // this also covers every unresolved point of the original box.
            if !expanded && x.iter().all(|v| v.width() < 1e-6) {
                x = x.map(|v| {
                    let pad = (v.width() * 2.0).max(128.0 * f64::EPSILON);
                    I {
                        lo: (v.lo - pad).max(0.0),
                        hi: (v.hi + pad).min(1.0),
                    }
                });
                expanded = true;
                continue;
            }
            if depth >= 64 {
                return Err(unresolved);
            }
            let axis = usize::from(x[1].width() > x[0].width());
            let middle = x[axis].mid();
            let overlap = x[axis].width() * 0.0625;
            let mut left = x;
            let mut right = x;
            left[axis].hi = middle + overlap;
            right[axis].lo = middle - overlap;
            if left[axis].hi >= x[axis].hi || right[axis].lo <= x[axis].lo {
                return Err(unresolved);
            }
            stack.push(Node {
                x: right,
                depth: depth + 1,
            });
            stack.push(Node {
                x: left,
                depth: depth + 1,
            });
            continue 'nodes;
        }
        return Err(unresolved);
    }
    Ok(roots)
}

fn boxed(bounds: [[f64; 2]; 2]) -> [I; 2] {
    bounds.map(|[lo, hi]| I { lo, hi })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NurbsCurve;

    fn all(system: System<'_>, tolerance: f64) -> Result<Vec<Crossing>, IntersectError> {
        crossings(system, tolerance, || true)
    }

    #[test]
    fn polynomial_pieces_cross_once_at_every_scale() {
        for scale in [1e-6, 1.0, 1e6] {
            let line = [[-scale, 0.0], [scale, 0.0]];
            let cubic = [
                [0.0, -scale],
                [0.2 * scale, -scale / 3.0],
                [-0.2 * scale, scale / 3.0],
                [0.0, scale],
            ];
            let tolerance = 1e-7 * scale;
            let found = all(
                System::Pieces(Piece::Polynomial(&line), Piece::Polynomial(&cubic)),
                tolerance,
            )
            .unwrap();
            assert_eq!(found.len(), 1);
            let crossing = found[0];
            assert!(crossing.point[0].hypot(crossing.point[1]) <= crossing.bound);
            assert!(crossing.bound <= tolerance);
            // The true crossing, at the middle of both, lies in the
            // proven enclosure.
            for [lo, hi] in crossing.enclosure {
                assert!(lo <= 0.5 && 0.5 <= hi, "{crossing:?}");
            }
        }
    }

    #[test]
    fn rational_arcs_cross_where_the_circles_do() {
        // The quarter circle of radius 1 from (1, 0) to (0, 1), exactly.
        let quarter = NurbsCurve::<2>::arc([0.0; 2], [1.0, 0.0], [0.0, 1.0], [0.0, FRAC_PI_2])
            .unwrap()
            .view()
            .prepare();
        let arc = quarter.span_controls(0).unwrap();
        // The diagonal meets it at (1, 1) / sqrt(2).
        let diagonal = [[0.0, 0.0], [1.0, 1.0]];
        let found = all(
            System::Pieces(Piece::Rational(arc), Piece::Polynomial(&diagonal)),
            1e-12,
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        let half = core::f64::consts::FRAC_1_SQRT_2;
        let [x, y] = found[0].point;
        assert!(
            (x - half).hypot(y - half) <= found[0].bound + 1e-15,
            "{found:?}"
        );
        // Against the unit circle about (1, 1), it crosses at (1 - s, 1 - s)
        // rotated: both circles' common points.
        let shifted = NurbsCurve::<2>::arc([1.0, 1.0], [-1.0, 0.0], [0.0, -1.0], [0.0, FRAC_PI_2])
            .unwrap()
            .view()
            .prepare();
        let found = all(
            System::Pieces(
                Piece::Rational(arc),
                Piece::Rational(shifted.span_controls(0).unwrap()),
            ),
            1e-12,
        );
        // The two quarter arcs meet only at their shared ends, (1, 0) and
        // (0, 1): roots on a piece's end are refused, not guessed.
        assert_eq!(found, Err(IntersectError::Unresolved));
        // A circle about (1, 1) of radius 1/2 crosses it twice, symmetric
        // about the diagonal; one of radius sqrt(2) - 1 only touches it.
        let found = all(System::Circle(Piece::Rational(arc), [1.0, 1.0], 0.5), 1e-12).unwrap();
        assert_eq!(found.len(), 2);
        for crossing in &found {
            let [x, y] = crossing.point;
            assert!((x.hypot(y) - 1.0).abs() <= crossing.bound + 1e-15);
            assert!(((x - 1.0).hypot(y - 1.0) - 0.5).abs() <= crossing.bound + 1e-15);
        }
        let touching = System::Circle(Piece::Rational(arc), [1.0, 1.0], 2.0_f64.sqrt() - 1.0);
        assert_eq!(all(touching, 1e-12), Err(IntersectError::Unresolved));
    }

    #[test]
    fn tangencies_coincidence_and_overlong_pieces_are_refused() {
        let horizontal = [[-2.0, 1.0], [2.0, 1.0]];
        let vertical = [[2.0, 0.0], [2.0, 2.0]];
        for system in [
            System::Circle(Piece::Polynomial(&horizontal), [0.0; 2], 1.0),
            System::Pieces(
                Piece::Polynomial(&horizontal),
                Piece::Polynomial(&horizontal),
            ),
            System::Pieces(Piece::Polynomial(&horizontal), Piece::Polynomial(&vertical)),
        ] {
            assert_eq!(
                all(system, 1e-6),
                Err(IntersectError::Unresolved),
                "{system:?}"
            );
        }
        let long = [[0.0; 2]; MAX_ORDER + 1];
        let short = [[0.0; 2]; 1];
        for piece in [&long[..], &short[..]] {
            let system = System::Pieces(Piece::Polynomial(piece), Piece::Polynomial(&horizontal));
            assert_eq!(all(system, 1e-6), Err(IntersectError::Unresolved));
            assert_eq!(Piece::Polynomial(piece).point_at(0.5), None);
        }
    }

    #[test]
    fn the_caller_can_stop_the_search() {
        let line = [[-1.0, 0.0], [1.0, 0.0]];
        let other = [[0.0, -1.0], [0.0, 1.0]];
        let system = System::Pieces(Piece::Polynomial(&line), Piece::Polynomial(&other));
        let mut steps = 0;
        let found = crossings(system, 1e-9, || {
            steps += 1;
            false
        });
        assert_eq!(steps, 1);
        assert_eq!(found, Err(IntersectError::Budget));
        assert_eq!(all(system, 1e-9).map(|found| found.len()), Ok(1));
    }

    #[test]
    fn pieces_of_sixteen_controls_cross_where_their_lines_do() {
        // Sixteen controls evenly along the diagonal trace it at uniform
        // speed; weights bunch them, which moves no point.
        let along: [[f64; 2]; MAX_ORDER] = core::array::from_fn(|i| {
            let x = 2.0 * i as f64 / 15.0 - 1.0;
            [x, x]
        });
        let weighted: [WeightedPoint<2>; MAX_ORDER] =
            core::array::from_fn(|i| WeightedPoint::new(along[i], 1.0 + (i % 3) as f64));
        let level = [[-1.0, 0.25], [1.0, 0.25]];
        for piece in [Piece::Polynomial(&along), Piece::Rational(&weighted)] {
            let found = all(System::Pieces(piece, Piece::Polynomial(&level)), 1e-12).unwrap();
            assert_eq!(found.len(), 1);
            let [x, y] = found[0].point;
            assert!((x - 0.25).abs() + (y - 0.25).abs() <= found[0].bound + 1e-15);
            // On the level line, the crossing is at 0.625 of the way.
            let [lo, hi] = found[0].enclosure[1];
            assert!(lo <= 0.625 && 0.625 <= hi, "{found:?}");
        }
        let [lo, hi] = all(
            System::Pieces(Piece::Polynomial(&along), Piece::Polynomial(&level)),
            1e-12,
        )
        .unwrap()[0]
            .enclosure[0];
        assert!(lo <= 0.625 && 0.625 <= hi);
    }

    #[test]
    fn evaluation_refuses_what_the_search_cannot_take() {
        let tiny = [
            WeightedPoint::new([0.0, 0.0], 1e-310),
            WeightedPoint::new([1.0, 1.0], 1e-310),
        ];
        assert_eq!(Piece::Rational(&tiny).point_at(0.5), None);
        let negative = [
            WeightedPoint::new([0.0, 0.0], -1.0),
            WeightedPoint::new([1.0, 1.0], -1.0),
        ];
        assert_eq!(Piece::Rational(&negative).point_at(0.5), None);
        let line = [[0.0, 1.0], [1.0, 0.0]];
        assert_eq!(
            all(
                System::Pieces(Piece::Rational(&negative), Piece::Polynomial(&line)),
                1e-9
            ),
            Err(IntersectError::Unresolved)
        );
        let fine = [
            WeightedPoint::new([0.0, 0.0], 1.0),
            WeightedPoint::new([1.0, 1.0], 1.0),
        ];
        let [x, y] = Piece::Rational(&fine).point_at(0.5).unwrap();
        assert!((x - 0.5).abs() < 1e-15 && (y - 0.5).abs() < 1e-15);
        for t in [-0.5, 1.5, f64::NAN] {
            assert_eq!(Piece::Rational(&fine).point_at(t), None);
            assert_eq!(Piece::Polynomial(&line).point_at(t), None);
        }
    }

    #[test]
    fn a_budget_spent_partway_stops_the_search() {
        let line = [[-1.0, 0.0], [1.0, 0.0]];
        let cubic = [
            [0.0, -1.0],
            [0.2, -1.0 / 3.0],
            [-0.2, 1.0 / 3.0],
            [0.0, 1.0],
        ];
        let system = System::Pieces(Piece::Polynomial(&line), Piece::Polynomial(&cubic));
        let mut needed = 0_u32;
        assert!(
            crossings(system, 1e-12, || {
                needed += 1;
                true
            })
            .is_ok()
        );
        assert!(needed > 2, "{needed}");
        let mut left = needed - 1;
        let found = crossings(system, 1e-12, || {
            left = left.saturating_sub(1);
            left > 0
        });
        assert_eq!(found, Err(IntersectError::Budget));
    }

    use core::f64::consts::FRAC_PI_2;
}
