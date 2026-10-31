// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bounds and the closest point of a NURBS curve, with the certainty of
//! each stated.

use alloc::collections::BinaryHeap;
use alloc::vec::Vec;
use core::cmp::Ordering;

use exedra_math::cross;

use crate::vector::{WeightedPoint, dot, magnitude, sub};
use crate::{CubicBasis, CurveEvaluation, NurbsCurve, PreparedNurbs, ValidNurbs};

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
        self.bounds_per(range, 1.0)
    }

    /// As [`Self::range_bounds`], with derivatives taken against a
    /// parameter `unit` times coarser than the curve's: each span's own
    /// derivatives are divided by its width over `unit`, and that ratio's
    /// square ([`per_unit`]), which for a unit near the range's length
    /// stays representable however long or short the range is.
    fn bounds_per(&self, range: [f64; 2], unit: f64) -> Option<RangeBounds<N>> {
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
            let [first, second] = per_unit([first, second], width, unit);
            out.first = upper(out.first, first);
            out.second = upper(out.second, second);
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

impl<const N: usize> ValidNurbs<'_, N> {
    /// The first knot strictly inside `range` that occurs `degree` times
    /// or more, where the curve's tangent may jump: a possible corner.
    #[must_use]
    pub fn corner(&self, range: [f64; 2]) -> Option<f64> {
        let curve = self.curve();
        let degree = curve.order as usize - 1;
        let knots = curve.knots;
        knots.iter().enumerate().find_map(|(i, &u)| {
            let first = i == 0 || knots[i - 1] < u;
            let inside = range[0] < u && u < range[1];
            (first && inside && knots[i..].iter().take_while(|&&k| k == u).count() >= degree)
                .then_some(u)
        })
    }

    /// How far chords between uniform samples over `range` stray from the
    /// curve, from [`PreparedNurbs::range_bounds`]; `None` unless `range`
    /// is an interval of the domain the curve does not jump inside. At a
    /// jump at either end of the range, samples there are the side within
    /// it, as [`PreparedNurbs::evaluation_in`] gives them.
    ///
    /// Prepares the curve, which allocates; a caller sampling the same
    /// range at several step sizes keeps the result.
    #[must_use]
    pub fn chord_bound(&self, range: [f64; 2]) -> Option<ChordBound> {
        // A knot repeated `order` times inside the range is a jump, which
        // no chord across it can follow.
        let curve = self.curve();
        let order = curve.order as usize;
        let knots = curve.knots;
        let jumps = knots.windows(order).any(|run| {
            let u = run[0];
            range[0] < u && u < range[1] && run.iter().all(|&k| k == u)
        });
        if jumps {
            return None;
        }
        // Derivatives against the range's own `[0, 1]`, which a range
        // however long or short leaves representable.
        let span = range[1] - range[0];
        let bounds = self.prepare().bounds_per(range, span)?;
        Some(ChordBound {
            span,
            first: bounds.first,
            second: bounds.second,
            corner: self.corner(range),
        })
    }
}

impl CubicBasis {
    /// How far chords between uniform samples of the span with `controls`
    /// stray from it, over its own `[0, 1]`, as [`ChordBound`] describes;
    /// nothing is allocated.
    ///
    /// The span is the cubic `a u^3 + b u^2 + c u + d` its basis matrix
    /// gives. `C'' = 6 a u + 2 b` is linear, so its largest length is at an
    /// end: the bound is exact. `C' = 3 a u^2 + 2 b u + c` is within its
    /// Bernstein coefficients `c`, `c + b` and `c + 2 b + 3 a`. Both allow
    /// for rounding in forming the coefficients, in proportion to the
    /// largest control; a bound whose arithmetic overflows is infinite.
    #[must_use]
    pub fn chord_bound<const N: usize>(self, controls: [[f64; N]; 4]) -> ChordBound {
        let m = self.matrix();
        let power = |row: [f64; 4]| -> [f64; N] {
            core::array::from_fn(|k| {
                row.iter()
                    .zip(&controls)
                    .fold(0.0, |sum, (w, control)| sum + w * control[k])
            })
        };
        let (a, b, c) = (power(m[0]), power(m[1]), power(m[2]));
        let ends = [
            core::array::from_fn(|k| 2.0 * b[k]),
            core::array::from_fn(|k| 6.0 * a[k] + 2.0 * b[k]),
        ];
        let hodograph = [
            c,
            core::array::from_fn(|k| c[k] + b[k]),
            core::array::from_fn(|k| c[k] + 2.0 * b[k] + 3.0 * a[k]),
        ];
        let reach = controls.iter().map(|p| magnitude(*p)).fold(0.0, upper);
        // Relative to the controls, and absolute for subnormal ones.
        let allowance = 256.0 * f64::EPSILON * reach + 256.0 * f64::from_bits(1);
        let bound = |v: &[[f64; N]]| {
            let largest = v.iter().map(|x| magnitude(*x)).fold(0.0, upper) + allowance;
            if largest >= 0.0 {
                largest
            } else {
                f64::INFINITY
            }
        };
        ChordBound {
            span: 1.0,
            first: bound(&hodograph),
            second: bound(&ends),
            corner: None,
        }
    }
}

