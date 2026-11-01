// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Rational Bezier segments of 2D profiles.

use alloc::{sync::Arc, vec::Vec};

use crate::intersect::{BezierEnclosure, IntersectError, MAX_ORDER};
use crate::measure::integrate;
use crate::nurbs::{local_evaluation, representable, valid_weight};
use crate::profile::arc::{cross, dot, length, sub};
use crate::vector::WeightedPoint;
use core::f64::consts::TAU;

/// The controls of a rational Bezier [`Segment`](super::Segment) between
/// its ends, which are the segment's start and end exactly, and its
/// weights. Cloning shares them.
///
/// A segment of `n` interior controls has order `n + 2`, at most
/// [`MAX_ORDER`]; weights, one per control including the ends, lie in
/// `[2^-256, 2^256]` as [`crate::Nurbs::validate`] requires, and `None`
/// makes the segment polynomial. Two limits keep every measure of it
/// within what `f64` resolves: in standard form, by the reparameterization
/// that makes its end weights equal and moves no point, its weights lie
/// within `2^32` of one another, and its controls span at least `2^-40` of
/// their largest coordinate. [`super::Loop::new`] checks them. A
/// multi-span spline enters a profile as one segment per span.
#[derive(Clone, Debug, PartialEq)]
pub struct Bezier(Arc<Controls>);

#[derive(Debug, PartialEq)]
struct Controls {
    interior: Vec<[f64; 2]>,
    weights: Option<Vec<f64>>,
}

impl Bezier {
    /// A segment's `interior` controls and its `weights`, one per control
    /// including the ends, or `None` for a polynomial segment.
    #[must_use]
    pub fn new(interior: Vec<[f64; 2]>, weights: Option<Vec<f64>>) -> Self {
        Self(Arc::new(Controls { interior, weights }))
    }

    /// The controls between the segment's ends.
    #[must_use]
    pub fn interior(&self) -> &[[f64; 2]] {
        &self.0.interior
    }

    /// The weights, one per control including the ends, or `None` for a
    /// polynomial segment.
    #[must_use]
    pub fn weights(&self) -> Option<&[f64]> {
        self.0.weights.as_deref()
    }

    /// Controls per segment: the interior ones and the two ends.
    #[must_use]
    pub fn order(&self) -> usize {
        self.0.interior.len() + 2
    }

    /// The same curve traversed the other way.
    #[must_use]
    pub(crate) fn reversed(&self) -> Self {
        Self::new(
            self.0.interior.iter().rev().copied().collect(),
            self.0
                .weights
                .as_ref()
                .map(|w| w.iter().rev().copied().collect()),
        )
    }

    /// These controls moved by `map`, weights kept.
    #[must_use]
    pub(crate) fn mapped(&self, map: impl Fn([f64; 2]) -> [f64; 2]) -> Self {
        Self::new(
            self.0.interior.iter().map(|&p| map(p)).collect(),
            self.0.weights.clone(),
        )
    }

    /// Whether a segment from `from` to `to` with these controls is one
    /// a profile accepts: an order in `2..=MAX_ORDER`, one weight per
    /// control, and every control one [`crate::Nurbs::validate`] accepts.
    pub(crate) fn valid(&self, from: [f64; 2], to: [f64; 2]) -> bool {
        let order = self.order();
        if order > MAX_ORDER || self.weights().is_some_and(|w| w.len() != order) {
            return false;
        }
        let weight = |i: usize| self.weights().map_or(1.0, |w| w[i]);
        if !((0..order).all(|i| valid_weight(weight(i)))
            && self
                .controls(from, to)
                .enumerate()
                .all(|(i, p)| p.iter().all(|c| c.is_finite()) && representable(&p, weight(i))))
        {
            return false;
        }
        // Weights in standard form, end weights equal, within `2^32` of one
        // another: `f64` then carries the curve's pace through every
        // computation on it.
        let n = (order - 1) as f64;
        let lambda = libm::pow(weight(0) / weight(order - 1), 1.0 / n);
        let standard: Vec<f64> = (0..order)
            .map(|i| weight(i) / weight(0) * libm::pow(lambda, i as f64))
            .collect();
        let (lo, hi) = standard
            .iter()
            .fold((f64::INFINITY, 0.0_f64), |(lo, hi), &w| {
                (lo.min(w), hi.max(w))
            });
        if hi.is_nan() || hi > MAX_SPREAD * lo {
            return false;
        }
        // A shape `f64` resolves: the controls span at least `2^-40` of
        // their largest coordinate.
        let [lo, hi] = self.controls(from, to).fold(
            [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]],
            |[lo, hi], p| {
                [
                    [lo[0].min(p[0]), lo[1].min(p[1])],
                    [hi[0].max(p[0]), hi[1].max(p[1])],
                ]
            },
        );
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        let magnitude = [lo, hi]
            .into_iter()
            .flatten()
            .fold(0.0_f64, |m, c| m.max(c.abs()));
        extent >= libm::ldexp(magnitude, -40)
    }

    /// Every control in order, from the segment's start `from` to its end
    /// `to`.
    pub fn controls(&self, from: [f64; 2], to: [f64; 2]) -> impl Iterator<Item = [f64; 2]> {
        core::iter::once(from)
            .chain(self.0.interior.iter().copied())
            .chain(core::iter::once(to))
    }
}

/// The widest spread of a Bezier segment's weights in standard form.
pub(crate) const MAX_SPREAD: f64 = 4_294_967_296.0; // 2^32

/// A rational Bezier segment placed between its ends.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BezierPiece {
    pub(crate) from: [f64; 2],
    pub(crate) to: [f64; 2],
    pub(crate) bezier: Bezier,
}

impl BezierPiece {
    /// The homogeneous controls.
    fn weighted(&self) -> Vec<WeightedPoint<2>> {
        let weight = |i: usize| self.bezier.weights().map_or(1.0, |w| w[i]);
        self.bezier
            .controls(self.from, self.to)
            .enumerate()
            .map(|(i, p)| WeightedPoint::new(p, weight(i)))
            .collect()
    }

