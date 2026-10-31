// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Rational B-splines (NURBS) evaluated by de Boor's algorithm.

use alloc::vec::Vec;

use crate::vector::WeightedPoint;
use crate::{CurveEvaluation, Workspace};

/// One rational B-spline curve in `N` dimensions, borrowed: a plane
/// curve for `N = 2`, a space curve for the default `N = 3`.
///
/// A curve of `points.len()` controls and `order` (degree + 1) has
/// `points.len() + order` nondecreasing knots, none repeated more than
/// `order` times, and is defined over the domain `[knots[order - 1], knots[points.len()]]`. Without weights every
/// control weighs 1 (a polynomial B-spline).
///
/// [`Self::validate`] checks the description once and returns a
/// [`ValidNurbs`], which evaluates, differentiates and samples it, and
/// prepares it ([`ValidNurbs::prepare`]) as a [`PreparedNurbs`] of
/// per-span Bezier controls for repeated evaluation.
///
/// **Discontinuities.** An interior knot repeated `order` times or more
/// ends one polynomial piece and starts the next with no continuity at all
/// (`C^-1`; Piegl and Tiller, *The NURBS Book*, 2nd ed., section 2.2,
/// property P2.5): the curve may jump there. Such a curve is valid; its
/// samples mark each jump ([`NurbsSamples::breaks`]).
#[derive(Clone, Copy, Debug)]
pub struct Nurbs<'a, const N: usize = 3> {
    /// Degree + 1, at least 2.
    pub order: u32,
    /// The knot vector, `points.len() + order` long, nondecreasing, no
    /// value more than `order` times.
    pub knots: &'a [f64],
    /// The controls.
    pub points: &'a [[f64; N]],
    /// One positive weight per control, or `None` for all 1.
    pub weights: Option<&'a [f64]>,
}

/// Why a [`Nurbs`] cannot be evaluated, or a range of it sampled.
///
/// Non-exhaustive: a later check may add a variant without breaking a
/// caller that reports the error through `Display` or a fallback arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NurbsError {
    /// The order is below 2.
    Order {
        /// The order.
        order: u32,
    },
    /// Fewer controls than the order.
    TooFewPoints {
        /// The controls.
        points: usize,
        /// The order.
        order: u32,
    },
    /// The knot vector is not `points + order` long.
    KnotCount {
        /// The knots given.
        knots: usize,
        /// The knots needed, or `usize::MAX` when that many cannot be
        /// counted.
        expected: usize,
    },
    /// A knot is smaller than the one before it, or not finite, or the
    /// last of more than `order` equal knots, where a control would shape
    /// no part of the curve, or the whole knot vector spans more than an
    /// `f64` holds (then `index` is the last knot's).
    Knots {
        /// The knot's index.
        index: usize,
    },
    /// The weights are not one per control.
    WeightCount {
        /// The weights given.
        weights: usize,
        /// The controls.
        points: usize,
    },
    /// A weight is outside `[2^-256, 2^256]`. Weights matter only
    /// relative to each other, so any curve whose weights span less than
    /// that range can be scaled into it.
    Weight {
        /// The weight's index.
        index: usize,
    },
    /// A control has a coordinate that is not finite, or that overflows
    /// when multiplied by its weight, or that its weight takes below the
    /// smallest normal `f64`, where it would lose the digits projection
    /// then magnifies.
    Point {
        /// The control's index.
        index: usize,
    },
    /// The domain is empty: its first and last knots are equal.
    EmptyDomain,
    /// A sampled range is NaN, or meets the domain in a single point or
    /// not at all.
    EmptyRange,
}

impl core::fmt::Display for NurbsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Order { order } => write!(f, "order {order} is below 2"),
            Self::TooFewPoints { points, order } => {
                write!(f, "{points} controls are fewer than the order {order}")
            }
            Self::KnotCount { knots, expected } => {
                write!(f, "{knots} knots where {expected} are needed")
            }
            Self::Knots { index } => write!(
                f,
                "knot {index} decreases, repeats past the order, or is not finite"
            ),
            Self::WeightCount { weights, points } => {
                write!(f, "{weights} weights for {points} controls")
            }
            Self::Weight { index } => write!(f, "weight {index} is outside [2^-256, 2^256]"),
            Self::Point { index } => {
                write!(
                    f,
                    "control {index} is not finite, or overflows or underflows with its weight"
                )
            }
            Self::EmptyDomain => f.write_str("the domain is empty"),
            Self::EmptyRange => f.write_str("the range does not meet the domain in an interval"),
        }
    }
}

impl core::error::Error for NurbsError {}

/// A curve sampled: each sample's parameter and point, and where the
/// curve jumps.
///
/// Non-exhaustive, so it can carry more per-sample data later; a caller
/// reuses one through [`Default`] and the `_into` functions.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct NurbsSamples<const N: usize = 3> {
    /// The parameters, nondecreasing: a parameter repeats only at a
    /// [break](Self::breaks).
    pub params: Vec<f64>,
    /// The points, one per parameter.
    pub points: Vec<[f64; N]>,
    /// The knot spans sampled: nonempty knot intervals meeting the range.
    pub spans: u32,
    /// The indices of the samples that start a new piece, increasing:
    /// before each, an interior knot of multiplicity `order` or more ends
    /// the previous piece, and the sample before it is that piece's end
    /// (its limit from the left, at the same parameter). Empty when the
    /// curve has no such knot inside the range.
    ///
    /// The break is structural: the pieces' ends coincide when the controls
    /// were authored to meet. A caller drawing one connected path splits
    /// it here, or joins pieces whose ends coincide.
    pub breaks: Vec<usize>,
}

/// The most homogeneous points of scratch kept on the stack when no
/// [`Workspace`] is given: two buffers of order 8. Larger needs use a heap
/// buffer per call.
const INLINE_SCRATCH: usize = 16;

/// The smallest weight a curve takes: every convex combination of weights
/// at least this large rounds to a normal number at least about as large,
/// so the weight a point is divided by never underflows to zero.
const MIN_WEIGHT: f64 = f64::from_bits(0x2FF0_0000_0000_0000); // 2^-256

/// The largest weight a curve takes, the reciprocal of [`MIN_WEIGHT`].
const MAX_WEIGHT: f64 = f64::from_bits(0x4FF0_0000_0000_0000); // 2^256