/// How far chords between uniform samples over a range of a curve stray
/// from it, as [`ValidNurbs::chord_bound`] finds it.
///
/// Where the curve's tangent turns smoothly, a chord `h` long in the
/// parameter strays at most `h^2 / 8` times the largest `|C''|`; across a
/// possible corner, every point is within half a step of a sample, so at
/// most `h / 2` times the largest `|C'|`. Both hold up to rounding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChordBound {
    /// The range's length in the curve's parameter.
    span: f64,
    /// The largest `|C'|` and `|C''|` against the range's own `[0, 1]`:
    /// the curve's, times the span and its square.
    first: f64,
    second: f64,
    corner: Option<f64>,
}

impl ChordBound {
    /// The farthest a chord between samples `step` apart in the parameter
    /// strays from the curve.
    #[must_use]
    pub fn deviation(&self, step: f64) -> f64 {
        self.deviation_over(step / self.span)
    }

    /// The deviation for samples `fraction` of the range apart.
    fn deviation_over(&self, fraction: f64) -> f64 {
        if self.corner.is_some() {
            0.5 * fraction * self.first
        } else {
            // One factor at a time, so neither overflows nor underflows on
            // the way where the product does not.
            0.125 * fraction * (fraction * self.second)
        }
    }

    /// The fewest uniform steps over the range whose chords all stray at
    /// most `tolerance`, found by bisection on the count, at most 32
    /// trials; `None` unless `tolerance` is positive and some count up to
    /// the largest `u32` suffices.
    #[must_use]
    pub fn steps(&self, tolerance: f64) -> Option<u32> {
        if tolerance.is_nan() || tolerance <= 0.0 {
            return None;
        }
        let fits = |steps: u32| self.deviation_over(1.0 / f64::from(steps)) <= tolerance;
        if !fits(u32::MAX) {
            return None;
        }
        // The fewest that fit lie in `(low, high]`.
        let (mut low, mut high) = (0_u32, u32::MAX);
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            if fits(middle) {
                high = middle;
            } else {
                low = middle;
            }
        }
        Some(high)
    }

    /// A knot inside the range where the tangent may jump, which the
    /// bound allows for.
    #[must_use]
    pub fn corner(&self) -> Option<f64> {
        self.corner
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
    // The span's controls themselves carry rounding at their absolute
    // scale, which measuring from the range's start does not remove; each
    // coefficient sums at most `2p` products of them.
    let absolute = controls
        .iter()
        .map(|h| magnitude(h.weighted) + h.weight * magnitude(origin))
        .fold(0.0, upper);
    let control_error = factor * 8.0 * controls.len() as f64 * f64::EPSILON * absolute;
    let noise = 16.0 * pf * pf * f64::EPSILON * largest(&a) * largest(&w).max(1.0)
        + 4.0 * pf * pf * control_error * largest(&w).max(1.0)
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
pub(crate) fn upper(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::INFINITY
    } else {
        a.max(b)
    }
}

/// A span's first and second derivative bounds, `[first, second]`
/// against its own `[0, 1]`, against a parameter of which `unit` spans one:
/// divided by `width / unit` and by its square. That ratio over- or
/// underflows for a width and unit far apart where the results do not, so
/// it is never formed: the mantissas divide, and the exponents' difference
/// scales by powers of two, which are exact. With `unit` 1 the results are
/// `first / width` and `second / width / width` exactly.
fn per_unit([first, second]: [f64; 2], width: f64, unit: f64) -> [f64; 2] {
    let (width_mantissa, width_exponent) = decompose(width);
    let (unit_mantissa, unit_exponent) = decompose(unit);
    let ratio = width_mantissa / unit_mantissa;
    let exponent = unit_exponent - width_exponent;
    [
        times_power_of_two(first / ratio, exponent),
        times_power_of_two(second / ratio / ratio, 2 * exponent),
    ]
}