    /// The box of the controls, which holds the curve: its weights are
    /// positive.
    pub(crate) fn bounds(&self) -> [[f64; 2]; 2] {
        self.bezier.controls(self.from, self.to).fold(
            [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]],
            |[lo, hi], p| {
                [
                    [lo[0].min(p[0]), lo[1].min(p[1])],
                    [hi[0].max(p[0]), hi[1].max(p[1])],
                ]
            },
        )
    }

    /// The enclosure of the exact curve, for proofs.
    pub(crate) fn enclosure(&self) -> Result<BezierEnclosure, IntersectError> {
        let points: Vec<[f64; 2]> = self.bezier.controls(self.from, self.to).collect();
        BezierEnclosure::new(&points, self.bezier.weights())
    }

    /// The point at `t` in `[0, 1]`.
    pub(crate) fn point_at(&self, t: f64) -> [f64; 2] {
        if t <= 0.0 {
            return self.from;
        }
        if t >= 1.0 {
            return self.to;
        }
        let controls = self.weighted();
        let mut scratch = controls.clone();
        local_evaluation(&controls, 1.0, t, &mut scratch).point
    }

    /// The length, by adaptive Gauss-Legendre quadrature over balanced
    /// pieces ([`Self::balanced`]): halves are taken until they agree with
    /// their whole to `2^-44` of the curve's size.
    pub(crate) fn length(&self) -> f64 {
        let scale = self.extent();
        let mut sum = 0.0;
        for (controls, _) in self.balanced() {
            let mut scratch = controls.clone();
            let mut speed =
                |t: f64| length(local_evaluation(&controls, 1.0, t, &mut scratch).first);
            sum += adaptive(&mut speed, scale, 64.0 * f64::EPSILON * scale);
        }
        sum
    }

    /// The diagonal of the controls' box: the segment's own extent, which
    /// translation leaves alone, at least the smallest normal number.
    fn extent(&self) -> f64 {
        let [lo, hi] = self.bounds();
        libm::hypot(hi[0] - lo[0], hi[1] - lo[1]).max(f64::MIN_POSITIVE)
    }

    /// The largest coordinate magnitude of the controls, at least the
    /// smallest normal number.
    fn size(&self) -> f64 {
        self.bezier
            .controls(self.from, self.to)
            .flatten()
            .fold(f64::MIN_POSITIVE, |m, c| m.max(c.abs()))
    }

    /// [`Self::segment_integrals`] in closed form for a polynomial segment,
    /// its weights all equal: Green's integrands are then polynomials
    /// whose Bernstein coefficients the controls give, and the integral of
    /// a Bernstein polynomial over `[0, 1]` is its coefficients' mean.
    fn polynomial_integrals(&self) -> Option<(f64, [f64; 2])> {
        if let Some(w) = self.bezier.weights()
            && w.iter().any(|&x| x.to_bits() != w[0].to_bits())
        {
            return None;
        }
        let points: Vec<[f64; 2]> = self
            .bezier
            .controls(self.from, self.to)
            .map(|p| sub(p, self.from))
            .collect();
        let n = points.len() - 1;
        let coordinate = |k: usize| points.iter().map(|p| p[k]).collect::<Vec<f64>>();
        let derivative = |v: &[f64]| {
            v.windows(2)
                .map(|x| n as f64 * (x[1] - x[0]))
                .collect::<Vec<f64>>()
        };
        let (x, y) = (coordinate(0), coordinate(1));
        let (dx, dy) = (derivative(&x), derivative(&y));
        let mean = |v: Vec<f64>| v.iter().sum::<f64>() / v.len() as f64;
        let area = 0.5 * (mean(product(&x, &dy)) - mean(product(&y, &dx)));
        let mx = 0.5 * mean(product(&product(&x, &x), &dy));
        let my = -0.5 * mean(product(&product(&y, &y), &dx));
        let d = sub(self.to, self.from);
        Some((
            area,
            [mx - d[0] * d[0] * d[1] / 6.0, my + d[1] * d[1] * d[0] / 6.0],
        ))
    }

    /// The curve as pieces whose weights, in standard form (end weights
    /// equal, by the reparameterization that leaves the curve unchanged),
    /// lie within a factor of four of each other, so a quadrature sees each
    /// traversed at a nearly even pace, and whose controls turn less than
    /// an eighth of a turn, so no turn hides between its nodes. Heavy interior weights
    /// rush a rational curve through its ends in parameter, past any fixed
    /// quadrature's nodes; halving takes such a weight to about its square
    /// root, so a handful of levels balance any weight validation accepts.
    /// Each comes with how far rounding may have moved it ([`drift`]).
    fn balanced(&self) -> Vec<(Vec<WeightedPoint<2>>, f64)> {
        let mut done = Vec::new();
        let weighted = self.weighted();
        let standard = standard_form(&weighted);
        let start = drift(&weighted, false).max(drift(&standard, false));
        let mut stack = alloc::vec![(standard, 0_u32, start)];
        while let Some((standard, depth, moved)) = stack.pop() {
            let (lo, hi) = standard
                .iter()
                .fold((f64::INFINITY, 0.0_f64), |(lo, hi), c| {
                    (lo.min(c.weight), hi.max(c.weight))
                });
            let settled = hi <= 4.0 * lo && gentle(&standard);
            if settled || depth >= 64 || done.len() + stack.len() >= 4096 {
                done.push((standard, moved));
                continue;
            }
            // Halved in standard form, so each half's spread is about the
            // square root of the whole's.
            let (left, right) = split(&standard);
            let halved = moved + drift(&standard, true);
            for half in [right, left] {
                let reweighted = standard_form(&half);
                let moved = halved + drift(&half, false).max(drift(&reweighted, false));
                stack.push((reweighted, depth + 1, moved));
            }
        }
        done
    }

    /// The signed area between the chord and the curve (positive where the
    /// curve passes right of the chord's direction, as a counter-clockwise
    /// loop bulges), and its first moments about `from`, by Green's
    /// theorem along the curve and back along the chord, the curve's part
    /// in closed form for a polynomial segment, and otherwise by adaptive
    /// Gauss-Legendre quadrature over balanced pieces, an estimate to the
    /// stated tolerance.
    pub(crate) fn segment_integrals(&self) -> (f64, [f64; 2]) {
        if let Some(exact) = self.polynomial_integrals() {
            return exact;
        }
        let from = self.from;
        let size = self.extent();
        let reach = self.size();
        let (mut area, mut mx, mut my) = (0.0, 0.0, 0.0);
        for (controls, _) in self.balanced() {
            let mut scratch = controls.clone();
            let mut integral = |k: usize| {
                let mut term = |t: f64| {
                    let e = local_evaluation(&controls, 1.0, t, &mut scratch);
                    let p = sub(e.point, from);
                    match k {
                        0 => 0.5 * cross(p, e.first),
                        1 => 0.5 * p[0] * p[0] * e.first[1],
                        _ => -0.5 * p[1] * p[1] * e.first[0],
                    }
                };
                // Points far from the origin carry its rounding into `p`.
                let noise = 64.0 * f64::EPSILON * reach * size;
                if k == 0 {
                    adaptive(&mut term, size * size, noise)
                } else {
                    adaptive(&mut term, size * size * size, noise * size)
                }
            };
            area += integral(0);
            mx += integral(1);
            my += integral(2);
        }
        // Back along the chord, from `to` to `from`, in the same forms: the
        // area term vanishes there, and the moments are closed-form.
        let d = sub(self.to, from);
        (
            area,
            [mx - d[0] * d[0] * d[1] / 6.0, my + d[1] * d[1] * d[0] / 6.0],
        )
    }

    /// The distance from `point` to the curve, to rounding. Its nearest
    /// points are the ends and the roots of `(C - p) . C'`, which times the
    /// cube of the weight is a polynomial whose Bernstein coefficients are
    /// formed from the controls ([`critical`]). Starting from balanced
    /// pieces, pieces whose control box lies no nearer than the nearest
    /// point found are dropped, and the rest halved until their
    /// coefficients change sign at most once: then the piece has at most
    /// one nearest point inside it, found by bisection. Where coefficients
    /// vanish within their rounding, near a nearest point or along an arc
    /// about `point`, pieces are halved until a sample and a lower bound,
    /// from the box or from the derivative's bound, agree to rounding.
    pub(crate) fn distance_to(&self, point: [f64; 2]) -> f64 {
        self.distance_bounds(point).0
    }

    /// [`Self::distance_to`] with a lower bound: the distance to a curve
    /// point found, and a bound below which the curve comes no nearer,
    /// equal to it wherever the search settled but for the rounding that
    /// halving and reweighting the controls may have moved the curve by
    /// ([`drift`]), and that evaluating it may carry.
    pub(crate) fn distance_bounds(&self, point: [f64; 2]) -> (f64, f64) {
        // Within the rounding of distances at these coordinates.
        let tolerance = 32.0 * f64::EPSILON * point.iter().fold(self.size(), |m, c| m.max(c.abs()));
        let mut best = length(sub(self.from, point)).min(length(sub(self.to, point)));
        // The least lower bound of the pieces the search let go unsettled.
        let mut floor = f64::INFINITY;
        // `g` is formed in normalized coordinates, positions relative to
        // `point` over `reach` and weights over `heaviest`, both powers of
        // two, so it neither under- nor overflows at any scale; the same
        // parameter serves both.
        // The most any piece searched may lie from the curve, and any of
        // its evaluations from the piece.
        let mut moved = 0.0_f64;
        let mut stack = Vec::new();
        for (controls, drifted) in self.balanced() {
            let power = |x: f64| libm::ldexp(1.0, libm::ilogb(x) + 1);
            let far = controls
                .iter()
                .map(|c| {
                    let d = sub(c.point(), point);
                    d[0].abs().max(d[1].abs())
                })
                .fold(0.0_f64, f64::max);
            if far == 0.0 {
                return (0.0, 0.0);
            }
            let (reach, heaviest) = (
                power(far),
                power(controls.iter().map(|c| c.weight).fold(0.0_f64, f64::max)),
            );
            let normalized: Vec<WeightedPoint<2>> = controls
                .iter()
                .map(|c| {
                    let d = sub(c.point(), point);
                    WeightedPoint::new([d[0] / reach, d[1] / reach], c.weight / heaviest)
                })
                .collect();
            let g = critical(&normalized, [0.0; 2]);
            stack.push((controls, g, 0_u32, reach, heaviest, drifted));
        }
        while let Some((controls, (g, rounding), depth, reach, heaviest, drifted)) = stack.pop() {
            // Evaluation rounds like one more halving.
            moved = moved.max(drifted + drift(&controls, true));
            let [lo, hi] = hull_box(&controls);
            let outside = [0, 1].map(|k| (lo[k] - point[k]).max(point[k] - hi[k]).max(0.0));
            if libm::hypot(outside[0], outside[1]) >= best {
                continue;
            }
            let at = |t: f64| {
                let mut scratch = controls.clone();
                length(sub(
                    local_evaluation(&controls, 1.0, t, &mut scratch).point,
                    point,
                ))
            };
            best = best.min(at(0.0)).min(at(1.0));
            let certain: Vec<f64> = g
                .iter()
                .zip(&rounding)
                .filter(|(v, r)| v.abs() > **r)
                .map(|(v, _)| *v)
                .collect();
            if certain.is_empty() || depth >= 40 {
                let sample = at(0.5);
                best = best.min(sample);
                // With every coefficient within its rounding, the distance's
                // derivative `g / (W^3 |C - p|)` bounds how far it can fall
                // below a sample over the piece; otherwise only the box does.
                let mut lower = libm::hypot(outside[0], outside[1]);
                if certain.is_empty() && lower > 0.0 {
                    let slope = g
                        .iter()
                        .zip(&rounding)
                        .map(|(v, r)| v.abs() + r)
                        .fold(0.0, f64::max);
                    let lightest = controls
                        .iter()
                        .map(|c| c.weight)
                        .fold(f64::INFINITY, f64::min)
                        / heaviest;
                    // The piece spans `2^-depth` of its balanced piece's
                    // parameter, in which `g` was formed, and normalized
                    // distances are `reach` times smaller.
                    let width = libm::ldexp(1.0, -i32::try_from(depth).unwrap_or(i32::MAX));
                    let fall =
                        reach * slope * width / (lightest * lightest * lightest * (lower / reach));
                    if fall.is_finite() {
                        lower = lower.max(sample - fall);
                    }
                }
                if best - lower <= tolerance {
                    floor = floor.min(lower);
                    continue;
                }
                if depth >= 60 {
                    floor = floor.min(lower);
                    continue;
                }
            } else {
                let changes = certain
                    .windows(2)
                    .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
                    .count();
                if certain.len() == g.len() && changes <= 1 {
                    if changes == 1 {
                        // The one root lies in the bracket; the distance
                        // runs monotone to either side of it, and within it
                        // falls no faster than the speed bound.
                        let (lo_t, hi_t) = root(&g, &rounding);
                        let (da, db) = (at(lo_t), at(hi_t));
                        best = best.min(da).min(db).min(at(0.5 * (lo_t + hi_t)));
                        floor = floor.min(da.min(db) - (hi_t - lo_t) * speed_bound(&controls));
                    }
                    continue;
                }
            }
            let (left, right) = split(&controls);
            let ((gl, rl), (gr, rr)) = (
                split_scalar(&g, &rounding, true),
                split_scalar(&g, &rounding, false),
            );
            let drifted = drifted + drift(&controls, true);
            stack.push((left, (gl, rl), depth + 1, reach, heaviest, drifted));
            stack.push((right, (gr, rr), depth + 1, reach, heaviest, drifted));
        }
        (best, floor.min(best) - moved)
    }

    /// The angle the curve turns about `point`, which must not lie on it,
    /// or NaN where the curve crosses the ray leftward from the point
    /// within `allowance` of it: the change in its ends' angles plus a
    /// whole turn for each signed crossing of that ray, as `atan2` jumps
    /// there, as Kurbo counts winding. Crossings are counted along one
    /// unbroken chain from the authored start through balanced pieces, in
    /// order, to the authored end, the joins between pieces' rounded ends
    /// included, so every jump in angle is counted once: pieces are halved
    /// until their controls' heights, which bound the curve's, run one way,
    /// so each crosses the ray's height once at most, located by bisection.
    /// The chain lies within rounding of the curve, as far as the second
    /// value says ([`drift`]), so winds about any point farther from the
    /// curve as the curve does.
    pub(crate) fn winding_angle(&self, point: [f64; 2], allowance: f64) -> (f64, f64) {
        let (a, b) = (sub(self.from, point), sub(self.to, point));
        // Pieces in order, each one-way in height or clear of the ray.
        let mut ordered = Vec::new();
        let mut moved = 0.0_f64;
        let mut stack: Vec<_> = self
            .balanced()
            .into_iter()
            .rev()
            .map(|(c, drifted)| (c, 0_u32, drifted))
            .collect();
        while let Some((controls, depth, drifted)) = stack.pop() {
            let relative: Vec<[f64; 2]> = controls.iter().map(|c| sub(c.point(), point)).collect();
            let (lo, hi) = height_range(&relative);
            let clear = lo >= 0.0 || hi < 0.0 || relative.iter().all(|p| p[0] > allowance);
            let one_way = relative.windows(2).all(|w| w[0][1] <= w[1][1])
                || relative.windows(2).all(|w| w[0][1] >= w[1][1]);
            if clear || one_way {
                // Evaluation rounds like one more halving.
                moved = moved.max(drifted + drift(&controls, true));
                ordered.push((controls, relative));
                continue;
            }
            if depth >= 60 {
                return (f64::NAN, moved);
            }
            let (left, right) = split(&controls);
            let halved = drifted + drift(&controls, true);
            for half in [right, left] {
                let reweighted = standard_form(&half);
                let drifted = halved + drift(&half, false).max(drift(&reweighted, false));
                stack.push((reweighted, depth + 1, drifted));
            }
        }
        let mut turns = 0_i32;
        let mut previous = a;
        for (controls, relative) in &ordered {
            let (start, end) = (relative[0], relative[relative.len() - 1]);
            match straight_crossing(previous, start, allowance) {
                Some(t) => turns += t,
                None => return (f64::NAN, moved),
            }
            match piece_crossing(controls, relative, point, allowance) {
                Some(t) => turns += t,
                None => return (f64::NAN, moved),
            }
            previous = end;
        }
        match straight_crossing(previous, b, allowance) {
            Some(t) => turns += t,
            None => return (f64::NAN, moved),
        }
        let angle = libm::atan2(b[1], b[0]) - libm::atan2(a[1], a[0]) + f64::from(turns) * TAU;
        (angle, moved)
    }
}

