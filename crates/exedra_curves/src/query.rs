// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bounds and the closest point of a NURBS curve, with the certainty of
//! each stated.

use alloc::collections::BinaryHeap;
use alloc::vec::Vec;
use core::cmp::Ordering;

use exedra_math::cross;

use crate::vector::{WeightedPoint, dot, magnitude, sub};
use crate::{CurveEvaluation, NurbsCurve, PreparedNurbs};

impl CurveEvaluation<3> {
    /// The curvature, `|C' x C''| / |C'|^3`, or `None` where the curve
    /// stops (`C' = 0`) and has no direction to curve from.
    #[must_use]
    pub fn curvature(&self) -> Option<f64> {
        let speed = magnitude(self.first);
        // Divided one speed at a time, so no power of it overflows.
        (speed > 0.0).then(|| magnitude(cross(self.first, self.second)) / speed / speed / speed)
    }
}

impl<const N: usize> NurbsCurve<N> {
    /// A box holding the whole curve: its controls' box, which holds the
    /// curve because each point is a convex combination of controls with
    /// positive weights (Piegl and Tiller, *The NURBS Book*, 2nd ed.,
    /// property P4.4).
    #[must_use]
    pub fn bounds(&self) -> [[f64; N]; 2] {
        bounds_of(self.points.iter().copied())
    }
}

impl<const N: usize> PreparedNurbs<N> {
    /// A box holding span `span`'s piece of the curve: the box of its
    /// Bezier controls, tighter than the whole curve's controls give.
    /// `None` past the last span.
    #[must_use]
    pub fn span_bounds(&self, span: usize) -> Option<[[f64; N]; 2]> {
        let controls = self.span_controls(span)?;
        Some(bounds_of(controls.iter().map(WeightedPoint::point)))
    }

    /// A box and bounds on the first and second derivatives over `range`
    /// of the domain, from the Bezier pieces of the spans it meets. `None`
    /// unless `range` is an interval of the domain.
    ///
    /// The bounds cover each span's piece over the part of `range` it
    /// holds, up to the span's ends: where the curve jumps at a knot inside
    /// the range, or at one of its ends, they cover the piece on each side
    /// within the range, not the point [`Self::evaluate`] picks there.
    ///
    /// The bounds hold up to rounding, from the convex hulls of Bernstein
    /// polynomials. With the span's weighted controls `(A, w)`,
    /// `C' = (A' w - A w') / w^2`: the numerator is a polynomial whose
    /// Bezier coefficients over the range bound it, and `w` is at least its
    /// least coefficient there. Then `C'' = (A'' - 2 w' C' - w'' C) / w`,
    /// with `C` measured from the range's first point so the bound does not
    /// grow with distance from the origin. Without weights these are the
    /// hodograph's bounds. The polynomials are formed over the whole span
    /// and only then restricted to the range, which takes convex
    /// combinations, so a range however narrow loses nothing to
    /// cancellation; weights are scaled by a power of two first, which
    /// changes no point, so the smallest legal weights do not underflow.
    /// A bound whose arithmetic overflows is infinite. They are
    /// conservative; splitting a range tightens them.
    #[must_use]
    pub fn range_bounds(&self, range: [f64; 2]) -> Option<RangeBounds<N>> {
        let [start, end] = self.domain();
        let [a, b] = range;
        if !(start <= a && a < b && b <= end) {
            return None;
        }
        let order = self.order();
        let mut piece = alloc::vec![WeightedPoint::ZERO; order];
        let mut scratch = alloc::vec![WeightedPoint::ZERO; order];
        let mut out = RangeBounds {
            bounds: [[f64::INFINITY; N], [f64::NEG_INFINITY; N]],
            first: 0.0,
            second: 0.0,
        };
        for span in 0..self.spans() {
            let [s0, s1] = self.span_domain(span)?;
            let (from, to) = (a.max(s0), b.min(s1));
            if from >= to {
                continue;
            }
            let controls = self.span_controls(span)?;
            let width = s1 - s0;
            let (u0, u1) = ((from - s0) / width, (to - s0) / width);
            cut(controls, u0, u1, &mut piece, &mut scratch);
            let [lo, hi] = bounds_of(piece.iter().map(WeightedPoint::point));
            for k in 0..N {
                out.bounds[0][k] = out.bounds[0][k].min(lo[k]);
                out.bounds[1][k] = out.bounds[1][k].max(hi[k]);
            }
            // Derivatives in the span's own `[0, 1]`, which spans `width`
            // of the curve's parameter.
            let [first, second] = derivative_bounds(controls, &piece, [u0, u1]);
            out.first = upper(out.first, first / width);
            out.second = upper(out.second, second / width / width);
        }
        // Rounding in the arithmetic above, generously.
        let slack = 1.0 + 64.0 * f64::EPSILON * order as f64;
        out.first *= slack;
        out.second *= slack;
        Some(out)
    }