/// Runs `f` on a zeroed scratch buffer of `len` homogeneous points, on the
/// stack up to [`INLINE_SCRATCH`].
pub(crate) fn with_scratch<const N: usize, R>(
    len: usize,
    f: impl FnOnce(&mut [WeightedPoint<N>]) -> R,
) -> R {
    if len <= INLINE_SCRATCH {
        let mut inline = [WeightedPoint::ZERO; INLINE_SCRATCH];
        f(&mut inline[..len])
    } else {
        f(&mut alloc::vec![WeightedPoint::ZERO; len])
    }
}

impl<'a, const N: usize> Nurbs<'a, N> {
    /// Checks the order, knots, weights and controls, and returns the
    /// curve as a [`ValidNurbs`] that evaluates and samples it; what that
    /// guarantees is listed there. One pass over the inputs, here rather
    /// than at every evaluation.
    ///
    /// # Errors
    ///
    /// The first [`NurbsError`] found, in the order of its variants.
    pub fn validate(&self) -> Result<ValidNurbs<'a, N>, NurbsError> {
        let order = self.order;
        if order < 2 {
            return Err(NurbsError::Order { order });
        }
        let n = self.points.len();
        if n < order as usize {
            return Err(NurbsError::TooFewPoints { points: n, order });
        }
        // Zero-dimensional controls occupy no memory, so `n` can be near
        // `usize::MAX`; a knot count no slice can reach saturates.
        let expected = n.saturating_add(order as usize);
        if self.knots.len() != expected || expected == usize::MAX {
            return Err(NurbsError::KnotCount {
                knots: self.knots.len(),
                expected,
            });
        }
        let span = order as usize;
        for (index, knot) in self.knots.iter().enumerate() {
            if !knot.is_finite()
                || (index > 0 && *knot < self.knots[index - 1])
                || (index >= span && *knot == self.knots[index - span])
            {
                return Err(NurbsError::Knots { index });
            }
        }
        // Every knot difference, and so every de Boor ratio's denominator,
        // is at most the whole vector's span.
        let last = expected - 1;
        if !(self.knots[last] - self.knots[0]).is_finite() {
            return Err(NurbsError::Knots { index: last });
        }
        if let Some(weights) = self.weights {
            if weights.len() != n {
                return Err(NurbsError::WeightCount {
                    weights: weights.len(),
                    points: n,
                });
            }
            if let Some(index) = weights
                .iter()
                .position(|w| !(MIN_WEIGHT..=MAX_WEIGHT).contains(w))
            {
                return Err(NurbsError::Weight { index });
            }
        }
        // The weighted controls de Boor's scheme combines; without
        // weights, the coordinates themselves.
        let mut weights = self.weights.unwrap_or(&[]).iter();
        for (index, point) in self.points.iter().enumerate() {
            let w = weights.next().copied().unwrap_or(1.0);
            let representable = |c: f64| {
                let weighted = (w * c).abs();
                weighted.is_finite()
                    && (weighted >= f64::MIN_POSITIVE || c.abs() < f64::MIN_POSITIVE)
            };
            if !point.iter().all(|&c| representable(c)) {
                return Err(NurbsError::Point { index });
            }
        }
        let curve = ValidNurbs { curve: *self };
        let [start, end] = curve.domain();
        if start >= end {
            return Err(NurbsError::EmptyDomain);
        }
        Ok(curve)
    }
}

/// A [`Nurbs`] that [`Nurbs::validate`] accepted: evaluating or sampling
/// it repeats no checks and cannot index out of bounds.
///
/// It guarantees:
/// - an order of at least 2, at least `order` controls, and
///   `points + order` knots;
/// - finite, nondecreasing knots, none repeated more than `order` times,
///   so every control shapes some span, whose whole span (last minus
///   first) is finite, and a domain of positive length;
/// - weights, when given, one per control, in `[2^-256, 2^256]`;
/// - finite weighted controls `w P`, none of whose normal coordinates
///   its weight takes below the smallest normal `f64`.
///
/// De Boor's scheme and blossoming then combine finite values with ratios
/// in `[0, 1]`, so the homogeneous results are finite, and the weight an
/// evaluated point is divided by is a convex combination of the weights,
/// which no rounding takes below `2^-256` by more than a few units in the
/// last place. A point is then a convex combination of the controls, so it
/// is finite too.
#[derive(Clone, Copy, Debug)]
pub struct ValidNurbs<'a, const N: usize = 3> {
    curve: Nurbs<'a, N>,
}