/// How far rounding in one halving, reweighting or evaluation of these
/// homogeneous controls may move the curve they describe. Each result
/// control's weighted coordinates and weight carry the rounding of at most
/// `order + 2` operations of `EPSILON` each: its point moves by that times
/// its coordinates' magnitude and the weights' spread, and its weight
/// relatively by that, which moves a rational curve by at most twice as
/// much times its controls' reach. Where reweighting, which scales each
/// control alone, rather than halving or evaluation, which mix them, the
/// spread does not enter. Twice the sum, generously.
fn drift(controls: &[WeightedPoint<2>], mixes: bool) -> f64 {
    let operations = controls.len() as f64 + 2.0;
    let (lightest, heaviest) = controls
        .iter()
        .fold((f64::INFINITY, 0.0_f64), |(lo, hi), c| {
            (lo.min(c.weight), hi.max(c.weight))
        });
    let reach = controls
        .iter()
        .flat_map(WeightedPoint::point)
        .fold(0.0_f64, |m, x| m.max(x.abs()));
    let spread = if mixes { heaviest / lightest } else { 1.0 };
    2.0 * operations * f64::EPSILON * reach * (spread + 1.0 + 4.0)
}

/// A weight evaluation can divide by: positive, normal and finite.
fn valid_weight_range(w: f64) -> bool {
    w.is_finite() && w >= f64::MIN_POSITIVE
}

/// Whether every direction between two controls, which bound the curve's
/// tangents, lies within an eighth of a turn of the chord's.
fn gentle(controls: &[WeightedPoint<2>]) -> bool {
    let points: Vec<[f64; 2]> = controls.iter().map(WeightedPoint::point).collect();
    let chord = sub(points[points.len() - 1], points[0]);
    if chord == [0.0; 2] {
        // A piece no wider than rounding turns nowhere that matters.
        return points.iter().all(|&p| p == points[0]);
    }
    // Directions only: each scaled by its largest coordinate, so their
    // products neither underflow nor overflow.
    let scaled = |v: [f64; 2]| {
        let m = v[0].abs().max(v[1].abs());
        [v[0] / m, v[1] / m]
    };
    let chord = scaled(chord);
    points.iter().enumerate().all(|(i, &p)| {
        points[i + 1..].iter().all(|&q| {
            let d = sub(q, p);
            // Within 45 degrees: `cos >= sin`, so `along >= |across|`.
            d == [0.0; 2] || {
                let d = scaled(d);
                dot(d, chord) >= cross(chord, d).abs()
            }
        })
    })
}