    /// The point of the curve closest to `target`, certified to within
    /// `tolerance`: the distance found is at most `tolerance` more than
    /// the true least distance, unless the report says otherwise.
    ///
    /// The spans are searched branch and bound: a piece of curve is no
    /// nearer than the convex hull of its Bezier controls, which holds it,
    /// and a lower bound on the hull's distance is cheap; pieces bounded
    /// farther than the best point found, less the tolerance, are dropped,
    /// and the nearest is split in half by de Casteljau's scheme until none
    /// is left. Each piece's middle is a
    /// candidate, polished by Newton's method on `(C - target) . C'`
    /// within the piece. Exact ties go to the earlier parameter, and the
    /// search visits pieces in a fixed order, so the result is
    /// deterministic.
    ///
    /// At most `max_pieces` pieces are split; a search stopped there
    /// reports the least distance it can still certify
    /// ([`Closest::lower`]), so a caller knows how far the answer may be
    /// off. A tolerance of zero is met only where boxes shrink onto the
    /// point exactly, so a positive one is the usual choice. The search's
    /// working storage is allocated per call.
    #[must_use]
    pub fn closest(&self, target: [f64; N], tolerance: f64, max_pieces: u32) -> Closest<N> {
        let tolerance = if tolerance.is_nan() {
            0.0
        } else {
            tolerance.max(0.0)
        };
        let order = self.order();
        let mut best = Closest {
            parameter: self.domain()[0],
            span: 0,
            point: self.evaluate(self.domain()[0]),
            distance: f64::INFINITY,
            lower: 0.0,
            pieces: 0,
            certain: false,
        };
        let consider = |span: usize, t: f64, best: &mut Closest<N>| {
            let e = self.evaluation_in(span, t).expect("a prepared span");
            offer(best, span, t, e.point, magnitude(sub(e.point, target)));
        };
        let mut heap = BinaryHeap::new();
        let mut pool: Vec<WeightedPoint<N>> = Vec::new();
        let mut sequence = 0_u64;
        for span in 0..self.spans() {
            let [a, b] = self.span_domain(span).expect("a prepared span");
            for t in [a, 0.5 * (a + b), b] {
                consider(span, t, &mut best);
            }
            let controls = self.span_controls(span).expect("a prepared span");
            let start = pool.len();
            pool.extend_from_slice(controls);
            heap.push(Piece {
                lower: piece_lower(&pool[start..], target),
                sequence,
                span,
                range: [a, b],
                controls: start,
            });
            sequence += 1;
        }
        let mut left = alloc::vec![WeightedPoint::ZERO; order];
        let mut right = alloc::vec![WeightedPoint::ZERO; order];
        // The nearest box of a piece too narrow to split, which the search
        // cannot look into further, and of the pieces dropped as no nearer
        // than the tolerance allows, which may still be nearer than the
        // point found.
        let mut unsplit = f64::INFINITY;
        let mut dropped = f64::INFINITY;
        let mut lower = loop {
            let Some(piece) = heap.pop() else {
                // Every piece dropped: none is nearer than the tolerance
                // allows.
                break (best.distance - tolerance).max(0.0);
            };
            if piece.lower >= best.distance - tolerance || best.pieces >= max_pieces {
                break piece.lower;
            }
            best.pieces += 1;
            let [a, b] = piece.range;
            let middle = a + 0.5 * (b - a);
            polish(self, piece.span, [a, b], middle, target, &mut best);
            if !(a < middle && middle < b) {
                unsplit = unsplit.min(piece.lower);
                continue;
            }
            split(
                &pool[piece.controls..piece.controls + order],
                &mut left,
                &mut right,
            );
            for (half, range) in [(&left, [a, middle]), (&right, [middle, b])] {
                let lower = piece_lower(half, target);
                if lower >= best.distance - tolerance {
                    dropped = dropped.min(lower);
                } else {
                    let start = pool.len();
                    pool.extend_from_slice(half);
                    heap.push(Piece {
                        lower,
                        sequence,
                        span: piece.span,
                        range,
                        controls: start,
                    });
                    sequence += 1;
                }
            }
        };
        lower = lower.min(unsplit).min(dropped).min(best.distance);
        best.lower = lower;
        // Lengths past the largest `f64` certify nothing.
        best.certain = best.distance.is_finite() && lower >= best.distance - tolerance;
        best
    }
}