/// A positive finite `x` as `m 2^e`, `m` in `[1, 2)`.
fn decompose(x: f64) -> (f64, i64) {
    if x < f64::MIN_POSITIVE {
        let (m, e) = decompose(x * f64::from_bits((1023 + 64) << 52));
        return (m, e - 64);
    }
    let biased = (x.to_bits() >> 52) & 0x7FF;
    let mantissa = f64::from_bits((x.to_bits() & 0x000F_FFFF_FFFF_FFFF) | (1023 << 52));
    (mantissa, biased.cast_signed() - 1023)
}

/// `x 2^e`, in steps of at most `2^1000`, so no step overflows or
/// underflows where the result does not.
fn times_power_of_two(mut x: f64, mut e: i64) -> f64 {
    let (up, down) = (
        f64::from_bits((1023 + 1000) << 52),
        f64::from_bits((1023 - 1000) << 52),
    );
    while e > 1000 {
        x *= up;
        e -= 1000;
    }
    while e < -1000 {
        x *= down;
        e += 1000;
    }
    #[expect(
        clippy::cast_sign_loss,
        reason = "e + 1023 lies in [23, 2023] after the steps above"
    )]
    let last = f64::from_bits(((e + 1023) as u64) << 52);
    x * last
}

/// The power of two `2^-e` for the exponent `e` of a positive normal `x`
/// below `2^1023`, as weights are, so `x 2^-e` lies in `[1, 2)`: its
/// biased exponent `1023 - e` is `2046` less `x`'s.
pub(crate) fn reciprocal_power_of_two(x: f64) -> f64 {
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
pub(crate) fn binomial(n: usize, k: usize) -> f64 {
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
    fn weights_that_would_lose_coordinates_are_refused_and_others_kept() {
        use crate::NurbsError;
        // Weights of 2^-256 that push a control's coordinates below the
        // smallest normal `f64` lose the digits projection magnifies.
        let w = f64::from_bits(0x2FF0_0000_0000_0000);
        let r = f64::from_bits(1) / w;
        let slight = NurbsCurve::<1>::new(
            2,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![[0.0], [2.0 * r / 3.0]],
            Some(vec![w, 3.0 * w]),
        );
        assert_eq!(slight.err(), Some(NurbsError::Point { index: 1 }));
        let tiny = 2.0_f64.powi(-900);
        let mixed = NurbsCurve::<1>::new(
            2,
            vec![0.0, 0.0, 1.0, 2.0, 2.0],
            vec![[0.0], [tiny], [0.0]],
            Some(vec![w, w, 1.0]),
        );
        assert_eq!(mixed.err(), Some(NurbsError::Point { index: 1 }));
        // Representable ones are evaluated as given: no overflow near the
        // largest `f64`, and spans sharing a tiny control agree on it.
        let large = NurbsCurve::<1>::new(
            2,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![[f64::MAX], [f64::MAX]],
            Some(vec![0.75, 0.75]),
        )
        .unwrap();
        assert_eq!(large.view().evaluate(0.5), [f64::MAX]);
        assert_eq!(large.view().prepare().evaluate(0.5), [f64::MAX]);
        let shared = NurbsCurve::<1>::new(
            2,
            vec![0.0, 0.0, 1.0, 2.0, 2.0],
            vec![[0.0], [tiny], [0.0]],
            Some(vec![1.0, 1.0, 2.0_f64.powi(256)]),
        )
        .unwrap();
        let prepared = shared.view().prepare();
        assert_eq!(prepared.evaluation_in(0, 1.0).unwrap().point, [tiny]);
        assert_eq!(prepared.evaluation_in(1, 1.0).unwrap().point, [tiny]);
        // A constant curve over a short span far from the origin stays
        // still, with no overflow in its rational derivative.
        let still = NurbsCurve::<1>::new(
            2,
            vec![0.0, 0.0, 2.0_f64.powi(-27), 2.0_f64.powi(-27)],
            vec![[2.0_f64.powi(998)]; 2],
            Some(vec![0.5, 0.75]),
        )
        .unwrap();
        assert_eq!(still.view().prepare().lengths(1), [0.0, 0.0]);
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

    /// The farthest the polyline through `steps` uniform samples of `at`
    /// over `[0, 1]` strays from the curve, measured densely.
    fn chord_error(at: impl Fn(f64) -> [f64; 3], steps: u32) -> f64 {
        let mut worst = 0.0_f64;
        for k in 0..steps {
            let (t0, t1) = (
                f64::from(k) / f64::from(steps),
                f64::from(k + 1) / f64::from(steps),
            );
            let (a, b) = (at(t0), at(t1));
            for i in 1..64 {
                let t = t0 + (t1 - t0) * f64::from(i) / 64.0;
                let p = at(t);
                let d = sub(b, a);
                let s = (exedra_math::dot(sub(p, a), d) / exedra_math::dot(d, d)).clamp(0.0, 1.0);
                let off = norm(sub(p, [a[0] + s * d[0], a[1] + s * d[1], a[2] + s * d[2]]));
                worst = worst.max(off);
            }
        }
        worst
    }

    #[test]
    fn uniform_chords_meet_their_tolerance() {
        use crate::CubicBasis;
        let controls = [
            [0.0, 0.0, 0.0],
            [1.0, 3.0, 0.5],
            [3.0, -2.0, 1.0],
            [4.0, 1.0, 0.0],
        ];
        for basis in [
            CubicBasis::Bezier,
            CubicBasis::BSpline,
            CubicBasis::CatmullRom,
            CubicBasis::Hermite,
        ] {
            let bound = basis.chord_bound(controls);
            for tolerance in [0.1, 1e-3, 1e-6] {
                let steps = bound.steps(tolerance).unwrap();
                assert!(bound.deviation(1.0 / f64::from(steps)) <= tolerance);
                let error = chord_error(|u| basis.evaluate(controls, u), steps);
                assert!(error <= tolerance, "{basis:?} {tolerance}: {error}");
                // Not wildly more than needed.
                if steps > 4 {
                    let fewer = chord_error(|u| basis.evaluate(controls, u), steps / 4);
                    assert!(fewer > tolerance, "{basis:?} {tolerance}: {steps} steps");
                }
            }
        }
        // A cubic Bezier's second derivative bound is exact: 6 times its
        // largest control second difference.
        let bound = CubicBasis::Bezier.chord_bound(controls);
        assert!((bound.deviation(1.0) * 8.0 / 6.0 - 67.25_f64.sqrt()).abs() < 1e-9);
        assert_eq!(bound.steps(0.0), None);
        assert_eq!(bound.steps(f64::NAN), None);
        assert_eq!(
            CubicBasis::Bezier
                .chord_bound([[f64::MAX; 3]; 4])
                .steps(1.0),
            None
        );
    }

    #[test]
    fn chords_across_a_corner_are_bounded_by_speed() {
        // A quadratic with a double knot at 1: a corner there.
        let curve = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 2.0],
            vec![
                [0.0; 3],
                [1.0, 2.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 2.0, 0.0],
                [4.0, 0.0, 0.0],
            ],
            None,
        )
        .unwrap();
        let view = curve.view();
        assert_eq!(view.corner([0.0, 2.0]), Some(1.0));
        assert_eq!(view.corner([0.0, 1.0]), None);
        let bound = view.chord_bound([0.0, 2.0]).unwrap();
        assert_eq!(bound.corner(), Some(1.0));
        for tolerance in [0.1, 1e-3] {
            // An odd count puts no sample on the corner.
            let steps = bound.steps(tolerance).unwrap() | 1;
            let error = chord_error(|u| view.evaluate(2.0 * u), steps);
            assert!(error <= tolerance, "{tolerance}: {error}");
        }
        // Either side alone turns smoothly.
        let side = view.chord_bound([0.0, 1.0]).unwrap();
        assert_eq!(side.corner(), None);
        let steps = side.steps(1e-3).unwrap();
        assert!(chord_error(|u| view.evaluate(u), steps) <= 1e-3);
    }

    #[test]
    fn chord_counts_survive_extreme_scales_and_refuse_jumps() {
        // A slight curve over a vast parameter range: one chord suffices,
        // and finding so takes no long correction.
        let vast = NurbsCurve::<2>::new(
            3,
            vec![0.0, 0.0, 0.0, 1e200, 1e200, 1e200],
            vec![[0.0, 0.0], [0.0, 1e100], [1e100, 0.0]],
            None,
        )
        .unwrap();
        let bound = vast.view().chord_bound([0.0, 1e200]).unwrap();
        let steps = bound.steps(1e100).unwrap();
        assert!(steps <= 2 && bound.deviation(1e200 / f64::from(steps)) <= 1e100);
        // Constant pieces between jumps: no chord across one follows it.
        let jumps = NurbsCurve::<2>::new(
            2,
            vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0],
            vec![
                [0.0, 0.0],
                [0.0, 0.0],
                [0.0, 1.0],
                [0.0, 1.0],
                [1.0, 0.0],
                [1.0, 0.0],
            ],
            None,
        )
        .unwrap();
        assert_eq!(jumps.view().chord_bound([0.0, 3.0]), None);
        assert!(jumps.view().chord_bound([1.0, 2.0]).is_some());
        // Subnormal controls keep an absolute allowance: this span's middle
        // is 9/8 of the least subnormal above its chord.
        let q = f64::from_bits(1);
        let tiny = crate::CubicBasis::CatmullRom.chord_bound([[-8.0 * q], [q], [q], [-8.0 * q]]);
        assert!(tiny.deviation(1.0) >= 9.0 / 8.0 * q, "{tiny:?}");
        assert!(tiny.steps(q).is_none_or(|steps| steps > 1));
    }

    #[test]
    fn step_counts_are_the_fewest_up_to_the_largest_u32() {
        use crate::CubicBasis;
        let flat = CubicBasis::Bezier.chord_bound([[0.0], [0.0], [1.0], [1.0]]);
        assert_eq!(flat.steps(flat.deviation(1.0 / 3.0)), Some(3));
        let most = CubicBasis::Bezier.chord_bound([[-2.0], [-2.0], [-2.0], [1.0]]);
        let steps = most
            .steps(most.deviation(1.0 / f64::from(u32::MAX)))
            .unwrap();
        assert!(steps >= u32::MAX - 4, "{steps}");
    }

    #[test]
    fn chord_counts_hold_over_vast_ranges() {
        // A parabola over a range of 1e200: its curve-parameter second
        // derivative underflows, but its chords still stray.
        let vast = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 1e200, 1e200, 1e200],
            vec![[0.0; 3], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
            None,
        )
        .unwrap();
        let view = vast.view();
        let steps = view.chord_bound([0.0, 1e200]).unwrap().steps(1e-3).unwrap();
        assert!(
            chord_error(|u| view.evaluate(u * 1e200), steps) <= 1e-3,
            "{steps}"
        );
        // A cornered line whose speed bound is near the bottom of `f64`:
        // about 1e5 steps, not the largest `u32`.
        let slow = NurbsCurve::<1>::new(
            2,
            vec![0.0, 0.0, 1e200, 2e200, 2e200],
            vec![[0.0], [1e-110], [0.0]],
            None,
        )
        .unwrap();
        let steps = slow
            .view()
            .chord_bound([0.0, 2e200])
            .unwrap()
            .steps(1e-115)
            .unwrap();
        assert!((50_000..=200_000).contains(&steps), "{steps}");
    }

    #[test]
    fn chord_counts_survive_spans_far_from_their_range() {
        // A span 2^600 long sampled over 2^-424 of it: the ratio of the
        // two overflows, but the bounds do not.
        let span = 2.0_f64.powi(600);
        let reach = 2.0_f64.powi(1018);
        let wide = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, span, span, span],
            vec![[0.0; 3], [reach, 0.0, 0.0], [0.0, reach, 0.0]],
            None,
        )
        .unwrap();
        let bound = wide.view().chord_bound([0.0, 2.0_f64.powi(-424)]).unwrap();
        assert!(
            bound
                .steps(2.0_f64.powi(-1040))
                .is_none_or(|steps| steps >= 16)
        );
        // A span 2^-600 wide beside one of 2^600: the ratio underflows, but
        // one chord within 2^300 still suffices.
        let narrow = NurbsCurve::<1>::new(
            2,
            vec![
                0.0,
                0.0,
                2.0_f64.powi(-600),
                2.0_f64.powi(600),
                2.0_f64.powi(600),
            ],
            vec![[0.0], [2.0_f64.powi(-900)], [0.0]],
            None,
        )
        .unwrap();
        let bound = narrow.view().chord_bound([0.0, 2.0_f64.powi(600)]).unwrap();
        assert_eq!(bound.steps(2.0_f64.powi(300)), Some(1));
    }
}