/// Homogeneous controls reparameterized to standard form, their end
/// weights equal, which moves no point of the curve: weight `i` times
/// `lambda^i / w_0` with `lambda = (w_0 / w_n)^(1 / n)`, all then scaled
/// by one power of two.
fn standard_form(controls: &[WeightedPoint<2>]) -> Vec<WeightedPoint<2>> {
    let n = controls.len() - 1;
    let (first, last) = (controls[0].weight, controls[n].weight);
    let lambda = libm::pow(first / last, 1.0 / n as f64);
    let mut scale = 1.0 / first;
    let weights: Vec<f64> = controls
        .iter()
        .map(|c| {
            let w = c.weight * scale;
            scale *= lambda;
            w
        })
        .collect();
    // One power of two for all, which moves no point: none where the
    // weights fit as they are, otherwise one centring the largest weighted
    // coordinate and the smallest weight about one, as evaluation
    // multiplies them. Each control must stay one validation accepts, its
    // nonzero coordinates normal; where no shift keeps them so, the
    // controls are kept as they are.
    let with = |shift: i32| -> Option<Vec<WeightedPoint<2>>> {
        let mut standard = Vec::with_capacity(controls.len());
        for (c, &w) in controls.iter().zip(&weights) {
            let (point, w) = (c.point(), libm::ldexp(w, shift));
            if !(valid_weight_range(w) && representable(&point, w)) {
                return None;
            }
            standard.push(WeightedPoint::new(point, w));
        }
        Some(standard)
    };
    let largest = controls
        .iter()
        .zip(&weights)
        .map(|(c, &w)| w * c.point().iter().fold(1.0_f64, |m, x| m.max(x.abs())))
        .fold(0.0_f64, f64::max);
    let smallest = weights.iter().copied().fold(f64::INFINITY, f64::min);
    with(0)
        .or_else(|| {
            (largest.is_finite() && smallest.is_finite() && smallest > 0.0)
                .then(|| with(-(libm::ilogb(largest) + libm::ilogb(smallest)) / 2))
                .flatten()
        })
        .unwrap_or_else(|| controls.to_vec())
}

/// `integral(f, 0, 1)` by five-point Gauss-Legendre on halves taken
/// until two halves agree with their whole within their share of
/// `2^-44 scale`, or of `noise`, the rounding `f` carries, at most 40
/// halvings deep and 2^14 halves in all.
fn adaptive(f: &mut impl FnMut(f64) -> f64, scale: f64, noise: f64) -> f64 {
    let mut sum = 0.0;
    let mut stack = alloc::vec![(0.0_f64, 1.0_f64, integrate(f, 0.0, 1.0, 1), 0_u32)];
    let mut halves = 0_u32;
    while let Some((a, b, whole, depth)) = stack.pop() {
        let m = 0.5 * (a + b);
        let (left, right) = (integrate(f, a, m, 1), integrate(f, m, b, 1));
        halves += 1;
        let tolerance = (scale * libm::ldexp(1.0, -44)).max(noise) * (b - a);
        if depth >= 40 || halves >= 1 << 14 || ((left + right) - whole).abs() <= tolerance {
            sum += left + right;
            continue;
        }
        stack.push((m, b, right, depth + 1));
        stack.push((a, m, left, depth + 1));
    }
    sum
}

/// The Bernstein coefficients of `g = Q . (Q' W - Q W')` on homogeneous
/// `controls`, where `Q = A - p W`, which is `(C - p) . C'` times `W^3`,
/// and for each the rounding its arithmetic may carry.
fn critical(controls: &[WeightedPoint<2>], point: [f64; 2]) -> (Vec<f64>, Vec<f64>) {
    let n = controls.len() - 1;
    let w: Vec<f64> = controls.iter().map(|c| c.weight).collect();
    let q: [Vec<f64>; 2] = core::array::from_fn(|k| {
        controls
            .iter()
            .map(|c| c.weighted[k] - point[k] * c.weight)
            .collect()
    });
    let derivative =
        |v: &[f64]| -> Vec<f64> { v.windows(2).map(|x| n as f64 * (x[1] - x[0])).collect() };
    let dw = derivative(&w);
    let mut g = alloc::vec![0.0; 3 * n];
    let mut magnitude = alloc::vec![0.0; 3 * n];
    for q in &q {
        let dq = derivative(q);
        let r: Vec<f64> = product(&dq, &w)
            .iter()
            .zip(product(q, &dw))
            .map(|(a, b)| a - b)
            .collect();
        let abs = |v: &[f64]| v.iter().map(|x| x.abs()).collect::<Vec<_>>();
        let r_abs: Vec<f64> = product(&abs(&dq), &abs(&w))
            .iter()
            .zip(product(&abs(q), &abs(&dw)))
            .map(|(a, b)| a + b)
            .collect();
        for (i, (v, m)) in product(q, &r)
            .into_iter()
            .zip(product(&abs(q), &r_abs))
            .enumerate()
        {
            g[i] += v;
            magnitude[i] += m;
        }
    }
    let rounding = magnitude
        .iter()
        .map(|m| 64.0 * (n as f64 + 1.0) * f64::EPSILON * m)
        .collect();
    (g, rounding)
}

/// The Bernstein coefficients of the product of two Bernstein
/// polynomials, of degrees one less than their lengths.
fn product(a: &[f64], b: &[f64]) -> Vec<f64> {
    let (m, r) = (a.len() - 1, b.len() - 1);
    (0..=m + r)
        .map(|k| {
            let lo = k.saturating_sub(r);
            (lo..=k.min(m))
                .map(|i| binomial(m, i) * binomial(r, k - i) / binomial(m + r, k) * a[i] * b[k - i])
                .sum()
        })
        .collect()
}

fn binomial(n: usize, k: usize) -> f64 {
    (0..k).fold(1.0, |c, i| c * (n - i) as f64 / (i + 1) as f64)
}

/// One half of a scalar Bernstein polynomial and its roundings, by de
/// Casteljau's scheme at `1/2`.
fn split_scalar(g: &[f64], rounding: &[f64], left: bool) -> (Vec<f64>, Vec<f64>) {
    let half = |v: &[f64]| {
        let mut row = v.to_vec();
        let mut out = Vec::with_capacity(v.len());
        let n = v.len();
        out.push(if left { row[0] } else { row[n - 1] });
        for width in (1..n).rev() {
            for i in 0..width {
                row[i] = 0.5 * (row[i] + row[i + 1]);
            }
            out.push(if left { row[0] } else { row[width - 1] });
        }
        if !left {
            out.reverse();
        }
        out
    };
    // Averages of roundings bound the halves' roundings, plus their own.
    let r = half(rounding)
        .into_iter()
        .zip(half(&g.iter().map(|v| v.abs()).collect::<Vec<_>>()))
        .map(|(r, m)| r + 4.0 * f64::EPSILON * m)
        .collect();
    (half(g), r)
}

/// A bracket `[a, b]` holding the one root of a Bernstein polynomial whose
/// ends differ in sign, by bisection while the sign at the midpoint is
/// certain: past its `rounding`, evaluated alike.
fn root(g: &[f64], rounding: &[f64]) -> (f64, f64) {
    let value = |v: &[f64], t: f64| {
        let mut row = v.to_vec();
        for width in (1..row.len()).rev() {
            for i in 0..width {
                row[i] = (1.0 - t) * row[i] + t * row[i + 1];
            }
        }
        row[0]
    };
    let negative_start = g[0] < 0.0;
    // The sign at `t`, where certain: whether it is the start's.
    let side = |t: f64| {
        let (v, r) = (value(g, t), value(rounding, t));
        (v.abs() > 2.0 * r).then_some((v < 0.0) == negative_start)
    };
    let (mut a, mut b) = (0.0_f64, 1.0_f64);
    for _ in 0..128 {
        let m = 0.5 * (a + b);
        match side(m) {
            Some(true) => a = m,
            Some(false) => b = m,
            // Uncertain at the middle: close in from either side where the
            // sign is still certain.
            None => {
                let (left, right) = (0.5 * (a + m), 0.5 * (m + b));
                let moved_a = side(left) == Some(true);
                let moved_b = side(right) == Some(false);
                if moved_a {
                    a = left;
                }
                if moved_b {
                    b = right;
                }
                if !moved_a && !moved_b {
                    break;
                }
            }
        }
    }
    (a, b)
}