/// Bounds on a piece of curve over a parameter range, as
/// [`PreparedNurbs::range_bounds`] gives them.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RangeBounds<const N: usize = 3> {
    /// A box holding the piece.
    pub bounds: [[f64; N]; 2],
    /// At least `|C'|` anywhere on the piece, with respect to the curve's
    /// own parameter.
    pub first: f64,
    /// At least `|C''|` anywhere on the piece.
    pub second: f64,
}

/// The closest point of a curve to a target, as
/// [`PreparedNurbs::closest`] found it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Closest<const N: usize = 3> {
    /// The curve's parameter there.
    pub parameter: f64,
    /// The prepared span the point lies in. Where the curve jumps at
    /// `parameter`, the point may be the span's limit there rather than
    /// what [`PreparedNurbs::evaluate`] gives; [`PreparedNurbs::evaluation_in`]
    /// with this span gives it either way.
    pub span: usize,
    /// The point.
    pub point: [f64; N],
    /// Its distance from the target.
    pub distance: f64,
    /// A distance no point of the curve is nearer than, up to rounding.
    /// Certain, it is within the tolerance of [`Self::distance`].
    pub lower: f64,
    /// Pieces of curve split during the search.
    pub pieces: u32,
    /// Whether the search finished within its piece budget, so `distance`
    /// is within the tolerance of the least.
    pub certain: bool,
}

/// A piece of a span waiting to be searched, ordered nearest box first,
/// then by when it was made.
struct Piece {
    lower: f64,
    sequence: u64,
    span: usize,
    range: [f64; 2],
    /// Where its homogeneous Bezier controls start in the pool.
    controls: usize,
}