impl<'a, const N: usize> ValidNurbs<'a, N> {
    /// The curve's description.
    #[must_use]
    pub fn curve(&self) -> Nurbs<'a, N> {
        self.curve
    }

    /// The parameter domain: `[knots[order - 1], knots[points.len()]]`, of
    /// positive length.
    #[must_use]
    pub fn domain(&self) -> [f64; 2] {
        let p = self.curve.order as usize - 1;
        [
            self.curve.knots[p],
            self.curve.knots[self.curve.points.len()],
        ]
    }

    /// The curve's point at `t`, clamped to the domain (NaN taken as the
    /// domain's start).
    ///
    /// The knot span is found by binary search: the last nonempty
    /// `[knots[k], knots[k + 1])` with `knots[k] <= t`, so at an interior
    /// knot the curve is evaluated from the right (the piece the knot
    /// starts), and at the domain's end in the last nonempty span. A
    /// sampled curve's left limits at its breaks are in
    /// [`NurbsSamples`], and any span's ends are reachable through
    /// [`PreparedNurbs::evaluation_in`]. The point is de Boor's algorithm
    /// over homogeneous controls `(w x, w y, w z, w)`, divided by the
    /// weight (Piegl and Tiller, *The NURBS Book*, 2nd ed., algorithms A2.1
    /// and A4.1, with de Boor's triangular scheme, section 3.3). No
    /// allocation up to order 8.
    #[must_use]
    pub fn evaluate(&self, t: f64) -> [f64; N] {
        let t = self.clamp(t);
        let k = self.span_at(t);
        with_scratch(self.curve.order as usize, |d| self.de_boor(k, t, d))
    }

    /// The curve's point and its first two derivatives with respect to `t`
    /// at `t`, clamped and in the span chosen as by [`Self::evaluate`].
    ///
    /// The span's polynomial piece is converted to its Bezier form by
    /// blossoming (each Bezier control is the piece's blossom at the span's
    /// ends, evaluated by de Boor's scheme with one parameter per level;
    /// Piegl and Tiller, section 5.4, and Ramshaw, "Blossoms are polar
    /// forms", 1989), whose de Casteljau scheme gives the homogeneous point
    /// and derivatives. The rational point and derivatives follow by
    /// Piegl and Tiller's algorithm A4.2 (equation 4.8):
    /// `C = A / w`, `C' = (A' - w' C) / w`,
    /// `C'' = (A'' - 2 w' C' - w'' C) / w`. The point agrees with
    /// [`Self::evaluate`] to rounding, not bit for bit. No allocation up to
    /// order 8.
    #[must_use]
    pub fn evaluation(&self, t: f64) -> CurveEvaluation<N> {
        let t = self.clamp(t);
        let k = self.span_at(t);
        let knots = self.curve.knots;
        let (a, b) = (knots[k], knots[k + 1]);
        let order = self.curve.order as usize;
        with_scratch(2 * order, |buffer| {
            let (bezier, scratch) = buffer.split_at_mut(order);
            self.bezier_span(k, bezier, scratch);
            span_evaluation(bezier, a, b, t, scratch)
        })
    }

    /// `t` clamped to the domain, NaN to its start. `max` then `min`
    /// rather than `clamp`, whose NaN assertion would format floats into
    /// every caller's binary.
    fn clamp(&self, t: f64) -> f64 {
        let [start, end] = self.domain();
        t.max(start).min(end)
    }

    /// The last nonempty knot span `k` in `[order - 1, points - 1]` with
    /// `knots[k] <= t`, for `t` in the domain.
    fn span_at(&self, t: f64) -> usize {
        let p = self.curve.order as usize - 1;
        let n = self.curve.points.len();
        let knots = self.curve.knots;
        // Knots `p + 1 .. n` at or before `t`: the span starts at the last.
        let mut k = p + knots[p + 1..n].partition_point(|&knot| knot <= t);
        // Only at the domain's end can that span be empty (a clamped end's
        // repeated knots); step back to the last nonempty one, which
        // exists since the domain is not empty.
        while knots[k] == knots[k + 1] {
            k -= 1;
        }
        k
    }

    /// Loads span `k`'s `order` weighted controls into `d`.
    pub(crate) fn load(&self, k: usize, d: &mut [WeightedPoint<N>]) {
        let first = k + 1 - self.curve.order as usize;
        for (j, slot) in d.iter_mut().enumerate() {
            let index = first + j;
            let w = self.curve.weights.map_or(1.0, |weights| weights[index]);
            *slot = WeightedPoint::new(self.curve.points[index], w);
        }
    }

    /// De Boor's triangular scheme over `d`, loaded with span `k`'s
    /// controls, with parameter `t(r)` at level `r`: the piece's point at
    /// `t` when every level takes `t`, its blossom otherwise. Returns the
    /// weighted result.
    pub(crate) fn de_boor_levels(
        &self,
        k: usize,
        d: &mut [WeightedPoint<N>],
        t: impl Fn(usize) -> f64,
    ) -> WeightedPoint<N> {
        let p = self.curve.order as usize - 1;
        let knots = self.curve.knots;
        for r in 1..=p {
            let t = t(r);
            for j in (r..=p).rev() {
                // Nonzero for a nonempty span: `left <= knots[k]` and
                // `right >= knots[k + 1]`.
                let left = knots[j + k - p];
                let right = knots[j + 1 + k - r];
                let alpha = (t - left) / (right - left);
                d[j] = WeightedPoint::lerp(d[j - 1], d[j], alpha);
            }
        }
        d[p]
    }

    /// The point at `t` of the polynomial piece of knot span `k`, with `d`
    /// of length `order` as scratch.
    fn de_boor(&self, k: usize, t: f64, d: &mut [WeightedPoint<N>]) -> [f64; N] {
        self.load(k, d);
        self.de_boor_levels(k, d, |_| t).point()
    }

    /// The weighted Bezier controls of knot span `k`'s piece into `out`:
    /// control `i` is the blossom at `p - i` copies of the span's start and
    /// `i` of its end. `scratch` and `out` are `order` long.
    fn bezier_span(
        &self,
        k: usize,
        out: &mut [WeightedPoint<N>],
        scratch: &mut [WeightedPoint<N>],
    ) {
        let p = self.curve.order as usize - 1;
        let (a, b) = (self.curve.knots[k], self.curve.knots[k + 1]);
        for (i, slot) in out.iter_mut().enumerate() {
            self.load(k, scratch);
            *slot = self.de_boor_levels(k, scratch, |r| if r <= p - i { a } else { b });
        }
    }

    /// Samples the curve over `range` (clamped to the domain; the whole
    /// domain when `None`) `per_span` times per knot span: each nonempty
    /// knot interval meeting the range, clipped to it to `[a, b]`, is
    /// sampled at `a + (b - a) * k / per_span` for `k` in `0..per_span`,
    /// each in that span's piece, and the range's end is sampled last, in
    /// the last span. `per_span` of 0 is taken as 1.
    ///
    /// Where an interior knot of multiplicity `order` or more ends a piece
    /// inside the range, the piece's end is sampled too, before the next
    /// piece's start at the same parameter, and the next piece's first
    /// sample is a [break](NurbsSamples::breaks). Each sample is evaluated
    /// in the span the loop is in, with no search.
    ///
    /// # Errors
    ///
    /// [`NurbsError::EmptyRange`] when the range is NaN or meets the domain
    /// in a single point or not at all.
    pub fn sample(
        &self,
        range: Option<[f64; 2]>,
        per_span: u32,
    ) -> Result<NurbsSamples<N>, NurbsError> {
        let mut out = NurbsSamples::default();
        self.sample_with(range, per_span, &mut Vec::new(), &mut out)?;
        Ok(out)
    }

    /// [`Self::sample`] into a caller's samples, whose buffers are cleared
    /// first, with `workspace` as scratch.
    ///
    /// # Errors
    ///
    /// As [`Self::sample`]; `out` is left empty.
    pub fn sample_into(
        &self,
        range: Option<[f64; 2]>,
        per_span: u32,
        workspace: &mut Workspace<N>,
        out: &mut NurbsSamples<N>,
    ) -> Result<(), NurbsError> {
        self.sample_with(range, per_span, &mut workspace.homogeneous, out)
    }

    fn sample_with(
        &self,
        range: Option<[f64; 2]>,
        per_span: u32,
        scratch: &mut Vec<WeightedPoint<N>>,
        out: &mut NurbsSamples<N>,
    ) -> Result<(), NurbsError> {
        out.params.clear();
        out.points.clear();
        out.breaks.clear();
        out.spans = 0;
        let per_span = per_span.max(1);
        let [start, end] = self.domain();
        let [from, to] = match range {
            None => [start, end],
            Some([a, b]) if a.is_nan() || b.is_nan() => return Err(NurbsError::EmptyRange),
            Some([a, b]) => [a.max(start), b.min(end)],
        };
        if from >= to {
            return Err(NurbsError::EmptyRange);
        }
        let curve = &self.curve;
        let order = curve.order as usize;
        let p = order - 1;
        scratch.clear();
        scratch.resize(order, WeightedPoint::ZERO);
        let mut previous: Option<usize> = None;
        for i in p..curve.points.len() {
            let (a, b) = (curve.knots[i].max(from), curve.knots[i + 1].min(to));
            if a >= b {
                continue;
            }
            // The knots from the previous span's end to this one's start
            // are equal: as many as their multiplicity.
            if let Some(last) = previous
                && i - last >= order
            {
                out.params.push(a);
                out.points.push(self.de_boor(last, a, scratch));
                out.breaks.push(out.params.len());
            }
            out.spans += 1;
            for k in 0..per_span {
                let t = a + (b - a) * (f64::from(k) / f64::from(per_span));
                out.params.push(t);
                out.points.push(self.de_boor(i, t, scratch));
            }
            previous = Some(i);
        }
        // The range is an interval of the domain, so a span was sampled.
        let last = previous.unwrap_or(p);
        out.params.push(to);
        out.points.push(self.de_boor(last, to, scratch));
        Ok(())
    }

    /// The curve's nonempty knot spans in Bezier form, for repeated
    /// evaluation: see [`PreparedNurbs`].
    #[must_use]
    pub fn prepare(&self) -> PreparedNurbs<N> {
        let mut out = PreparedNurbs {
            order: 0,
            bounds: Vec::new(),
            controls: Vec::new(),
            breaks: Vec::new(),
        };
        self.prepare_into(&mut Workspace::new(), &mut out);
        out
    }

    /// [`Self::prepare`] into an existing [`PreparedNurbs`], reusing its
    /// buffers, with `workspace` as scratch.
    pub fn prepare_into(&self, workspace: &mut Workspace<N>, out: &mut PreparedNurbs<N>) {
        let curve = &self.curve;
        let order = curve.order as usize;
        let p = order - 1;
        out.order = order;
        out.bounds.clear();
        out.controls.clear();
        out.breaks.clear();
        let scratch = &mut workspace.homogeneous;
        scratch.clear();
        scratch.resize(order, WeightedPoint::ZERO);
        let mut previous: Option<usize> = None;
        for k in p..curve.points.len() {
            if curve.knots[k] >= curve.knots[k + 1] {
                continue;
            }
            if let Some(last) = previous
                && k - last >= order
            {
                out.breaks.push(out.bounds.len());
            }
            out.bounds.push(curve.knots[k]);
            let first = out.controls.len();
            out.controls.resize(first + order, WeightedPoint::ZERO);
            self.bezier_span(k, &mut out.controls[first..], scratch);
            previous = Some(k);
        }
        // The domain is not empty, so a span was prepared; its end is the
        // domain's.
        out.bounds.push(self.domain()[1]);
    }
}