/// A bound on a piece's speed: `n` times the squared spread of its
/// weights times its controls' widest separation.
fn speed_bound(controls: &[WeightedPoint<2>]) -> f64 {
    let n = (controls.len() - 1) as f64;
    let (lo, hi) = controls
        .iter()
        .fold((f64::INFINITY, 0.0_f64), |(lo, hi), c| {
            (lo.min(c.weight), hi.max(c.weight))
        });
    let points: Vec<[f64; 2]> = controls.iter().map(WeightedPoint::point).collect();
    let widest = points
        .iter()
        .flat_map(|p| points.iter().map(move |q| length(sub(*p, *q))))
        .fold(0.0, f64::max);
    n * (hi / lo) * (hi / lo) * widest
}

/// Whether a point relative to the query lies on the upper side of the
/// ray's height, which holds height zero, as `atan2` puts the ray's own
/// points at a half turn.
fn upper(p: [f64; 2]) -> bool {
    p[1] >= 0.0
}

/// The turns a crossing of the leftward ray adds: one going down from the
/// upper side, where `atan2` jumps from a half turn to minus one, minus
/// one going up.
fn turn(from_upper: bool) -> i32 {
    if from_upper { 1 } else { -1 }
}

/// The least and greatest heights of points.
fn height_range(points: &[[f64; 2]]) -> (f64, f64) {
    points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p[1]), hi.max(p[1]))
        })
}

/// The turns a straight join from `p` to `q`, relative to the query,
/// crosses the leftward ray, or `None` where it crosses within
/// `allowance` of the query.
fn straight_crossing(p: [f64; 2], q: [f64; 2], allowance: f64) -> Option<i32> {
    if upper(p) == upper(q) {
        return Some(0);
    }
    let x = p[0] + (q[0] - p[0]) * (p[1] / (p[1] - q[1]));
    if x.abs() <= allowance || x.is_nan() {
        return None;
    }
    Some(if x < 0.0 { turn(upper(p)) } else { 0 })
}

/// The turns a piece whose heights run one way, or which is clear of the
/// ray, crosses the leftward ray, or `None` where it crosses within
/// `allowance` of the query, at `relative` controls.
fn piece_crossing(
    controls: &[WeightedPoint<2>],
    relative: &[[f64; 2]],
    point: [f64; 2],
    allowance: f64,
) -> Option<i32> {
    let (start, end) = (relative[0], relative[relative.len() - 1]);
    if upper(start) == upper(end) {
        return Some(0);
    }
    let x = if relative.iter().all(|p| p[0] < -allowance) {
        -1.0
    } else if relative.iter().all(|p| p[0] > allowance) {
        1.0
    } else {
        let mut scratch = controls.to_vec();
        let mut at = |t: f64| {
            sub(
                local_evaluation(controls, 1.0, t, &mut scratch).point,
                point,
            )
        };
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        for _ in 0..64 {
            let m = 0.5 * (lo + hi);
            if upper(at(m)) == upper(start) {
                lo = m;
            } else {
                hi = m;
            }
        }
        at(hi)[0]
    };
    if x.abs() <= allowance {
        return None;
    }
    Some(if x < 0.0 { turn(upper(start)) } else { 0 })
}

/// The box of homogeneous controls' points.
fn hull_box(controls: &[WeightedPoint<2>]) -> [[f64; 2]; 2] {
    controls.iter().fold(
        [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]],
        |[lo, hi], c| {
            let p = c.point();
            [
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            ]
        },
    )
}