impl PartialEq for Piece {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Piece {}

impl PartialOrd for Piece {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Piece {
    /// Reversed, so the max-heap pops the nearest, earliest piece.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .lower
            .total_cmp(&self.lower)
            .then(other.sequence.cmp(&self.sequence))
    }
}

/// The Bezier controls of `controls`' piece over `[u0, u1]` of its own
/// `[0, 1]`, into `out`: control `i` is the blossom at `p - i` copies of
/// `u0` and `i` of `u1`, by de Casteljau's scheme with one parameter per
/// level.
fn cut<const N: usize>(
    controls: &[WeightedPoint<N>],
    u0: f64,
    u1: f64,
    out: &mut [WeightedPoint<N>],
    scratch: &mut [WeightedPoint<N>],
) {
    let order = controls.len();
    for (i, slot) in out.iter_mut().enumerate() {
        scratch.copy_from_slice(controls);
        for r in 1..order {
            let t = if r < order - i { u0 } else { u1 };
            for j in 0..order - r {
                scratch[j] = WeightedPoint::lerp(scratch[j], scratch[j + 1], t);
            }
        }
        *slot = scratch[0];
    }
}

/// Upper bounds on `|C'|` and `|C''|`, with respect to the span's own
/// `[0, 1]`, over `range` of it, for the rational Bezier piece with
/// weighted `controls`, whose part over `range` is `piece`, as
/// [`PreparedNurbs::range_bounds`] describes.
fn derivative_bounds<const N: usize>(
    controls: &[WeightedPoint<N>],
    piece: &[WeightedPoint<N>],
    [u0, u1]: [f64; 2],
) -> [f64; 2] {
    let p = controls.len() - 1;
    // Weights within `[1, 2)` at most, by an exact power of two.
    let heaviest = controls.iter().map(|h| h.weight).fold(0.0, f64::max);
    let factor = reciprocal_power_of_two(heaviest);
    // Measured from the range's first point, which moves no derivative.
    let origin = piece[0].point();
    let a: Vec<[f64; N]> = controls
        .iter()
        .map(|h| core::array::from_fn(|k| (h.weighted[k] - h.weight * origin[k]) * factor))
        .collect();
    let w: Vec<[f64; 1]> = controls.iter().map(|h| [h.weight * factor]).collect();
    let pf = p as f64;
    // Hodographs: degree `p - 1` and `p - 2` Bezier coefficients.
    let a1 = hodograph(&a, pf);
    let w1 = hodograph(&w, pf);
    let a2 = hodograph(&a1, pf - 1.0);
    let w2 = hodograph(&w1, pf - 1.0);
    // `A' w - A w'`, as a Bezier polynomial of degree `2p - 1`.
    let mut numerator = product(&a1, &w);
    for (n, term) in numerator.iter_mut().zip(product(&a, &w1)) {
        *n = sub(*n, term);
    }
    let w_min = restrict(&w, u0, u1)
        .iter()
        .fold(f64::INFINITY, |m, w| m.min(w[0]));
    // What rounding the coefficients above may have lost, absolutely, so a
    // derivative near zero is not bounded by less than its error.
    let noise = 16.0 * pf * pf * f64::EPSILON * largest(&a) * largest(&w).max(1.0)
        + 16.0 * f64::from_bits(1);
    let first = (largest(&restrict(&numerator, u0, u1)) + noise) / (w_min * w_min);
    let reach = piece
        .iter()
        .map(|h| magnitude(sub(h.point(), origin)))
        .fold(0.0, upper);
    let second = (largest(&restrict(&a2, u0, u1))
        + noise
        + 2.0 * largest(&restrict(&w1, u0, u1)) * first
        + largest(&restrict(&w2, u0, u1)) * reach)
        / w_min;
    [first, second].map(|bound| if bound >= 0.0 { bound } else { f64::INFINITY })
}

/// The Bezier coefficients of the derivative of the polynomial with
/// coefficients `c`, of degree `degree`: `degree` times their differences.
fn hodograph<const M: usize>(c: &[[f64; M]], degree: f64) -> Vec<[f64; M]> {
    c.windows(2)
        .map(|e| core::array::from_fn(|k| degree * (e[1][k] - e[0][k])))
        .collect()
}

/// The Bezier coefficients over `[u0, u1]`, inside `[0, 1]`, of the
/// polynomial with coefficients `c` over `[0, 1]`: coefficient `i` is the
/// blossom at `n - i` copies of `u0` and `i` of `u1`, convex combinations
/// of `c`. Empty for an empty `c`.
fn restrict<const M: usize>(c: &[[f64; M]], u0: f64, u1: f64) -> Vec<[f64; M]> {
    let n = c.len();
    let mut scratch = c.to_vec();
    (0..n)
        .map(|i| {
            scratch.copy_from_slice(c);
            for r in 1..n {
                let t = if r < n - i { u0 } else { u1 };
                for j in 0..n - r {
                    scratch[j] =
                        core::array::from_fn(|k| (1.0 - t) * scratch[j][k] + t * scratch[j + 1][k]);
                }
            }
            scratch[0]
        })
        .collect()
}

/// The largest length among `v`, infinite if any is NaN.
fn largest<const M: usize>(v: &[[f64; M]]) -> f64 {
    v.iter().map(|x| magnitude(*x)).fold(0.0, upper)
}

/// The larger of two bounds, infinite if either is NaN, so arithmetic
/// that failed bounds nothing rather than vanishing from a maximum.
fn upper(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::INFINITY
    } else {
        a.max(b)
    }
}

/// The power of two `2^-e` for the exponent `e` of a positive normal `x`
/// below `2^1023`, as weights are, so `x 2^-e` lies in `[1, 2)`: its
/// biased exponent `1023 - e` is `2046` less `x`'s.
fn reciprocal_power_of_two(x: f64) -> f64 {
    let biased = (x.to_bits() >> 52) & 0x7FF;
    f64::from_bits((2046 - biased) << 52)
}