/// A rational B-spline curve that owns its knots, controls and weights,
/// checked once when made.
///
/// [`Nurbs`] borrows a caller's buffers, so a caller rebuilding curves
/// from its own storage allocates nothing; this type is for curves kept as
/// values, which are cloned, compared and stored, as a modelling kernel
/// keeps them. [`Self::view`] lends it as the [`ValidNurbs`] that
/// evaluates, samples and prepares it, with no checks repeated.
#[derive(Clone, Debug, PartialEq)]
pub struct NurbsCurve<const N: usize = 3> {
    pub(crate) order: u32,
    pub(crate) knots: Vec<f64>,
    pub(crate) points: Vec<[f64; N]>,
    pub(crate) weights: Option<Vec<f64>>,
}

impl<const N: usize> NurbsCurve<N> {
    /// A curve of `order` (degree + 1), checked as [`Nurbs::validate`]
    /// checks it.
    ///
    /// # Errors
    ///
    /// The first [`NurbsError`] found.
    pub fn new(
        order: u32,
        knots: Vec<f64>,
        points: Vec<[f64; N]>,
        weights: Option<Vec<f64>>,
    ) -> Result<Self, NurbsError> {
        Nurbs {
            order,
            knots: &knots,
            points: &points,
            weights: weights.as_deref(),
        }
        .validate()?;
        Ok(Self {
            order,
            knots,
            points,
            weights,
        })
    }

    /// The curve as the [`ValidNurbs`] that evaluates, samples and
    /// prepares it.
    #[must_use]
    pub fn view(&self) -> ValidNurbs<'_, N> {
        ValidNurbs {
            curve: Nurbs {
                order: self.order,
                knots: &self.knots,
                points: &self.points,
                weights: self.weights.as_deref(),
            },
        }
    }

    /// Degree + 1.
    #[must_use]
    pub fn order(&self) -> u32 {
        self.order
    }

    /// The knot vector, `points + order` long.
    #[must_use]
    pub fn knots(&self) -> &[f64] {
        &self.knots
    }

    /// The controls.
    #[must_use]
    pub fn points(&self) -> &[[f64; N]] {
        &self.points
    }

    /// One weight per control, or `None` when every weight is 1.
    #[must_use]
    pub fn weights(&self) -> Option<&[f64]> {
        self.weights.as_deref()
    }

    /// The parameter domain, as [`ValidNurbs::domain`].
    #[must_use]
    pub fn domain(&self) -> [f64; 2] {
        self.view().domain()
    }
}

impl<const N: usize> ValidNurbs<'_, N> {
    /// The curve copied into a [`NurbsCurve`] of its own, with no checks
    /// repeated.
    #[must_use]
    pub fn to_curve(&self) -> NurbsCurve<N> {
        NurbsCurve {
            order: self.curve.order,
            knots: self.curve.knots.to_vec(),
            points: self.curve.points.to_vec(),
            weights: self.curve.weights.map(<[f64]>::to_vec),
        }
    }
}

/// The point and derivatives with respect to `t` at `t` of the rational
/// Bezier piece with weighted `controls` over `[a, b]`, with `scratch` as
/// long as `controls`.
fn span_evaluation<const N: usize>(
    controls: &[WeightedPoint<N>],
    a: f64,
    b: f64,
    t: f64,
    scratch: &mut [WeightedPoint<N>],
) -> CurveEvaluation<N> {
    let u = (t - a) / (b - a);
    let [h, d1, d2] = de_casteljau(controls, u, scratch);
    // Derivatives with respect to `u`, then to `t = a + (b - a) u`:
    // divided by the width rather than multiplied by its reciprocal, which
    // overflows for a span narrower than `1 / f64::MAX`.
    let width = b - a;
    rational(h, d1.divided(width), d2.divided(width).divided(width))
}