/// The two halves of homogeneous Bezier controls at `t = 1/2`, by de
/// Casteljau's scheme.
fn split(controls: &[WeightedPoint<2>]) -> (Vec<WeightedPoint<2>>, Vec<WeightedPoint<2>>) {
    let n = controls.len();
    let mut row = controls.to_vec();
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    left.push(row[0]);
    right.push(row[n - 1]);
    for width in (1..n).rev() {
        for i in 0..width {
            row[i] = WeightedPoint::lerp(row[i], row[i + 1], 0.5);
        }
        left.push(row[0]);
        right.push(row[width - 1]);
    }
    right.reverse();
    (left, right)
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::f64::consts::{FRAC_1_SQRT_2, PI};

    use super::*;
    use crate::profile::{
        CornerTreatment, Curve, Loop, LoopError, LoopRef, Profile, ProfileError, ProfilePolicy,
        RegionError, RegionOp, RegionPolicy, Segment, SegmentRef,
    };

    const POLICY: ProfilePolicy = ProfilePolicy::new(1e-9);

    /// A quarter disk of radius `r` about the origin: in along the x axis,
    /// round by a rational quadratic, back down the y axis.
    fn quarter_disk(r: f64) -> Loop {
        Loop::new(vec![
            Segment::line([r, 0.0]),
            Segment::bezier(
                [0.0, r],
                Bezier::new(vec![[r, r]], Some(vec![1.0, FRAC_1_SQRT_2, 1.0])),
            ),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap()
    }

    /// A unit square whose top bows up through a polynomial cubic.
    fn bowed_square() -> Loop {
        Loop::new(vec![
            Segment::line([1.0, 0.0]),
            Segment::line([1.0, 1.0]),
            Segment::bezier(
                [0.0, 1.0],
                Bezier::new(vec![[2.0 / 3.0, 1.5], [1.0 / 3.0, 1.5]], None),
            ),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap()
    }

    fn square(min: [f64; 2], max: [f64; 2]) -> Loop {
        Loop::new(vec![
            Segment::line([max[0], min[1]]),
            Segment::line(max),
            Segment::line([min[0], max[1]]),
            Segment::line(min),
        ])
        .unwrap()
    }

    #[test]
    fn areas_moments_and_lengths_follow_the_curves() {
        for r in [1e-3, 1.0, 1e3] {
            let profile = Profile::new(quarter_disk(r), vec![], &POLICY).unwrap();
            let area = PI / 4.0 * r * r;
            assert!((profile.area() - area).abs() <= 1e-13 * area, "{r}");
            // The quarter disk's centroid is `4 r / (3 pi)` along each axis.
            let moments = profile.first_moments([0.0; 2]).unwrap();
            for m in moments {
                assert!((m - r * r * r / 3.0).abs() <= 1e-13 * r * r * r, "{m}");
            }
            let length = quarter_disk(r).length();
            assert!((length - (2.0 + PI / 2.0) * r).abs() <= 1e-13 * r);
        }
        // The cubic `y = 1 + 3/2 t (1 - t)` adds `1/4` to the unit square.
        let profile = Profile::new(bowed_square(), vec![], &POLICY).unwrap();
        assert!((profile.area() - 1.25).abs() < 1e-15);
    }

    #[test]
    fn bezier_profiles_validate_with_holes_and_wind_as_drawn() {
        let hole = square([0.4, 0.2], [0.6, 0.4]).reversed();
        Profile::new(bowed_square(), vec![hole.clone()], &POLICY).unwrap();
        // A hole above the square, in the bow.
        Profile::new(
            bowed_square(),
            vec![square([0.45, 1.05], [0.55, 1.3]).reversed()],
            &POLICY,
        )
        .unwrap();
        assert_eq!(
            Profile::new(bowed_square().reversed(), vec![], &POLICY),
            Err(ProfileError::WrongWinding {
                loop_ref: LoopRef::Outer
            })
        );
        assert_eq!(
            Profile::new(
                bowed_square(),
                vec![square([2.0, 0.2], [2.2, 0.4]).reversed()],
                &POLICY
            ),
            Err(ProfileError::HoleOutside { hole: 0 })
        );
        // A Bezier hole: the quarter disk, small and reversed.
        let quarter = quarter_disk(0.2).translated([0.3, 0.3]).unwrap().reversed();
        Profile::new(square([0.0; 2], [1.0; 2]), vec![quarter], &POLICY).unwrap();
    }

    #[test]
    fn crossing_touching_and_looping_curves_are_refused() {
        // Over the hole's width the bow, `y = 1 + 3/2 t (1 - t)`, rises
        // from 1.36 to 1.375: it crosses a hole reaching above it, comes
        // within the separation of one reaching nearly to it, and clears
        // one a little lower.
        let hole = |top: f64| vec![square([0.4, 0.8], [0.6, top]).reversed()];
        for top in [1.4, 1.36 - 1e-10] {
            assert!(matches!(
                Profile::new(bowed_square(), hole(top), &POLICY),
                Err(ProfileError::Unresolved { .. })
            ));
        }
        Profile::new(bowed_square(), hole(1.36 - 1e-6), &POLICY).unwrap();
        // A cubic that loops over itself.
        let looping = Loop::new(vec![
            Segment::line([1.0, 0.0]),
            Segment::bezier([0.0, 0.0], Bezier::new(vec![[-1.0, 1.0], [2.0, 1.0]], None)),
        ])
        .unwrap();
        assert!(matches!(
            Profile::new(looping, vec![], &POLICY),
            Err(ProfileError::Unresolved { .. })
        ));
    }

    #[test]
    fn arcs_and_center_arcs_are_proved_beside_bezier_segments() {
        let circle = |center: [f64; 2], radius: f64, centered: bool| {
            let east = [center[0] + radius, center[1]];
            let west = [center[0] - radius, center[1]];
            let arc = |to| {
                if centered {
                    Segment::arc_centered(to, center, true)
                } else {
                    Segment::arc(to, -1.0)
                }
            };
            Loop::new(vec![arc(west), arc(east)]).unwrap()
        };
        for centered in [false, true] {
            Profile::new(
                bowed_square(),
                vec![circle([0.5, 0.5], 0.3, centered)],
                &POLICY,
            )
            .unwrap();
            // Touching the right side.
            assert!(matches!(
                Profile::new(
                    bowed_square(),
                    vec![circle([0.5, 0.5], 0.5, centered)],
                    &POLICY
                ),
                Err(ProfileError::Unresolved { .. })
            ));
        }
    }

    #[test]
    fn invalid_controls_are_refused_by_the_loop() {
        let with = |bezier| {
            Loop::new(vec![
                Segment::line([1.0, 0.0]),
                Segment::bezier([0.0, 0.0], bezier),
            ])
        };
        let invalid = Err(LoopError::InvalidBezier { segment: 1 });
        assert_eq!(with(Bezier::new(vec![[0.5, 1.0]; 15], None)), invalid);
        assert_eq!(
            with(Bezier::new(vec![[0.5, 1.0]], Some(vec![1.0; 2]))),
            invalid
        );
        assert_eq!(
            with(Bezier::new(vec![[0.5, 1.0]], Some(vec![1.0, 0.0, 1.0]))),
            invalid
        );
        assert_eq!(with(Bezier::new(vec![[0.5, f64::NAN]], None)), invalid);
        assert!(with(Bezier::new(vec![[0.5, 1.0]; 14], None)).is_ok());
    }

    #[test]
    fn points_are_classified_around_bezier_segments() {
        let disk = quarter_disk(1.0);
        let inside = disk.query_point([0.5, 0.5]).unwrap();
        assert_eq!(inside.winding, Some(1));
        let outside = disk.query_point([0.8, 0.8]).unwrap();
        assert_eq!(outside.winding, Some(0));
        let on = disk.query_point([FRAC_1_SQRT_2, FRAC_1_SQRT_2]).unwrap();
        assert!(on.boundary_distance <= on.numeric_allowance);
        assert_eq!(on.winding, None);
        let near = disk.query_point([0.7, 0.7]).unwrap();
        assert!((near.boundary_distance - (1.0 - 0.7 * 2.0_f64.sqrt())).abs() < 1e-12);
    }

    #[test]
    fn corners_carry_bezier_segments_and_regions_refuse_them() {
        let profile = Profile::new(bowed_square(), vec![], &POLICY).unwrap();
        let corner = SegmentRef {
            loop_ref: LoopRef::Outer,
            segment: 0,
        };
        let treated = profile
            .with_corners([corner], CornerTreatment::chamfer(0.1), &POLICY)
            .unwrap();
        assert!(
            treated
                .profile
                .outer()
                .segments()
                .iter()
                .any(|s| matches!(s.curve, Curve::Bezier(_)))
        );
        assert!(matches!(
            profile.combine(
                &Profile::new(square([0.0; 2], [0.5; 2]), vec![], &POLICY).unwrap(),
                RegionOp::Union,
                &RegionPolicy::new(POLICY),
            ),
            Err(RegionError::UnsupportedCurve { .. })
        ));
    }
    #[test]
    fn weights_spread_past_f64_and_shapes_below_it_are_refused() {
        // In standard form a weight 2^33 times its ends' would rush the
        // curve faster than `f64` follows; 2^31 is fine.
        let corner = |weight: f64| {
            Loop::new(vec![
                Segment::line([1.0, 0.0]),
                Segment::bezier(
                    [0.0, 1.0],
                    Bezier::new(vec![[1.0, 1.0]], Some(vec![1.0, weight, 1.0])),
                ),
                Segment::line([0.0, 0.0]),
            ])
        };
        assert!(corner(2.0_f64.powi(31)).is_ok());
        assert_eq!(
            corner(2.0_f64.powi(33)),
            Err(LoopError::InvalidBezier { segment: 1 })
        );
        // A bow 2^-45 across at a coordinate near one.
        let s = 2.0_f64.powi(-45);
        assert_eq!(
            Loop::new(vec![
                Segment::line([1.0 + s, 1.0]),
                Segment::bezier(
                    [1.0, 1.0],
                    Bezier::new(vec![[1.0 + 0.5 * s, 1.0 + s]], None)
                ),
            ]),
            Err(LoopError::InvalidBezier { segment: 1 })
        );
    }

    #[test]
    fn heavy_weights_are_integrated_over_balanced_pieces() {
        // A rational quadratic weighted at its corner hugs the corner: the
        // profile is all but the unit square.
        for weight in [1e4, 1e8] {
            let corner = Loop::new(vec![
                Segment::line([1.0, 0.0]),
                Segment::bezier(
                    [0.0, 1.0],
                    Bezier::new(vec![[1.0, 1.0]], Some(vec![1.0, weight, 1.0])),
                ),
                Segment::line([0.0, 0.0]),
            ])
            .unwrap();
            let profile = Profile::new(corner.clone(), vec![], &POLICY).unwrap();
            // Against a dense chord sum and shoelace, in standard form's
            // own parameter, which paces the curve evenly.
            let piece = BezierPiece {
                from: [1.0, 0.0],
                to: [0.0, 1.0],
                bezier: Bezier::new(vec![[1.0, 1.0]], Some(vec![1.0, weight, 1.0])),
            };
            let mut points = vec![[0.0, 0.0]];
            for (leaf, _) in piece.balanced() {
                let mut scratch = leaf.clone();
                points.extend((0..20_000).map(|i| {
                    local_evaluation(&leaf, 1.0, f64::from(i) / 20_000.0, &mut scratch).point
                }));
            }
            points.push([0.0, 1.0]);
            let chords: f64 = points.windows(2).map(|w| length(sub(w[1], w[0]))).sum();
            let shoelace: f64 = points.windows(2).map(|w| 0.5 * cross(w[0], w[1])).sum();
            assert!(
                (corner.length() - chords - 1.0).abs() < 1e-7,
                "{}",
                corner.length()
            );
            assert!(
                (profile.area() - shoelace).abs() < 1e-7,
                "{}",
                profile.area()
            );
            assert!(profile.area() > 0.99 && profile.area() < 1.0);
        }
    }

    #[test]
    fn distances_reach_the_whole_curve() {
        // A straight Bezier as the bottom of a unit square.
        let square = Loop::new(vec![
            Segment::bezier([1.0, 0.0], Bezier::new(vec![], None)),
            Segment::line([1.0, 1.0]),
            Segment::line([0.0, 1.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        for x in [0.25, 0.3, 0.71] {
            let on = square.query_point([x, 0.0]).unwrap();
            assert!(on.boundary_distance <= on.numeric_allowance, "{x}");
            assert_eq!(on.winding, None);
            let above = square.query_point([x, 1e-3]).unwrap();
            assert!((above.boundary_distance - 1e-3).abs() < 1e-12, "{x}");
            assert_eq!(above.winding, Some(1));
        }
    }

    #[test]
    fn segments_sharing_a_vertex_keep_the_separation_away_from_it() {
        // Two sides of a sliver run 5e-7 apart, far from their shared
        // vertex: in contact at a separation of 1e-3, curves or lines.
        let policy = ProfilePolicy::new(1e-3);
        for bezier in [false, true] {
            let first = if bezier {
                Segment::bezier([10.0, 0.0], Bezier::new(vec![], None))
            } else {
                Segment::line([10.0, 0.0])
            };
            let sliver = Loop::new(vec![
                first,
                Segment::line([0.0, 1e-6]),
                Segment::line([0.0, 0.0]),
            ])
            .unwrap();
            assert!(
                matches!(
                    Profile::new(sliver, vec![], &policy),
                    Err(ProfileError::Contact { .. } | ProfileError::Unresolved { .. })
                ),
                "{bezier}"
            );
        }
    }

    #[test]
    fn a_center_arc_crossing_a_neighbour_beside_its_end_is_refused() {
        // A center-defined quarter circle whose stored end sits 2^-50 inside
        // its circle, then a side doubling back along it: the authoritative
        // circle crosses that side 9.3e-10 from the shared end.
        let policy = ProfilePolicy::new(1e-12);
        let outline = Loop::new(vec![
            Segment::arc_centered([0.0, 1.0 - 2.0_f64.powi(-50)], [0.0; 2], false),
            Segment::bezier([0.125, 1.0 + 2.0_f64.powi(-23)], Bezier::new(vec![], None)),
            Segment::line([-1.0, 2.0]),
            Segment::line([-1.0, -1.0]),
            Segment::line([1.0, -1.0]),
            Segment::line([1.0, 0.0]),
        ])
        .unwrap();
        assert!(matches!(
            Profile::new(outline, vec![], &policy),
            Err(ProfileError::Unresolved { .. })
        ));
        // A center-defined arc beside Bezier segments at ordinary corners
        // still validates.
        let rounded = Loop::new(vec![
            Segment::bezier([1.0, 0.0], Bezier::new(vec![], None)),
            Segment::arc_centered([2.0, 1.0], [1.0, 1.0], false),
            Segment::line([0.0, 1.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        Profile::new(rounded, vec![], &policy).unwrap();
        // With the stored end 2^-50 inside, a side leaving it outward
        // crosses the start's circle within 2^-49 of the end: inside the
        // vertex's ball, where the arc's readings may meet, so it validates.
        let near = Loop::new(vec![
            Segment::arc_centered([0.0, 1.0 - 2.0_f64.powi(-50)], [0.0; 2], false),
            Segment::bezier([0.125, 1.125], Bezier::new(vec![], None)),
            Segment::line([-1.0, 2.0]),
            Segment::line([-1.0, -1.0]),
            Segment::line([1.0, -1.0]),
            Segment::line([1.0, 0.0]),
        ])
        .unwrap();
        Profile::new(near, vec![], &policy).unwrap();
    }

    #[test]
    fn corners_far_from_the_origin_are_resolved() {
        // A unit square at x = 1e8, closed by a weighted straight Bezier
        // segment: its corners' radii, from a separation finer than the
        // coordinates' rounding, must still be wide enough for the pieces
        // meeting there to fit.
        let square = Loop::new(vec![
            Segment::line([1e8 + 1.0, 0.0]),
            Segment::line([1e8 + 1.0, 1.0]),
            Segment::line([1e8, 1.0]),
            Segment::bezier([1e8, 0.0], Bezier::new(vec![], Some(vec![1.1, 1.1]))),
        ])
        .unwrap();
        Profile::new(square, vec![], &POLICY).unwrap();
    }

    #[test]
    fn a_nearly_full_center_arc_is_enclosed_as_its_circle() {
        // A unit circle about the origin from (1, 0) nearly all the way
        // round, to 2^-30 below its start, closed by a thin spike out to
        // (1.5, 0). The arc through its stored ends with its rounded sweep
        // strays from the circle by about 7e-7 on the far side; a hole
        // 1e-7 inside the circle there is clear of it, and one 1e-7
        // outside crosses it.
        let policy = ProfilePolicy::new(1e-12);
        let end = [1.0 - 2.0_f64.powi(-50), -(2.0_f64.powi(-30))];
        let outline = || {
            Loop::new(vec![
                Segment::arc_centered(end, [0.0; 2], false),
                Segment::bezier([1.5, 0.0], Bezier::new(vec![], None)),
                Segment::line([1.0, 0.0]),
            ])
            .unwrap()
        };
        let hole = |x: f64| {
            Loop::new(vec![
                Segment::line([-0.5, 0.2]),
                Segment::line([-0.5, -0.2]),
                Segment::line([x, 0.0]),
            ])
            .unwrap()
        };
        Profile::new(outline(), vec![hole(-1.0 + 1e-7)], &policy).unwrap();
        assert!(Profile::new(outline(), vec![hole(-1.0 - 1e-7)], &policy).is_err());
    }

    #[test]
    fn tiny_nested_holes_beside_bezier_segments_are_found() {
        // A unit circle about its center, and clockwise diamonds of radii
        // 4s and s within it, one side of the larger a Bezier segment:
        // products of their coordinates underflow, yet the inner lies in
        // the outer.
        let s = 1e-200;
        let circle = Loop::new(vec![
            Segment::arc_centered([-1.0, 0.0], [0.0; 2], false),
            Segment::arc_centered([1.0, 0.0], [0.0; 2], false),
        ])
        .unwrap();
        let diamond = |r: f64, curved: bool| {
            Loop::new(vec![
                if curved {
                    Segment::bezier([0.0, r], Bezier::new(vec![], None))
                } else {
                    Segment::line([0.0, r])
                },
                Segment::line([r, 0.0]),
                Segment::line([0.0, -r]),
                Segment::line([-r, 0.0]),
            ])
            .unwrap()
        };
        assert_eq!(
            Profile::new(
                circle,
                vec![diamond(4.0 * s, true), diamond(s, false)],
                &ProfilePolicy::new(1e-202)
            ),
            Err(ProfileError::NestedHole { hole: 1, within: 0 })
        );
    }

    #[test]
    fn windings_count_crossings_along_one_chain() {
        // Weights of 0.7 round the start's height in standard form; the
        // ray from (2, 0.1) runs through it. Outside, by the triangle's
        // whole width.
        for reversed in [false, true] {
            let triangle = Loop::new(vec![
                Segment::bezier([1.0, 1.1], Bezier::new(vec![], Some(vec![0.7, 0.7]))),
                Segment::line([0.0, 1.1]),
                Segment::line([0.0, 0.1]),
            ])
            .unwrap();
            let triangle = if reversed {
                triangle.reversed()
            } else {
                triangle
            };
            assert_eq!(triangle.query_point([2.0, 0.1]).unwrap().winding, Some(0));
            let inside = if reversed { -1 } else { 1 };
            assert_eq!(
                triangle.query_point([0.2, 0.5]).unwrap().winding,
                Some(inside)
            );
        }
    }

    #[test]
    fn distances_find_the_nearest_of_several_minima() {
        // A wavy quartic: from (3.1, 20) its nearest point is near t = 0.8.
        let piece = BezierPiece {
            from: [0.0, 0.0],
            to: [4.0, 0.0],
            bezier: Bezier::new(vec![[1.0, 0.5], [2.0, -0.25], [3.0, 0.25]], None),
        };
        let mut nearest = f64::INFINITY;
        for i in 0..=100_000 {
            nearest = nearest.min(length(sub(piece.point_at(f64::from(i) / 1e5), [3.1, 20.0])));
        }
        let found = piece.distance_to([3.1, 20.0]);
        assert!(
            found <= nearest + 1e-9 && found >= nearest - 1e-6,
            "{found} {nearest}"
        );
    }

    #[test]
    fn extreme_weights_keep_queries_exact() {
        // A straight bottom weighted 2^200 at its end: (0.5, 0) lies on it.
        let square = Loop::new(vec![
            Segment::bezier(
                [1.0, 0.0],
                Bezier::new(vec![], Some(vec![1.0, 2.0_f64.powi(200)])),
            ),
            Segment::line([1.0, 1.0]),
            Segment::line([0.0, 1.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        let on = square.query_point([0.5, 0.0]).unwrap();
        assert!(on.boundary_distance <= on.numeric_allowance);
        // A quarter circle reparameterized by weights 1, 2^100 / sqrt 2,
        // 2^200 winds its inside once.
        let disk = Loop::new(vec![
            Segment::line([1.0, 0.0]),
            Segment::bezier(
                [0.0, 1.0],
                Bezier::new(
                    vec![[1.0, 1.0]],
                    Some(vec![
                        1.0,
                        2.0_f64.powi(100) * FRAC_1_SQRT_2,
                        2.0_f64.powi(200),
                    ]),
                ),
            ),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        assert_eq!(disk.query_point([0.7, 0.7]).unwrap().winding, Some(1));
        let profile = Profile::new(disk, vec![], &POLICY).unwrap();
        assert!((profile.area() - PI / 4.0).abs() < 1e-12);
    }

    #[test]
    fn a_curve_returning_near_its_neighbour_is_in_contact() {
        // Back along the bottom, the quartic dips to 0.0019 above it at
        // x = 7.5, far from both shared vertices.
        let policy = ProfilePolicy::new(0.01);
        let wiggle = Loop::new(vec![
            Segment::line([10.0, 0.0]),
            Segment::bezier(
                [0.0, 0.0],
                Bezier::new(
                    vec![[7.5, 1.565], [5.0, -6.246666666666667], [2.5, 14.065]],
                    None,
                ),
            ),
        ])
        .unwrap();
        assert!(matches!(
            Profile::new(wiggle, vec![], &policy),
            Err(ProfileError::Unresolved { .. } | ProfileError::Contact { .. })
        ));
        // An acute corner of two long sides is fine.
        let acute = Loop::new(vec![
            Segment::bezier([10.0, 0.0], Bezier::new(vec![], None)),
            Segment::line([0.0, 5.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        Profile::new(acute, vec![], &policy).unwrap();
    }

    #[test]
    fn steep_curves_are_measured_to_rounding() {
        // `(t, 1000 (t - 0.3)^2)`, whose length is 580.0040068630...
        let piece = BezierPiece {
            from: [0.0, 90.0],
            to: [1.0, 490.0],
            bezier: Bezier::new(vec![[0.5, -210.0]], None),
        };
        let length = piece.length();
        let exact = {
            let f = |u: f64| 0.5 * u * libm::sqrt(1.0 + u * u) + 0.5 * libm::asinh(u);
            (f(1400.0) - f(-600.0)) / 2000.0
        };
        assert!((length - exact).abs() < 1e-9 * exact, "{length} {exact}");
    }
    #[test]
    fn a_point_near_a_stationary_turn_is_measured_to_rounding() {
        // The bottom runs back on itself along y = 0 through the origin.
        let outline = Loop::new(vec![
            Segment::bezier([0.49, 0.0], Bezier::new(vec![[-0.21, 0.0]], None)),
            Segment::line([0.49, 1.0]),
            Segment::line([0.09, 1.0]),
            Segment::line([0.09, 0.0]),
        ])
        .unwrap();
        let query = outline.query_point([0.0, 0.0]).unwrap();
        assert!(
            query.boundary_distance <= query.numeric_allowance,
            "{query:?}"
        );
    }
    #[test]
    fn a_narrow_turn_is_not_missed() {
        // `((t - 0.001)^2, 1e-9 t)`: a turn near its start no fixed node
        // sees; its length is 0.998002.
        let piece = BezierPiece {
            from: [1e-6, 0.0],
            to: [0.998_001, 1e-9],
            bezier: Bezier::new(vec![[-0.000_999, 5e-10]], None),
        };
        assert!(
            (piece.length() - 0.998_002).abs() < 1e-9,
            "{}",
            piece.length()
        );
    }

    #[test]
    fn integrals_do_not_depend_on_where_the_profile_sits() {
        let at = |x: f64| {
            Loop::new(vec![
                Segment::line([x + 1.0, 0.0]),
                Segment::bezier(
                    [x, 1.0],
                    Bezier::new(vec![[x + 1.0, 1.0]], Some(vec![1.0, 4.0, 1.0])),
                ),
                Segment::line([x, 0.0]),
            ])
            .unwrap()
        };
        let near = at(0.0).signed_area();
        assert!((near - 0.962_296_378_633_071).abs() < 1e-12, "{near}");
        for x in [1e3, 1e6, -1e9] {
            assert!(
                (at(x).signed_area() - near).abs() < 1e-9 * x.abs().max(1.0),
                "{x}"
            );
            let moments = Profile::new(at(x), vec![], &ProfilePolicy::new(1e-3))
                .unwrap()
                .first_moments([x, 0.0])
                .unwrap();
            let base = Profile::new(at(0.0), vec![], &POLICY)
                .unwrap()
                .first_moments([0.0; 2])
                .unwrap();
            for (m, b) in moments.iter().zip(base) {
                assert!((m - b).abs() < 1e-6, "{x}: {m} {b}");
            }
        }
    }

    #[test]
    fn a_circle_center_is_queried_promptly() {
        let circle = Loop::new(vec![
            Segment::bezier(
                [0.0, 1.0],
                Bezier::new(vec![[1.0, 1.0]], Some(vec![1.0, FRAC_1_SQRT_2, 1.0])),
            ),
            Segment::bezier(
                [-1.0, 0.0],
                Bezier::new(vec![[-1.0, 1.0]], Some(vec![1.0, FRAC_1_SQRT_2, 1.0])),
            ),
            Segment::bezier(
                [0.0, -1.0],
                Bezier::new(vec![[-1.0, -1.0]], Some(vec![1.0, FRAC_1_SQRT_2, 1.0])),
            ),
            Segment::bezier(
                [1.0, 0.0],
                Bezier::new(vec![[1.0, -1.0]], Some(vec![1.0, FRAC_1_SQRT_2, 1.0])),
            ),
        ])
        .unwrap();
        let query = circle.query_point([0.0, 0.0]).unwrap();
        assert!((query.boundary_distance - 1.0).abs() < 1e-12);
        assert_eq!(query.winding, Some(1));
    }
    #[test]
    fn extreme_scales_and_weights_keep_their_measure() {
        // The narrow turn, 1e-200 the size.
        let s = 1e-200;
        let piece = BezierPiece {
            from: [1e-6 * s, 0.0],
            to: [0.998_001 * s, 1e-9 * s],
            bezier: Bezier::new(vec![[-0.000_999 * s, 5e-10 * s]], None),
        };
        assert!(
            (piece.length() / s - 0.998_002).abs() < 1e-9,
            "{}",
            piece.length() / s
        );
        // A straight segment 2e200 long whose middle weighs 2^512 times its
        // ends.
        let heavy = BezierPiece {
            from: [0.0, 0.0],
            to: [2e200, 0.0],
            bezier: Bezier::new(
                vec![[1e200, 0.0]],
                Some(vec![
                    2.0_f64.powi(-256),
                    2.0_f64.powi(256),
                    2.0_f64.powi(-256),
                ]),
            ),
        };
        assert!(
            (heavy.length() / 2e200 - 1.0).abs() < 1e-12,
            "{}",
            heavy.length()
        );
    }
    #[test]
    fn rescaling_keeps_every_coordinate() {
        // A shallow quadratic 1e100 wide rising 1e-300: its area survives.
        let outline = Loop::new(vec![
            Segment::line([1e100, 0.0]),
            Segment::bezier([0.0, 0.0], Bezier::new(vec![[5e99, 1e-300]], None)),
        ])
        .unwrap();
        let area = outline.signed_area();
        assert!(
            (area / 3.333_333_333_333_333_5e-201 - 1.0).abs() < 1e-12,
            "{area}"
        );
    }
    #[test]
    fn tiny_curves_far_from_zero_are_measured() {
        // A quadratic 1e-163 across, 2e-153 from the origin.
        let (x, s) = (2e-153, 1e-163);
        let outline = Loop::new(vec![
            Segment::bezier(
                [x + 2.0 * s, 0.0],
                Bezier::new(vec![[x + s, 0.5 * s]], None),
            ),
            Segment::bezier([x + 2.0 * s, 2.0 * s], Bezier::new(vec![], None)),
            Segment::bezier([x, 2.0 * s], Bezier::new(vec![], None)),
            Segment::bezier([x, 0.0], Bezier::new(vec![], None)),
        ])
        .unwrap();
        let query = outline.query_point([x + 0.5 * s, -0.1 * s]).unwrap();
        // The curve at t = 1/4 is 0.2875 s away.
        assert!(
            query.boundary_distance <= 0.2875 * s + query.numeric_allowance,
            "{query:?}"
        );
        assert!(query.boundary_distance >= 0.2 * s);
    }
}