/// The Bezier coefficients of the product of vector polynomial `a` and
/// scalar polynomial `b`, given by theirs: `C(m, i) C(n, j) / C(m + n, i + j)`
/// weights `a_i b_j` toward coefficient `i + j`.
fn product<const N: usize>(a: &[[f64; N]], b: &[[f64; 1]]) -> Vec<[f64; N]> {
    let (m, n) = (a.len() - 1, b.len() - 1);
    let mut out = alloc::vec![[0.0; N]; m + n + 1];
    for (i, ai) in a.iter().enumerate() {
        for (j, &[bj]) in b.iter().enumerate() {
            let c = binomial(m, i) * binomial(n, j) / binomial(m + n, i + j);
            for k in 0..N {
                out[i + j][k] += c * ai[k] * bj;
            }
        }
    }
    out
}

/// `n` choose `k`, exact for the small degrees curves take.
fn binomial(n: usize, k: usize) -> f64 {
    (0..k).fold(1.0, |b, i| b * (n - i) as f64 / (i + 1) as f64)
}

fn bounds_of<const N: usize>(points: impl Iterator<Item = [f64; N]>) -> [[f64; N]; 2] {
    let mut lo = [f64::INFINITY; N];
    let mut hi = [f64::NEG_INFINITY; N];
    for p in points {
        for k in 0..N {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    [lo, hi]
}

/// A distance no point of the piece with weighted Bezier `controls` is
/// nearer to `target` than. Each point is a convex combination of the
/// controls' points, so it is no nearer than their box, nor nearer along
/// any direction `u` than the least of `u . (P - target)` over the
/// controls. With `u` toward the controls' mean, that second bound closes
/// in on the true distance as fast as a piece flattens, quadratically in
/// its length, where the box alone closes only linearly.
fn piece_lower<const N: usize>(controls: &[WeightedPoint<N>], target: [f64; N]) -> f64 {
    let points = controls.iter().map(WeightedPoint::point);
    let [lo, hi] = bounds_of(points.clone());
    let gap: [f64; N] =
        core::array::from_fn(|k| (lo[k] - target[k]).max(target[k] - hi[k]).max(0.0));
    let boxed = magnitude(gap);
    let count = controls.len() as f64;
    let mean = points.clone().fold([0.0; N], |sum, p| {
        core::array::from_fn(|k| sum[k] + p[k] / count)
    });
    let toward = sub(mean, target);
    let length = magnitude(toward);
    if !(length > 0.0 && length.is_finite()) {
        return boxed;
    }
    let u = toward.map(|c| c / length);
    let along = points
        .map(|p| dot(u, sub(p, target)))
        .fold(f64::INFINITY, f64::min);
    // Overflowed, the projection bounds nothing.
    if along.is_finite() {
        boxed.max(along)
    } else {
        boxed
    }
}

/// Splits the Bezier piece `controls` at its middle into `left` and
/// `right`, by de Casteljau's scheme on weighted controls. Level `r` of
/// the scheme starts with the left half's control `r` first, and ends with
/// the right half's control `n - 1 - r` last, which later levels do not
/// touch, so `right` is left holding the right half in order.
fn split<const N: usize>(
    controls: &[WeightedPoint<N>],
    left: &mut [WeightedPoint<N>],
    right: &mut [WeightedPoint<N>],
) {
    let n = controls.len();
    right.copy_from_slice(controls);
    for (r, slot) in left.iter_mut().enumerate() {
        *slot = right[0];
        for j in 0..n - 1 - r {
            right[j] = WeightedPoint::lerp(right[j], right[j + 1], 0.5);
        }
    }
}

/// Keeps the point at `t` of span `span`, `distance` from the target, if it
/// is nearer than the best so far, or as near and earlier.
fn offer<const N: usize>(
    best: &mut Closest<N>,
    span: usize,
    t: f64,
    point: [f64; N],
    distance: f64,
) {
    if distance < best.distance || (distance == best.distance && t < best.parameter) {
        best.parameter = t;
        best.span = span;
        best.point = point;
        best.distance = distance;
    }
}

/// Newton's method on `f(t) = (C(t) - target) . C'(t)`, from `t` within
/// `range` of span `span`, keeping the nearest point it visits.
fn polish<const N: usize>(
    curve: &PreparedNurbs<N>,
    span: usize,
    range: [f64; 2],
    mut t: f64,
    target: [f64; N],
    best: &mut Closest<N>,
) {
    for _ in 0..8 {
        let Some(e) = curve.evaluation_in(span, t) else {
            return;
        };
        let offset = sub(e.point, target);
        offer(best, span, t, e.point, magnitude(offset));
        let f = dot(offset, e.first);
        let slope = dot(e.first, e.first) + dot(offset, e.second);
        if slope.is_nan() || slope <= 0.0 {
            return;
        }
        let next = (t - f / slope).max(range[0]).min(range[1]);
        if next == t {
            return;
        }
        t = next;
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::f64::consts::{PI, TAU};

    use exedra_math::{norm, sub};

    use crate::{NurbsCurve, PreparedNurbs};

    fn curves() -> Vec<NurbsCurve> {
        let weighted = NurbsCurve::new(
            4,
            vec![0.0, 0.0, 0.0, 0.0, 0.4, 1.5, 2.0, 2.0, 2.0, 2.0],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 2.0, 0.5],
                [3.0, 2.5, -1.0],
                [4.0, -1.0, 0.0],
                [6.0, 0.0, 2.0],
                [7.0, 1.0, 1.0],
            ],
            Some(vec![1.0, 0.5, 2.0, 1.5, 0.75, 1.0]),
        )
        .unwrap();
        let unclamped = NurbsCurve::new(
            4,
            (0..10).map(f64::from).collect(),
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 3.0, 0.0],
                [2.0, -1.0, 1.0],
                [4.0, 2.0, 0.0],
                [5.0, 0.0, -2.0],
                [6.0, 1.0, 0.0],
            ],
            None,
        )
        .unwrap();
        let arc = NurbsCurve::arc(
            [1.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.5, 5.0],
        )
        .unwrap();
        vec![weighted, unclamped, arc]
    }

    /// The least distance from `target` to dense samples of `curve`.
    fn brute(curve: &PreparedNurbs, target: [f64; 3]) -> f64 {
        let [a, b] = curve.domain();
        (0..=20_000)
            .map(|i| a + (b - a) * f64::from(i) / 20_000.0)
            .map(|t| norm(sub(curve.evaluate(t), target)))
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn curvature_of_circles_lines_and_stops() {
        let circle =
            NurbsCurve::arc([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, TAU]).unwrap();
        for t in [0.1, 1.0, 3.0, 6.0] {
            let k = circle.view().evaluation(t).curvature().unwrap();
            assert!((k - 0.5).abs() < 1e-12, "{k}");
        }
        let line = NurbsCurve::line([0.0; 3], [1.0, 2.0, 3.0]).unwrap();
        assert_eq!(line.view().evaluation(0.5).curvature(), Some(0.0));
        let still = NurbsCurve::line([1.0; 3], [1.0; 3]).unwrap();
        assert_eq!(still.view().evaluation(0.5).curvature(), None);
    }

    #[test]
    fn bounds_hold_every_point() {
        for curve in curves() {
            let [lo, hi] = curve.bounds();
            let prepared = curve.view().prepare();
            let [a, b] = curve.domain();
            for i in 0..=500 {
                let t = a + (b - a) * f64::from(i) / 500.0;
                let p = curve.view().evaluate(t);
                assert!((0..3).all(|k| lo[k] - 1e-12 <= p[k] && p[k] <= hi[k] + 1e-12));
                let span = prepared.span_at(t);
                let [lo, hi] = prepared.span_bounds(span).unwrap();
                let p = prepared.evaluate(t);
                assert!((0..3).all(|k| lo[k] - 1e-12 <= p[k] && p[k] <= hi[k] + 1e-12));
            }
            assert!(prepared.span_bounds(prepared.spans()).is_none());
        }
    }

    #[test]
    fn closest_points_are_certified() {
        let targets = [
            [3.0, 1.0, 0.0],
            [-2.0, 4.0, 1.0],
            [5.5, 0.2, 1.4],
            [1.0, 1.0, 0.0],
            [10.0, -3.0, -2.0],
            [2.0, 1.2, 0.3],
        ];
        for curve in curves() {
            let prepared = curve.view().prepare();
            for target in targets {
                let found = prepared.closest(target, 1e-9, 10_000);
                let truth = brute(&prepared, target);
                assert!(found.certain, "{found:?}");
                assert!(found.distance <= truth + 1e-9, "{found:?} against {truth}");
                assert!(found.lower <= truth + 1e-12, "{found:?} against {truth}");
                assert!(found.distance - found.lower <= 1e-9 + 1e-15);
                let at = prepared.evaluate(found.parameter);
                assert!(norm(sub(at, found.point)) < 1e-12);
            }
        }
    }

    #[test]
    fn circles_are_nearest_where_expected_and_budgets_are_reported() {
        let circle = NurbsCurve::arc([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, TAU])
            .unwrap()
            .view()
            .prepare();
        // Every point is equally near, to rounding, so every piece must
        // be split until its bound is within the tolerance: the worst case.
        let center = circle.closest([0.0; 3], 1e-6, 10_000);
        assert!((center.distance - 1.0).abs() < 1e-15 && center.certain);
        // Outside, on the bisector of the second quarter.
        let outside = circle.closest([-3.0, 3.0, 0.0], 1e-12, 10_000);
        assert!((outside.parameter - 0.75 * PI).abs() < 1e-6, "{outside:?}");
        assert!((outside.distance - (18.0_f64.sqrt() - 1.0)).abs() < 1e-12);
        // With no pieces to split, the answer is uncertain but bounded.
        let rushed = circle.closest([-3.0, 3.0, 0.0], 1e-12, 0);
        assert!(!rushed.certain && rushed.lower <= outside.distance);
    }

    #[test]
    fn range_bounds_hold_derivatives_everywhere() {
        for curve in curves() {
            let prepared = curve.view().prepare();
            let [a, b] = prepared.domain();
            for (f0, f1) in [(0.0, 1.0), (0.1, 0.35), (0.42, 0.43), (0.6, 0.99)] {
                let range = [a + (b - a) * f0, a + (b - a) * f1];
                let bounds = prepared.range_bounds(range).unwrap();
                let [lo, hi] = bounds.bounds;
                let (mut first, mut second) = (0.0_f64, 0.0_f64);
                for i in 0..=400 {
                    let t = range[0] + (range[1] - range[0]) * f64::from(i) / 400.0;
                    let span = prepared.span_at(t);
                    // Both sides of a knot inside the range.
                    for span in [span.saturating_sub(1), span] {
                        let [s0, s1] = prepared.span_domain(span).unwrap();
                        if t < s0 || t > s1 {
                            continue;
                        }
                        let e = prepared.evaluation_in(span, t).unwrap();
                        assert!((0..3).all(|k| lo[k] - 1e-12 <= e.point[k] && e.point[k] <= hi[k] + 1e-12));
                        first = first.max(norm(e.first));
                        second = second.max(norm(e.second));
                    }
                }
                assert!(first <= bounds.first, "{first} over {bounds:?}");
                assert!(second <= bounds.second, "{second} over {bounds:?}");
                // Not wildly loose.
                assert!(
                    bounds.first <= 50.0 * first.max(1e-9),
                    "{first} under {bounds:?}"
                );
            }
            assert!(prepared.range_bounds([b, a]).is_none());
            assert!(prepared.range_bounds([a - 1.0, b]).is_none());
        }
        // A parabola's bounds are its own derivatives exactly.
        let parabola = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [2.0, 0.0, 0.0]],
            None,
        )
        .unwrap()
        .view()
        .prepare();
        let bounds = parabola.range_bounds([0.0, 1.0]).unwrap();
        assert!((bounds.second - 8.0).abs() < 1e-12, "{bounds:?}");
        assert!((bounds.first - 20.0_f64.sqrt()).abs() < 1e-12, "{bounds:?}");
    }

    #[test]
    fn narrow_ranges_and_slight_weights_keep_their_speed() {
        let line = NurbsCurve::line([1.0, 0.0, 0.0], [2.0, 0.0, 0.0])
            .unwrap()
            .view()
            .prepare();
        let bounds = line.range_bounds([0.5, 0.5_f64.next_up()]).unwrap();
        assert!(bounds.first >= 1.0, "{bounds:?}");
        // Weights scaled alike by a power of two move no point, and no
        // bound, however slight.
        let points: Vec<[f64; 3]> = (0..4)
            .map(|i| [1e-8 * f64::from(i), 1e-8 * f64::from(i * i % 3), 0.0])
            .collect();
        let weighted = |w: f64| {
            NurbsCurve::new(
                3,
                vec![0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0],
                points.clone(),
                Some(vec![w, 2.0 * w, w, 1.5 * w]),
            )
            .unwrap()
            .view()
            .prepare()
        };
        let slightest = f64::from_bits(0x2FF0_0000_0000_0000); // 2^-256
        let (heavy, slight) = (weighted(1.0), weighted(slightest));
        let bounds = heavy.range_bounds([0.0, 1.0]).unwrap();
        assert!(bounds.first > 1e-8, "{bounds:?}");
        assert_eq!(slight.range_bounds([0.0, 1.0]), Some(bounds));
    }

    #[test]
    fn pieces_dropped_by_the_tolerance_still_bound_the_distance() {
        let knots: Vec<f64> = [0.0, 1.0, 2.0].iter().flat_map(|&u| [u; 5]).collect();
        let points: Vec<[f64; 3]> = [2.0, -1.0, 4.0, -1.0, 2.0, 3.0, 3.0, 3.0, 3.0, 3.0]
            .iter()
            .map(|&x| [x, 0.0, 0.0])
            .collect();
        let curve = NurbsCurve::new(5, knots, points, None)
            .unwrap()
            .view()
            .prepare();
        let found = curve.closest([0.0; 3], 1.0, 1);
        let least = (0..=4000)
            .map(|i| f64::from(i) / 2000.0)
            .map(|t| curve.evaluate(t)[0].abs())
            .fold(f64::INFINITY, f64::min);
        assert!(found.lower <= least, "{found:?} against {least}");
        assert!(found.certain && found.distance - least <= 1.0, "{found:?}");
    }

    #[test]
    fn huge_curves_neither_overflow_nor_certify_falsely() {
        let line = NurbsCurve::line([0.0; 3], [1e200, 0.0, 0.0])
            .unwrap()
            .view()
            .prepare();
        let found = line.closest([2.5e199, 1e200, 0.0], 1.0, 200);
        assert!((found.distance / 1e200 - 1.0).abs() < 1e-12, "{found:?}");
        assert!(found.lower <= 1e200, "{found:?}");
        let bump = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![[0.0; 3], [1e200, 0.0, 0.0], [0.0; 3]],
            None,
        )
        .unwrap()
        .view()
        .prepare();
        let bounds = bump.range_bounds([0.0, 1.0]).unwrap();
        assert!(bounds.second >= 4e200, "{bounds:?}");
        let circle =
            NurbsCurve::arc([0.0; 3], [1e150, 0.0, 0.0], [0.0, 1e150, 0.0], [0.0, 1.0]).unwrap();
        let curvature = circle.view().evaluation(0.5).curvature().unwrap();
        assert!((curvature * 1e150 - 1.0).abs() < 1e-12, "{curvature}");
    }

    #[test]
    fn at_a_jump_results_name_their_side() {
        let jump = NurbsCurve::new(
            2,
            vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0],
            vec![
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [11.0, 0.0, 0.0],
            ],
            None,
        )
        .unwrap()
        .view()
        .prepare();
        let found = jump.closest([1.0, 0.0, 0.0], 1e-9, 100);
        assert_eq!((found.parameter, found.point), (1.0, [1.0, 0.0, 0.0]));
        let side = jump.evaluation_in(found.span, found.parameter).unwrap();
        assert_eq!(side.point, found.point);
        // The range up to the jump holds the piece before it.
        let bounds = jump.range_bounds([0.0, 1.0]).unwrap();
        assert_eq!(bounds.bounds[1][0], 1.0);
    }
}