/// The weighted point, first and second derivatives with respect to `u`
/// of the Bezier curve of `controls` (degree `p = len - 1`) at `u`: de
/// Casteljau's scheme down to three points `R0..R2`, whose second
/// difference is `p (p - 1)` times the second derivative, then to two
/// `S0, S1`, whose difference is `p` times the first (Piegl and Tiller,
/// section 1.4, equations 1.9 and 1.10).
fn de_casteljau<const N: usize>(
    controls: &[WeightedPoint<N>],
    u: f64,
    scratch: &mut [WeightedPoint<N>],
) -> [WeightedPoint<N>; 3] {
    let p = controls.len() - 1;
    scratch.copy_from_slice(controls);
    let mut count = p + 1;
    while count > 3 {
        for j in 0..count - 1 {
            scratch[j] = WeightedPoint::lerp(scratch[j], scratch[j + 1], u);
        }
        count -= 1;
    }
    let pf = p as f64;
    let (s0, s1, second) = if count == 3 {
        let [r0, r1, r2] = [scratch[0], scratch[1], scratch[2]];
        let second = WeightedPoint {
            weighted: core::array::from_fn(|c| {
                pf * (pf - 1.0) * (r2.weighted[c] - 2.0 * r1.weighted[c] + r0.weighted[c])
            }),
            weight: pf * (pf - 1.0) * (r2.weight - 2.0 * r1.weight + r0.weight),
        };
        (
            WeightedPoint::lerp(r0, r1, u),
            WeightedPoint::lerp(r1, r2, u),
            second,
        )
    } else {
        (scratch[0], scratch[1], WeightedPoint::ZERO)
    };
    let first = WeightedPoint::times(WeightedPoint::minus(s1, s0), pf);
    [WeightedPoint::lerp(s0, s1, u), first, second]
}

/// The rational point and derivatives from weighted ones (Piegl and
/// Tiller, algorithm A4.2).
fn rational<const N: usize>(
    h: WeightedPoint<N>,
    d1: WeightedPoint<N>,
    d2: WeightedPoint<N>,
) -> CurveEvaluation<N> {
    let w = h.weight;
    let point = h.point();
    let first: [f64; N] = core::array::from_fn(|c| (d1.weighted[c] - d1.weight * point[c]) / w);
    let second = core::array::from_fn(|c| {
        (d2.weighted[c] - 2.0 * d1.weight * first[c] - d2.weight * point[c]) / w
    });
    CurveEvaluation {
        point,
        first,
        second,
    }
}

/// A validated curve's nonempty knot spans, each as the weighted Bezier
/// controls of its polynomial piece, for repeated evaluation
/// without knot arithmetic.
///
/// Preparing costs `O(order^3)` per span once; evaluating then costs one
/// binary search over the spans and one de Casteljau scheme of `order`
/// points (`O(order^2)`), with derivatives from the same scheme. The
/// controls are contiguous, `order` per span, and a span's own parameter
/// interval is kept, so a renderer can take the Bezier pieces as they are.
/// Results agree with [`ValidNurbs::evaluate`] to rounding, not bit for
/// bit.
///
/// The breaks of a discontinuous curve are listed ([`Self::breaks`]), and
/// [`Self::evaluation_in`] evaluates a named span up to either end, so
/// both sides of a break are reachable.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedNurbs<const N: usize = 3> {
    order: usize,
    /// Each span's start, then the domain's end: `spans + 1` values.
    bounds: Vec<f64>,
    /// `order` weighted controls per span.
    controls: Vec<WeightedPoint<N>>,
    /// Spans that start a new piece.
    breaks: Vec<usize>,
}

impl<const N: usize> PreparedNurbs<N> {
    /// The curve's order, degree + 1.
    #[must_use]
    pub fn order(&self) -> usize {
        self.order
    }

    /// The parameter domain.
    #[must_use]
    pub fn domain(&self) -> [f64; 2] {
        [self.bounds[0], self.bounds[self.bounds.len() - 1]]
    }

    /// How many nonempty knot spans there are, at least one.
    #[must_use]
    pub fn spans(&self) -> usize {
        self.bounds.len() - 1
    }

    /// Span `span`'s parameter interval, or `None` past the last span.
    #[must_use]
    pub fn span_domain(&self, span: usize) -> Option<[f64; 2]> {
        (span < self.spans()).then(|| [self.bounds[span], self.bounds[span + 1]])
    }

    /// Span `span`'s weighted Bezier controls,
    /// [`Self::order`] of them, over [`Self::span_domain`]; `None` past the
    /// last span.
    #[must_use]
    pub fn span_controls(&self, span: usize) -> Option<&[WeightedPoint<N>]> {
        (span < self.spans()).then(|| &self.controls[span * self.order..(span + 1) * self.order])
    }

    /// Every span's weighted Bezier controls, contiguous: [`Self::order`]
    /// per span, in span order, as [`Self::span_controls`] gives them one
    /// span at a time. With [`Self::bounds`] this is the whole prepared
    /// curve as plain arrays, ready to copy to another evaluator.
    #[must_use]
    pub fn controls(&self) -> &[WeightedPoint<N>] {
        &self.controls
    }

    /// Each span's start, then the domain's end: [`Self::spans`] + 1
    /// increasing parameters.
    #[must_use]
    pub fn bounds(&self) -> &[f64] {
        &self.bounds
    }

    /// The spans that start a new piece, increasing: an interior knot of
    /// multiplicity `order` or more ends the span before each, and the
    /// curve may jump there.
    #[must_use]
    pub fn breaks(&self) -> &[usize] {
        &self.breaks
    }

    /// The span holding `t`, clamped to the domain (NaN to its start): as
    /// for [`ValidNurbs::evaluate`], the span a shared bound starts, and
    /// the last span at the domain's end.
    #[must_use]
    pub fn span_at(&self, t: f64) -> usize {
        let t = self.clamp(t);
        self.bounds[1..self.spans()].partition_point(|&bound| bound <= t)
    }

    /// The curve's point at `t` ([`Self::evaluation`]'s point).
    #[must_use]
    pub fn evaluate(&self, t: f64) -> [f64; N] {
        self.evaluation(t).point
    }

    /// The curve's point and first two derivatives with respect to `t` at
    /// `t`, clamped, in [`Self::span_at`]'s span: de Casteljau's scheme on
    /// the span's Bezier controls, then Piegl and Tiller's algorithm A4.2
    /// for the rational point and derivatives. No allocation up to order 8.
    #[must_use]
    pub fn evaluation(&self, t: f64) -> CurveEvaluation<N> {
        let t = self.clamp(t);
        self.in_span(self.span_at(t), t)
    }

    /// As [`Self::evaluation`] in span `span`, with `t` clamped to that
    /// span's interval rather than the domain: at a span's ends this is
    /// the piece's own limit, so both sides of a break, and one-sided
    /// derivatives at a corner, are reachable. `None` past the last span.
    #[must_use]
    pub fn evaluation_in(&self, span: usize, t: f64) -> Option<CurveEvaluation<N>> {
        let [a, b] = self.span_domain(span)?;
        Some(self.in_span(span, t.max(a).min(b)))
    }

    fn clamp(&self, t: f64) -> f64 {
        let [start, end] = self.domain();
        t.max(start).min(end)
    }

    fn in_span(&self, span: usize, t: f64) -> CurveEvaluation<N> {
        with_scratch(self.order, |scratch| self.in_span_with(span, t, scratch))
    }

    /// As [`Self::evaluation_in`], with `t` already in span `span` and
    /// `scratch`, `order` long, for de Casteljau's scheme.
    pub(crate) fn in_span_with(
        &self,
        span: usize,
        t: f64,
        scratch: &mut [WeightedPoint<N>],
    ) -> CurveEvaluation<N> {
        let (a, b) = (self.bounds[span], self.bounds[span + 1]);
        let first = span * self.order;
        let controls = &self.controls[first..first + self.order];
        span_evaluation(controls, a, b, t, scratch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALF_SQRT2: f64 = core::f64::consts::FRAC_1_SQRT_2;

    fn quarter() -> ([[f64; 3]; 3], [f64; 6], [f64; 3]) {
        (
            [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            [1.0, HALF_SQRT2, 1.0],
        )
    }

    #[test]
    fn an_owned_curve_is_checked_once_and_lent_as_its_view() {
        let (points, knots, weights) = quarter();
        let owned =
            NurbsCurve::new(3, knots.to_vec(), points.to_vec(), Some(weights.to_vec())).unwrap();
        let borrowed = Nurbs {
            order: 3,
            knots: &knots,
            points: &points,
            weights: Some(&weights),
        }
        .validate()
        .unwrap();
        assert_eq!(owned.view().evaluate(0.3), borrowed.evaluate(0.3));
        assert_eq!(owned.view().prepare(), borrowed.prepare());
        assert_eq!(borrowed.to_curve(), owned);
        assert_eq!(owned.domain(), [0.0, 1.0]);
        assert_eq!(
            NurbsCurve::new(
                3,
                knots.to_vec(),
                points.to_vec(),
                Some(alloc::vec![1.0, 0.0, 1.0])
            ),
            Err(NurbsError::Weight { index: 1 })
        );
    }

    #[test]
    fn a_rational_quadratic_is_an_exact_quarter_circle() {
        let (points, knots, weights) = quarter();
        let curve = Nurbs {
            order: 3,
            knots: &knots,
            points: &points,
            weights: Some(&weights),
        };
        let curve = curve.validate().unwrap();
        assert_eq!(curve.evaluate(0.0), [1.0, 0.0, 0.0]);
        assert_eq!(curve.evaluate(1.0), [0.0, 1.0, 0.0]);
        // The midpoint by hand: homogeneous (1 + 2 w + 0, 0 + 2 w + 1) / 4
        // over weight (2 + 2 w) / 4, which is (1/sqrt 2, 1/sqrt 2).
        let mid = curve.evaluate(0.5);
        assert!((mid[0] - HALF_SQRT2).abs() < 1e-15 && (mid[1] - HALF_SQRT2).abs() < 1e-15);
        let samples = curve.sample(None, 16).unwrap();
        assert_eq!(samples.spans, 1);
        assert_eq!(samples.points.len(), 17);
        for p in &samples.points {
            let r = exedra_math::norm(*p);
            assert!((r - 1.0).abs() < 1e-15, "radius {r} at {p:?}");
        }
    }

    #[test]
    fn without_weights_a_clamped_cubic_is_its_bezier() {
        let points = [
            [0.0, 0.0, 0.0],
            [1.0, 2.0, 0.0],
            [3.0, 2.0, 0.0],
            [4.0, 0.0, 0.0],
        ];
        let knots = [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        let curve = Nurbs {
            order: 4,
            knots: &knots,
            points: &points,
            weights: None,
        };
        let bezier = crate::CubicBasis::Bezier.evaluate(points, 0.25);
        let nurbs = curve.validate().unwrap().evaluate(0.25);
        for c in 0..3 {
            assert!((bezier[c] - nurbs[c]).abs() < 1e-15);
        }
    }

    #[test]
    fn ranges_clip_the_spans_and_errors_are_typed() {
        let points: [[f64; 3]; 5] = core::array::from_fn(|i| [i as f64, 0.0, 0.0]);
        let knots = [0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.0];
        let curve = Nurbs {
            order: 2,
            knots: &knots,
            points: &points,
            weights: None,
        };
        let samples = curve
            .validate()
            .unwrap()
            .sample(Some([0.5, 2.5]), 2)
            .unwrap();
        assert_eq!(samples.spans, 3);
        assert_eq!(samples.params, [0.5, 0.75, 1.0, 1.5, 2.0, 2.25, 2.5]);
        // Linear B-spline over these knots: x = t.
        assert_eq!(samples.points[3], [1.5, 0.0, 0.0]);
        assert_eq!(
            Nurbs {
                knots: &knots[..6],
                ..curve
            }
            .validate()
            .err(),
            Some(NurbsError::KnotCount {
                knots: 6,
                expected: 7
            })
        );
        assert_eq!(
            Nurbs {
                weights: Some(&[1.0, 1.0, 0.0, 1.0, 1.0]),
                ..curve
            }
            .validate()
            .err(),
            Some(NurbsError::Weight { index: 2 })
        );
        let valid = curve.validate().unwrap();
        for range in [[5.0, 6.0], [1.0, 1.0], [f64::NAN, 2.0]] {
            assert_eq!(valid.sample(Some(range), 2), Err(NurbsError::EmptyRange));
        }
    }

    #[test]
    fn a_full_multiplicity_knot_is_a_break_sampled_on_both_sides() {
        // Order 2 with knot 1 repeated twice: two separate segments, from
        // x = 0 to 1 and from 10 to 11.
        let points = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [11.0, 0.0, 0.0],
        ];
        let knots = [0.0, 0.0, 1.0, 1.0, 2.0, 2.0];
        let curve = Nurbs {
            order: 2,
            knots: &knots,
            points: &points,
            weights: None,
        }
        .validate()
        .unwrap();
        let samples = curve.sample(None, 2).unwrap();
        assert_eq!(samples.params, [0.0, 0.5, 1.0, 1.0, 1.5, 2.0]);
        let x: Vec<f64> = samples.points.iter().map(|p| p[0]).collect();
        assert_eq!(x, [0.0, 0.5, 1.0, 10.0, 10.5, 11.0]);
        assert_eq!(samples.breaks, [3]);
        assert_eq!(samples.spans, 2);
        // Cropped at the break, the range ends on the left piece.
        let left = curve.sample(Some([0.0, 1.0]), 2).unwrap();
        assert_eq!(left.points.last(), Some(&[1.0, 0.0, 0.0]));
        assert!(left.breaks.is_empty());
        let right = curve.sample(Some([1.0, 2.0]), 2).unwrap();
        assert_eq!(right.points[0], [10.0, 0.0, 0.0]);
        assert!(right.breaks.is_empty());
        // A single evaluation at the knot takes the piece it starts.
        assert_eq!(curve.evaluate(1.0), [10.0, 0.0, 0.0]);
        // Multiplicity one below the order is continuous: no break.
        let knots = [0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 2.0];
        let points = [
            [0.0; 3],
            [1.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [4.0, 0.0, 0.0],
        ];
        let curve = Nurbs {
            order: 3,
            knots: &knots,
            points: &points,
            weights: None,
        };
        assert!(
            curve
                .validate()
                .unwrap()
                .sample(None, 4)
                .unwrap()
                .breaks
                .is_empty()
        );
    }

    #[test]
    fn high_orders_evaluate_past_the_inline_scratch() {
        // A clamped order-12 curve without interior knots is a Bezier
        // curve; its sampled and evaluated points agree.
        let points: Vec<[f64; 3]> = (0..12)
            .map(|i| [f64::from(i), f64::from(i * i % 5), 0.0])
            .collect();
        let knots: Vec<f64> = (0..24).map(|i| if i < 12 { 0.0 } else { 1.0 }).collect();
        let curve = Nurbs {
            order: 12,
            knots: &knots,
            points: &points,
            weights: None,
        }
        .validate()
        .unwrap();
        let samples = curve.sample(None, 4).unwrap();
        for (t, p) in samples.params.iter().zip(&samples.points) {
            assert_eq!(curve.evaluate(*t), *p);
        }
        assert_eq!(curve.evaluate(1.0), [11.0, 1.0, 0.0]);
    }

    /// A weighted cubic with an interior knot, for comparisons.
    fn weighted() -> ([[f64; 3]; 6], [f64; 10], [f64; 6]) {
        (
            [
                [0.0, 0.0, 0.0],
                [1.0, 2.0, 0.5],
                [3.0, 2.5, -1.0],
                [4.0, -1.0, 0.0],
                [6.0, 0.0, 2.0],
                [7.0, 1.0, 1.0],
            ],
            [0.0, 0.0, 0.0, 0.0, 0.4, 1.5, 2.0, 2.0, 2.0, 2.0],
            [1.0, 0.5, 2.0, 1.5, 0.75, 1.0],
        )
    }

    fn near(a: [f64; 3], b: [f64; 3], tolerance: f64) -> bool {
        (0..3).all(|c| (a[c] - b[c]).abs() <= tolerance * (1.0 + b[c].abs()))
    }

    #[test]
    fn derivatives_match_hand_values_and_differences() {
        // The quarter circle's start: C'(0) = 2 (w1 / w0) (P1 - P0).
        let (points, knots, weights) = quarter();
        let circle = Nurbs {
            order: 3,
            knots: &knots,
            points: &points,
            weights: Some(&weights),
        }
        .validate()
        .unwrap();
        let start = circle.evaluation(0.0);
        assert!(near(start.first, [0.0, 2.0 * HALF_SQRT2, 0.0], 1e-15));
        // On a circle the second derivative's part along the radius is
        // -|C'|^2 / r.
        for t in [0.1, 0.5, 0.9] {
            let e = circle.evaluation(t);
            let speed2 = exedra_math::dot(e.first, e.first);
            assert!((exedra_math::dot(e.second, e.point) + speed2).abs() < 1e-12);
        }
        let (points, knots, weights) = weighted();
        let curve = Nurbs {
            order: 4,
            knots: &knots,
            points: &points,
            weights: Some(&weights),
        }
        .validate()
        .unwrap();
        let h = 1e-6;
        for k in 1..20 {
            let t = f64::from(k) / 10.0 + 0.003;
            let e = curve.evaluation(t);
            assert!(near(e.point, curve.evaluate(t), 1e-13));
            let (a, b) = (curve.evaluation(t - h), curve.evaluation(t + h));
            let first = core::array::from_fn(|c| (b.point[c] - a.point[c]) / (2.0 * h));
            let second = core::array::from_fn(|c| (b.first[c] - a.first[c]) / (2.0 * h));
            assert!(near(e.first, first, 1e-6), "{t}: {:?} {first:?}", e.first);
            assert!(
                near(e.second, second, 1e-5),
                "{t}: {:?} {second:?}",
                e.second
            );
        }
    }

    #[test]
    fn prepared_spans_agree_with_de_boor() {
        let (points, knots, weights) = weighted();
        let curve = Nurbs {
            order: 4,
            knots: &knots,
            points: &points,
            weights: Some(&weights),
        }
        .validate()
        .unwrap();
        let prepared = curve.prepare();
        assert_eq!(prepared.spans(), 3);
        assert_eq!(prepared.domain(), [0.0, 2.0]);
        assert_eq!(prepared.span_domain(1), Some([0.4, 1.5]));
        assert_eq!(prepared.span_domain(3), None);
        assert_eq!(prepared.span_controls(2).map(<[_]>::len), Some(4));
        assert_eq!(prepared.controls().len(), 3 * 4);
        assert_eq!(
            prepared.controls()[8..],
            *prepared.span_controls(2).unwrap()
        );
        assert_eq!(prepared.bounds(), [0.0, 0.4, 1.5, 2.0]);
        assert!(prepared.breaks().is_empty());
        for k in 0..=40 {
            let t = f64::from(k) / 20.0;
            assert!(near(prepared.evaluate(t), curve.evaluate(t), 1e-13), "{t}");
            let (p, v) = (prepared.evaluation(t), curve.evaluation(t));
            assert!(near(p.first, v.first, 1e-12) && near(p.second, v.second, 1e-11));
        }
        // A span's Bezier controls start and end on the curve.
        let [a, b] = prepared.span_domain(1).unwrap();
        let controls = prepared.span_controls(1).unwrap();
        let ends = [controls[0], controls[3]].map(|c| c.point());
        assert!(near(ends[0], curve.evaluate(a), 1e-13));
        assert!(near(ends[1], curve.evaluate(b), 1e-13));
        // Preparing into an existing one reuses it.
        let mut again = Nurbs {
            order: 2,
            knots: &[0.0, 0.0, 1.0, 1.0],
            points: &points[..2],
            weights: None,
        }
        .validate()
        .unwrap()
        .prepare();
        curve.prepare_into(&mut Workspace::new(), &mut again);
        assert_eq!(again, prepared);
    }

    #[test]
    fn prepared_breaks_reach_both_sides() {
        let points = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [11.0, 0.0, 0.0],
        ];
        let knots = [0.0, 0.0, 1.0, 1.0, 2.0, 2.0];
        let prepared = Nurbs {
            order: 2,
            knots: &knots,
            points: &points,
            weights: None,
        }
        .validate()
        .unwrap()
        .prepare();
        assert_eq!(prepared.spans(), 2);
        assert_eq!(prepared.breaks(), [1]);
        assert_eq!(prepared.span_at(1.0), 1);
        assert_eq!(prepared.evaluate(1.0), [10.0, 0.0, 0.0]);
        let left = prepared.evaluation_in(0, 1.0).unwrap();
        assert_eq!(left.point, [1.0, 0.0, 0.0]);
        assert_eq!(left.first, [1.0, 0.0, 0.0]);
        assert!(prepared.evaluation_in(2, 1.0).is_none());
    }

    #[test]
    fn validation_guarantees_finite_inputs() {
        let points = [[0.0; 3], [1.0, 2.0, 0.0], [3.0, 1.0, 0.0]];
        let knots = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let curve = Nurbs {
            order: 3,
            knots: &knots,
            points: &points,
            weights: None,
        };
        assert!(curve.validate().is_ok());
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut with = points;
            with[1][2] = bad;
            assert_eq!(
                Nurbs {
                    points: &with,
                    ..curve
                }
                .validate()
                .err(),
                Some(NurbsError::Point { index: 1 }),
                "{bad}"
            );
        }
        // Finite, but not once weighted.
        let mut large = points;
        large[2][0] = 1e300;
        assert_eq!(
            Nurbs {
                points: &large,
                weights: Some(&[1.0, 1.0, 1e10]),
                ..curve
            }
            .validate()
            .err(),
            Some(NurbsError::Point { index: 2 })
        );
        // Finite knots whose span overflows.
        assert_eq!(
            Nurbs {
                knots: &[
                    -f64::MAX,
                    -f64::MAX,
                    -f64::MAX,
                    f64::MAX,
                    f64::MAX,
                    f64::MAX
                ],
                ..curve
            }
            .validate()
            .err(),
            Some(NurbsError::Knots { index: 5 })
        );
        // Zero-dimensional controls take no memory, so no knot slice can
        // be as long as their count asks.
        let empty: &[[f64; 0]] = &[[]; usize::MAX - 1];
        assert_eq!(
            Nurbs {
                order: 2,
                knots: &[],
                points: empty,
                weights: None,
            }
            .validate()
            .err(),
            Some(NurbsError::KnotCount {
                knots: 0,
                expected: usize::MAX
            })
        );
    }

    #[test]
    fn extreme_inputs_stay_finite_or_are_refused() {
        // Weights below 2^-256 are refused; at it, points stay finite.
        let tiny = f64::from_bits(0x2FF0_0000_0000_0000);
        let points = [[0.0; 3], [0.0; 3]];
        let knots = [0.0, 0.0, 1.0, 1.0];
        let low = [tiny / 2.0, tiny / 2.0];
        let at = [tiny, tiny];
        assert_eq!(
            Nurbs {
                order: 2,
                knots: &knots,
                points: &points,
                weights: Some(&low)
            }
            .validate()
            .err(),
            Some(NurbsError::Weight { index: 0 })
        );
        let valid = Nurbs {
            order: 2,
            knots: &knots,
            points: &points,
            weights: Some(&at),
        }
        .validate()
        .unwrap();
        assert_eq!(valid.evaluate(0.5), [0.0; 3]);
        assert_eq!(valid.prepare().evaluate(0.5), [0.0; 3]);
        // A span narrower than 1 / f64::MAX still has finite derivatives.
        let narrow = Nurbs {
            order: 2,
            knots: &[0.0, 0.0, 1e-310, 1e-310],
            points: &[[0.0; 3], [1e-310, 0.0, 0.0]],
            weights: None,
        }
        .validate()
        .unwrap();
        for e in [
            narrow.evaluation(5e-311),
            narrow.prepare().evaluation(5e-311),
        ] {
            assert_eq!(e.first, [1.0, 0.0, 0.0]);
            assert_eq!(e.second, [0.0; 3]);
        }
        // Accessors past the last span answer `None`, however far past.
        assert!(valid.prepare().span_controls(usize::MAX / 2).is_none());
        assert!(valid.prepare().span_controls(usize::MAX).is_none());
    }

    #[test]
    fn plane_curves_take_every_path_space_curves_do() {
        // The quarter circle in the plane, and the same in space.
        let (points, knots, weights) = quarter();
        let flat: Vec<[f64; 2]> = points.iter().map(|p| [p[0], p[1]]).collect();
        let plane = NurbsCurve::new(3, knots.to_vec(), flat, Some(weights.to_vec())).unwrap();
        let space =
            NurbsCurve::new(3, knots.to_vec(), points.to_vec(), Some(weights.to_vec())).unwrap();
        let (plane, space) = (plane.view(), space.view());
        for k in 0..=10 {
            let t = f64::from(k) / 10.0;
            let (a, b) = (plane.evaluation(t), space.evaluation(t));
            assert_eq!(a.point, [b.point[0], b.point[1]]);
            assert_eq!(a.first, [b.first[0], b.first[1]]);
            assert_eq!(a.second, [b.second[0], b.second[1]]);
        }
        let samples = plane.sample(None, 4).unwrap();
        assert_eq!(samples.points.len(), 5);
        let (prepared, solid) = (plane.prepare(), space.prepare());
        let (p, q) = (prepared.evaluate(0.3), solid.evaluate(0.3));
        assert_eq!(p, [q[0], q[1]]);
        let tolerance = crate::Tolerance {
            chord: 1e-4,
            angle: 0.2,
        };
        let (flat, report) = prepared.adaptive_sample(tolerance, 1000);
        let (round, same) = solid.adaptive_sample(tolerance, 1000);
        assert_eq!((flat.params, report), (round.params, same));
        assert_eq!(prepared.lengths(4), solid.lengths(4));
    }
}
