// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Arc length and tolerance-driven sampling.

use alloc::vec::Vec;

use exedra_math::Real;

use crate::nurbs::with_scratch;
use crate::query::{binomial, reciprocal_power_of_two, upper};
use crate::sample::span_controls;
use crate::vector::{WeightedPoint, add, dot, magnitude, norm, scale, sub};
use crate::{ControlOutOfRange, CubicBasis, NurbsSamples, PreparedNurbs, Span, Station, Workspace};

/// Five-point Gauss-Legendre nodes on `[-1, 1]` (Abramowitz and Stegun,
/// *Handbook of Mathematical Functions*, table 25.4): exact for
/// polynomials up to degree 9.
const NODES: [f64; 5] = [
    -0.906_179_845_938_664,
    -0.538_469_310_105_683_1,
    0.0,
    0.538_469_310_105_683_1,
    0.906_179_845_938_664,
];

/// The weights of [`NODES`].
const WEIGHTS: [f64; 5] = [
    0.236_926_885_056_189_1,
    0.478_628_670_499_366_5,
    0.568_888_888_888_888_9,
    0.478_628_670_499_366_5,
    0.236_926_885_056_189_1,
];

/// The start of piece `k` of `pieces` equal parts of `[a, b]`, with the
/// last piece ending at `b` exactly; an interpolation, so it cannot
/// overflow where `a` and `b` are finite.
fn piece_start(a: f64, b: f64, k: u32, pieces: u32) -> f64 {
    if k == 0 {
        a
    } else if k >= pieces {
        b
    } else {
        let f = f64::from(k) / f64::from(pieces);
        a * (1.0 - f) + b * f
    }
}

/// `integral(speed, a, b)` by five-point Gauss-Legendre on `pieces` equal
/// parts (0 taken as 1), summed in order.
pub(crate) fn integrate(speed: &mut impl FnMut(f64) -> f64, a: f64, b: f64, pieces: u32) -> f64 {
    let pieces = pieces.max(1);
    let mut sum = 0.0;
    for k in 0..pieces {
        let (from, to) = (
            piece_start(a, b, k, pieces),
            piece_start(a, b, k + 1, pieces),
        );
        sum += piece_integral(&node_speeds(speed, from, to), from, to);
    }
    sum
}

/// The speed at the Gauss-Legendre nodes of `[from, to]`.
fn node_speeds(speed: &mut impl FnMut(f64) -> f64, from: f64, to: f64) -> [f64; 5] {
    let half = 0.5 * (to - from);
    let middle = from + half;
    NODES.map(|node| speed(middle + half * node))
}

/// Gauss-Legendre's estimate of the integral over `[from, to]` from the
/// speeds at its nodes.
fn piece_integral(speeds: &[f64; 5], from: f64, to: f64) -> f64 {
    let half = 0.5 * (to - from);
    let mut sum = 0.0;
    for (s, weight) in speeds.iter().zip(WEIGHTS) {
        sum += weight * half * s;
    }
    sum
}

/// The polynomial through `speeds` at the nodes, at `x` in `[-1, 1]`: the
/// speed as Gauss-Legendre's rule sees it.
fn interpolated(speeds: &[f64; 5], x: f64) -> f64 {
    let mut sum = 0.0;
    for (k, s) in speeds.iter().enumerate() {
        let mut basis = 1.0;
        for (j, node) in NODES.iter().enumerate() {
            if j != k {
                basis *= (x - node) / (NODES[k] - node);
            }
        }
        sum += s * basis;
    }
    sum
}

/// The integral of [`interpolated`] from `-1` to `x`, in units of the
/// piece's half width: exact, by Gauss-Legendre on `[-1, x]`, since the
/// polynomial has degree 4. At `x = 1` it is the piece's own estimate.
fn interpolated_integral(speeds: &[f64; 5], x: f64) -> f64 {
    let half = 0.5 * (x + 1.0);
    let mut sum = 0.0;
    for (node, weight) in NODES.iter().zip(WEIGHTS) {
        sum += weight * half * interpolated(speeds, -1.0 + half * (1.0 + node));
    }
    sum
}

/// The `t` in `[a, b]` at which [`integrate`] from `a` reaches `length`,
/// clamped to the ends.
///
/// The pieces' own estimates, summed, find the piece; inside it, the
/// length from its start is the integral of the polynomial through the
/// node speeds, which agrees with the piece's estimate at its end and
/// grows wherever that polynomial is nonnegative, so a longer `length`
/// never gives an earlier `t` there. It is inverted by Newton's method,
/// with no further evaluation of the curve, inside a bracket that a
/// bisection halves whenever a step would leave it or shrink it too
/// little, until the bracket stops shrinking.
fn invert(speed: &mut impl FnMut(f64) -> f64, a: f64, b: f64, pieces: u32, length: f64) -> f64 {
    if length.is_nan() || length <= 0.0 {
        return a;
    }
    let pieces = pieces.max(1);
    let mut before = 0.0;
    for k in 0..pieces {
        let (from, to) = (
            piece_start(a, b, k, pieces),
            piece_start(a, b, k + 1, pieces),
        );
        let speeds = node_speeds(speed, from, to);
        let whole = piece_integral(&speeds, from, to);
        let target = length - before;
        if target >= whole {
            before += whole;
            continue;
        }
        // In the piece's own coordinate `x` in `[-1, 1]`.
        let half = 0.5 * (to - from);
        let length_at = |x: f64| half * interpolated_integral(&speeds, x);
        let (mut lo, mut hi) = (-1.0_f64, 1.0_f64);
        let mut x = -1.0 + 2.0 * (target / whole);
        // The bracket's widths one and two steps ago: a step that has not
        // halved it since the step before last bisects instead.
        let mut widths = [2.0, 2.0];
        for _ in 0..256 {
            let excess = length_at(x) - target;
            if excess < 0.0 {
                lo = x;
            } else if excess > 0.0 {
                hi = x;
            } else {
                break;
            }
            let rate = half * interpolated(&speeds, x);
            let newton = x - excess / rate;
            let stalled = hi - lo > 0.5 * widths[1];
            widths = [hi - lo, widths[0]];
            let next = if rate > 0.0 && lo < newton && newton < hi && !stalled {
                newton
            } else {
                f64::midpoint(lo, hi)
            };
            if next == x || !(lo < next && next < hi) {
                break;
            }
            x = next;
        }
        let f = 0.5 * (x + 1.0);
        return (from * (1.0 - f) + to * f).max(from).min(to);
    }
    b
}

impl CubicBasis {
    /// The arc length of the span over `u` in `[0, 1]`: the integral of
    /// `|dP/du|`, by five-point Gauss-Legendre quadrature (exact for
    /// polynomials to degree 9) on `pieces` equal parts of the span (0 taken
    /// as 1).
    ///
    /// The speed of a cubic is the square root of a quartic, not a
    /// polynomial, so the rule is not exact; it converges quickly where
    /// the speed stays away from zero and slowly near a cusp (a span whose
    /// derivative vanishes), where more pieces help. The result is an
    /// estimate, not a bound.
    #[must_use]
    pub fn arc_length(self, controls: [[f64; 3]; 4], pieces: u32) -> f64 {
        integrate(
            &mut |u| norm(self.derivative(controls, u)),
            0.0,
            1.0,
            pieces,
        )
    }

    /// The `u` in `[0, 1]` at which the span's [arc
    /// length](Self::arc_length) from `u = 0`, by the same quadrature,
    /// reaches `length` (clamped to the span): safeguarded Newton steps,
    /// within `1e-12` of the span's length or 64 steps.
    #[must_use]
    pub fn parameter_at_length(self, controls: [[f64; 3]; 4], pieces: u32, length: f64) -> f64 {
        invert(
            &mut |u| norm(self.derivative(controls, u)),
            0.0,
            1.0,
            pieces,
            length,
        )
    }
}

/// The cumulative arc lengths of a chain of spans: entry `s` is the length
/// from the chain's start to span `s`'s start, and the last entry, at
/// `spans.len()`, the chain's length. Each span's length is
/// [`CubicBasis::arc_length`] with `pieces`.
///
/// # Errors
///
/// [`ControlOutOfRange`] when a span names a control past `controls`.
pub fn chain_lengths(
    basis: CubicBasis,
    controls: &[[f64; 3]],
    spans: &[Span],
    pieces: u32,
) -> Result<Vec<f64>, ControlOutOfRange> {
    let mut out = Vec::new();
    chain_lengths_into(basis, controls, spans, pieces, &mut out)?;
    Ok(out)
}

/// [`chain_lengths`] into a caller's buffer, which is cleared first.
///
/// # Errors
///
/// As [`chain_lengths`]; `out` is left empty.
pub fn chain_lengths_into(
    basis: CubicBasis,
    controls: &[[f64; 3]],
    spans: &[Span],
    pieces: u32,
    out: &mut Vec<f64>,
) -> Result<(), ControlOutOfRange> {
    out.clear();
    out.reserve(spans.len() + 1);
    let mut total = 0.0;
    out.push(total);
    for span in 0..spans.len() {
        match span_controls(controls, spans, span_index(span)) {
            Ok(picked) => total += basis.arc_length(picked, pieces),
            Err(error) => {
                out.clear();
                return Err(error);
            }
        }
        out.push(total);
    }
    Ok(())
}

/// The station at `length` along a chain (clamped to the chain), given its
/// `cumulative` lengths from [`chain_lengths`] with the same `pieces`: the
/// span whose cumulative interval holds `length` (the earlier span at a
/// shared end), and the [parameter](CubicBasis::parameter_at_length) in it.
/// For a chain of no spans, span 0 at `u = 0`.
///
/// # Errors
///
/// [`ControlOutOfRange`] when the span found names a control past
/// `controls`.
pub fn chain_station_at_length(
    basis: CubicBasis,
    controls: &[[f64; 3]],
    spans: &[Span],
    cumulative: &[f64],
    pieces: u32,
    length: f64,
) -> Result<Station, ControlOutOfRange> {
    let count = spans.len().min(cumulative.len().saturating_sub(1));
    if count == 0 {
        return Ok(Station { span: 0, u: 0.0 });
    }
    // The first span whose end reaches `length`, or the last.
    let span = cumulative[1..count].partition_point(|&end| end < length);
    let picked = span_controls(controls, spans, span_index(span))?;
    let u = basis.parameter_at_length(picked, pieces, length - cumulative[span]);
    Ok(Station {
        span: span_index(span),
        u,
    })
}

/// A span index as the `u32` stations carry; a slice of spans longer than
/// `u32` indexes saturates, and its later spans are out of range.
fn span_index(span: usize) -> u32 {
    u32::try_from(span).unwrap_or(u32::MAX)
}

impl<const N: usize> PreparedNurbs<N> {
    /// The arc length of prepared span `span`, by five-point
    /// Gauss-Legendre quadrature on `pieces` equal parts, as
    /// [`CubicBasis::arc_length`] describes; `None` past the last span.
    #[must_use]
    pub fn span_length(&self, span: usize, pieces: u32) -> Option<f64> {
        self.span_domain(span)?;
        Some(with_scratch(self.order(), |scratch| {
            self.span_length_with(span, pieces, scratch)
        }))
    }

    /// As [`Self::span_length`] for a span that exists, with `scratch`,
    /// `order` long, for de Casteljau's scheme.
    fn span_length_with(&self, span: usize, pieces: u32, scratch: &mut [WeightedPoint<N>]) -> f64 {
        let [a, b] = self.span_domain(span).unwrap_or(self.domain());
        integrate(&mut self.speed(span, scratch), a, b, pieces)
    }

    /// The cumulative arc lengths of the prepared spans: entry `s` is the
    /// length from the domain's start to span `s`'s start, and the last
    /// entry the curve's length.
    #[must_use]
    pub fn lengths(&self, pieces: u32) -> Vec<f64> {
        let mut out = Vec::new();
        self.lengths_into(pieces, &mut Workspace::new(), &mut out);
        out
    }

    /// [`Self::lengths`] into a caller's buffer, which is cleared first,
    /// with `workspace` for scratch.
    pub fn lengths_into(&self, pieces: u32, workspace: &mut Workspace<N>, out: &mut Vec<f64>) {
        out.clear();
        out.reserve(self.spans() + 1);
        let scratch = &mut workspace.evaluation;
        scratch.clear();
        scratch.resize(self.order(), WeightedPoint::ZERO);
        let mut total = 0.0;
        out.push(total);
        for span in 0..self.spans() {
            total += self.span_length_with(span, pieces, scratch);
            out.push(total);
        }
    }

    /// The parameter at `length` along the curve (clamped to it), given its
    /// `cumulative` lengths from [`Self::lengths`] with the same `pieces`,
    /// found as [`chain_station_at_length`] finds a station.
    #[must_use]
    pub fn parameter_at_length(&self, cumulative: &[f64], pieces: u32, length: f64) -> f64 {
        let count = self.spans().min(cumulative.len().saturating_sub(1));
        if count == 0 {
            return self.domain()[0];
        }
        let span = cumulative[1..count].partition_point(|&end| end < length);
        let [a, b] = self.span_domain(span).unwrap_or(self.domain());
        with_scratch(self.order(), |scratch| {
            invert(
                &mut self.speed(span, scratch),
                a,
                b,
                pieces,
                length - cumulative[span],
            )
        })
    }

    /// `|C'(t)|` in span `span`.
    fn speed<'s>(
        &'s self,
        span: usize,
        scratch: &'s mut [WeightedPoint<N>],
    ) -> impl FnMut(f64) -> f64 + 's {
        let [a, b] = self.span_domain(span).unwrap_or(self.domain());
        move |t| norm(self.in_span_with(span, t.max(a).min(b), scratch).first)
    }
}

/// How finely [adaptive sampling](adaptive_stations) must follow a curve.
///
/// An interval between two samples is split at its parameter midpoint
/// while either bound fails. Both are taken from the interval's Bezier
/// piece. Its control polygon contains the curve's piece (the convex hull
/// property; Piegl and Tiller, *The NURBS Book*, 2nd ed., sections 1.4 and
/// 1.5, for rational pieces with positive weights). Its tangents point
/// along `A' w - A w'` for weighted controls `(A, w)`, a polynomial whose
/// Bezier coefficients span a cone holding them all, which for a
/// polynomial piece is its hodograph's. Both bounds allow for the rounding
/// that computed them, so they are bounds, not estimates: once an interval
/// passes, its piece of curve meets the tolerance everywhere, not only at
/// sampled points. They are conservative, and may split an interval whose
/// piece already meets the tolerance; [`PreparedNurbs::adaptive_intervals`]
/// reports each interval's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// The largest distance from the curve to the chord between an
    /// interval's ends, bounded by the polygon's farthest control.
    /// `f64::INFINITY` (or NaN) disables the test.
    pub chord: f64,
    /// The largest angle, in radians, between any two tangents within an
    /// interval, bounded by the widest pair of the tangent cone's
    /// coefficients, each widened by its rounding. That bound holds only
    /// while those angles are at most a right angle, so an angle between a
    /// right angle and pi is held to a right angle; anything at or above
    /// pi (or NaN) disables the test. Where the curve stops, as at a cusp,
    /// its tangent has no bound, and the interval there is
    /// [unresolved](AdaptiveReport::unresolved).
    pub angle: f64,
}

/// What [adaptive sampling](adaptive_stations) did.
///
/// Non-exhaustive, so it can report more later without breaking a caller
/// that reads its fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdaptiveReport {
    /// Samples produced.
    pub samples: u32,
    /// Rounds of splitting: each splits every interval that failed the
    /// tolerance in the round before, so refinement spreads evenly.
    pub rounds: u32,
    /// Whether the sample cap stopped refinement while intervals still
    /// failed the tolerance: the samples are the last whole round's,
    /// coarser than asked.
    pub capped: bool,
    /// Whether an interval still failed the tolerance when it could not
    /// usefully be split: narrower than `2^-40` of its span's parameter,
    /// or with no representable parameter inside it, or, where its chord
    /// was met but rounding left its tangent unbounded, narrower than
    /// `2^-12`, or not moving at all. That happens beside a point where
    /// the curve stops, as at a cusp, where its speed falls below the
    /// rounding of its span's coordinates. The interval was kept as it is,
    /// and does not meet the tolerance.
    pub unresolved: bool,
}

/// One interval between [adaptive samples](PreparedNurbs::adaptive_intervals)
/// and what it is proven to satisfy, up to the rounding its bounds allow
/// for.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Interval {
    /// The prepared span it lies in.
    pub span: u32,
    /// Its ends, in the curve's parameter.
    pub range: [f64; 2],
    /// The farthest the curve strays from the chord between its ends.
    pub chord: f64,
    /// The largest angle, in radians, between two of its tangents; at
    /// most a right angle, or infinite where it cannot be bounded, as near
    /// a point where the curve stops.
    pub angle: f64,
}

/// One sample of a curve being refined: where, and what the curve is
/// there.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Refined<const N: usize> {
    span: u32,
    /// Where in the span's own `[0, 1]` the sample was evaluated.
    u: f64,
    /// The same place in the curve's parameter, as reported.
    param: f64,
    point: [f64; N],
    derivative: [f64; N],
    /// Whether the interval from this sample to the next passed.
    settled: bool,
    /// That interval's bounds, `[chord, angle]`, once measured.
    bound: [f64; 2],
}

/// A sample as the curve evaluated it: where, in the span's own `[0, 1]`
/// and the curve's parameter, which the asked-for place rounds to, and
/// the curve's point and derivative there.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Evaluated<const N: usize> {
    u: f64,
    param: f64,
    point: [f64; N],
    derivative: [f64; N],
}

/// Intervals narrower than this in a span's parameter are not split.
const NARROWEST: f64 = 1.0 / 1_099_511_627_776.0; // 2^-40

/// Intervals whose chord is met but whose angle is uncertain are not split
/// narrower than this in a span's parameter.
const UNCERTAIN_NARROWEST: f64 = 1.0 / 4096.0; // 2^-12

/// Refines `spans` spans, each starting as its two ends `u = 0` and
/// `u = 1`, into `list`, in rounds, until every interval passes
/// `tolerance` or the next round would make `kept(list) > max`.
/// `dropped` is how many listed span ends the caller will not output.
///
/// `bound` gives the bounds ([`piece_bound`]) of a span's piece between
/// two samples of it.
fn refine<const N: usize, E>(
    spans: u32,
    dropped: u32,
    mut eval: impl FnMut(u32, f64) -> Result<Evaluated<N>, E>,
    mut bound: impl FnMut(&Refined<N>, &Refined<N>) -> PieceBound,
    tolerance: Tolerance,
    max: u32,
    [list, next]: [&mut Vec<Refined<N>>; 2],
) -> Result<AdaptiveReport, E> {
    let mut sample = |span: u32, u: f64| {
        eval(span, u).map(|e| Refined {
            span,
            u: e.u,
            param: e.param,
            point: e.point,
            derivative: e.derivative,
            settled: false,
            bound: [f64::INFINITY; 2],
        })
    };
    list.clear();
    for span in 0..spans {
        list.push(sample(span, 0.0)?);
        list.push(sample(span, 1.0)?);
    }
    // Pairwise angles bound the spread of the edges' positive cone only up
    // to a right angle, so a wider tolerance is held to one.
    let angle_limit = (tolerance.angle < core::f64::consts::PI).then(|| {
        #[expect(
            clippy::manual_clamp,
            reason = "`f64::clamp`'s NaN assertion would format floats into no_std binaries"
        )]
        let angle = tolerance.angle.max(0.0).min(core::f64::consts::FRAC_PI_2);
        angle
    });
    let chord_passes = |chord: f64| tolerance.chord.is_nan() || chord <= tolerance.chord;
    let angle_passes = |angle: f64| angle_limit.is_none_or(|limit| angle <= limit);
    let mut report = AdaptiveReport::default();
    let count = |list: &Vec<Refined<N>>| (list.len() as u64).saturating_sub(u64::from(dropped));
    loop {
        next.clear();
        let mut failing = 0_u64;
        for i in 0..list.len() {
            let a = list[i];
            next.push(a);
            let Some(b) = list.get(i + 1).filter(|b| b.span == a.span) else {
                continue;
            };
            if a.settled {
                continue;
            }
            let piece = bound(&a, b);
            let measured = [piece.chord, piece.angle];
            let chord_met = chord_passes(piece.chord);
            let failed = !(chord_met && angle_passes(piece.angle));
            // Near a point where the curve stops, its speed falls below the
            // cutting's rounding, which follows the span's scale, so ever
            // narrower pieces there only multiply angles no rounding can
            // show: an interval whose chord is met but whose angle is
            // uncertain splits only while wider than `UNCERTAIN_NARROWEST`.
            // One with no known direction at all does not move, and is not
            // split for its angle.
            let narrowest = if chord_met && piece.directionless {
                f64::INFINITY
            } else if chord_met && piece.uncertain {
                UNCERTAIN_NARROWEST
            } else {
                NARROWEST
            };
            // The middle, unless the interval is too narrow to split or
            // its middle rounds onto an end in either parameter.
            let middle = if failed && b.u - a.u > narrowest {
                let m = sample(a.span, 0.5 * (a.u + b.u))?;
                (a.u < m.u && m.u < b.u && a.param < m.param && m.param < b.param).then_some(m)
            } else {
                None
            };
            if failed && middle.is_none() {
                // Too narrow to split: left as it is, and reported.
                report.unresolved = true;
                if let Some(last) = next.last_mut() {
                    last.settled = true;
                    last.bound = measured;
                }
            } else if let Some(middle) = middle {
                failing += 1;
                next.push(middle);
            } else if let Some(last) = next.last_mut() {
                last.settled = true;
                last.bound = measured;
            }
        }
        if failing == 0 {
            // The same samples, now with every interval's bounds.
            core::mem::swap(list, next);
            break;
        }
        if count(list) + failing > u64::from(max) {
            report.capped = true;
            // Keep this round's samples, with the bounds of the intervals
            // that passed in it; `next` holds them all, in order, with a
            // middle after each that failed.
            let mut kept = next.iter();
            for r in list.iter_mut() {
                if let Some(measured) = kept.find(|n| n.span == r.span && n.u == r.u)
                    && measured.settled
                {
                    r.settled = true;
                    r.bound = measured.bound;
                }
            }
            break;
        }
        core::mem::swap(list, next);
        report.rounds += 1;
    }
    report.samples = u32::try_from(count(list)).unwrap_or(u32::MAX);
    Ok(report)
}

/// The absolute scale of the controls a piece was cut from, which the
/// rounding in cutting and projecting it grows with: errors follow the
/// span's coordinates, not the piece's, so a short piece far from the
/// origin, or near it from a large span, keeps them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Reach {
    /// The largest projected control.
    points: f64,
    /// The largest weighted control.
    weighted: f64,
    /// The heaviest and lightest weights.
    weights: [f64; 2],
}

impl Reach {
    pub(crate) fn of<const N: usize>(controls: &[WeightedPoint<N>]) -> Self {
        let mut reach = Self {
            points: 0.0,
            weighted: 0.0,
            weights: [0.0, f64::INFINITY],
        };
        for h in controls {
            reach.points = upper(reach.points, magnitude(h.point()));
            reach.weighted = upper(reach.weighted, magnitude(h.weighted));
            reach.weights = [
                reach.weights[0].max(h.weight),
                reach.weights[1].min(h.weight),
            ];
        }
        reach
    }

    /// How far rounding may move a point cut and projected from `order`
    /// such controls.
    fn point_error(&self, order: usize) -> f64 {
        let [heaviest, lightest] = self.weights;
        let levels = 8.0 * order as f64 * f64::EPSILON;
        // Subnormal rounding is absolute, and projection divides it by the
        // weight too.
        levels * (self.points + (self.weighted + self.points * heaviest) / lightest)
            + 64.0 * f64::from_bits(1) / lightest.min(1.0)
    }
}

/// What [`piece_bound`] proves of a piece.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PieceBound {
    /// The farthest the piece strays from the chord between its ends.
    pub(crate) chord: f64,
    /// The largest angle between two of its tangents.
    pub(crate) angle: f64,
    /// Whether that angle is unknown because a coefficient is no longer
    /// than its rounding, rather than because the piece turns too far.
    pub(crate) uncertain: bool,
    /// Whether no coefficient at all has a known direction, as on a piece
    /// that does not move: smaller pieces have smaller coefficients, so
    /// splitting it cannot help.
    pub(crate) directionless: bool,
}

/// Bounds on the rational Bezier piece with weighted controls `piece`,
/// cut from controls of `reach`: the farthest it strays from the chord
/// between its ends and the largest angle between two of its tangents,
/// each allowing for the rounding that computed the controls. `numerator`
/// is scratch.
///
/// The piece lies in its projected controls' convex hull, so no farther
/// from the chord than the farthest of them. Its tangents point along
/// `A' w - A w'`, a polynomial whose Bezier coefficients span a cone
/// holding them all; while every two coefficients are within a right
/// angle, the cone is no wider than the widest pair. Each coefficient may
/// be off by the rounding in forming it, which turns its direction by at
/// most `asin(noise / |n|)`; one no longer than that, even one computed as
/// exactly zero, has no known direction, so the angle is infinite, as near
/// a point where the curve stops. An angle past a right angle is infinite
/// too.
pub(crate) fn piece_bound<const N: usize>(
    piece: &[WeightedPoint<N>],
    reach: Reach,
    numerator: &mut Vec<[f64; N]>,
) -> PieceBound {
    let m = piece.len();
    let (first, last) = (piece[0].point(), piece[m - 1].point());
    let error = reach.point_error(m);
    // Projected on the chord's direction rather than divided by its
    // squared length, which underflows for tiny curves.
    let along = sub(last, first);
    let length = magnitude(along);
    let mut chord = 0.0;
    for h in &piece[1..m - 1] {
        let offset = sub(h.point(), first);
        #[expect(
            clippy::manual_clamp,
            reason = "`f64::clamp`'s NaN assertion would format floats into no_std binaries"
        )]
        let s = if length > 0.0 && length.is_finite() {
            let unit = along.map(|c| c / length);
            (dot(offset, unit) / length).max(0.0).min(1.0)
        } else {
            0.0
        };
        chord = upper(chord, magnitude(sub(offset, scale(along, s))));
    }
    // The control and both ends may each be off by the error.
    let chord = chord + 3.0 * error;
    let (angle, known_directions) = turn(piece, first, reach, numerator);
    PieceBound {
        chord: if chord >= 0.0 { chord } else { f64::INFINITY },
        angle,
        uncertain: known_directions < numerator.len(),
        directionless: known_directions == 0,
    }
}

/// The largest angle between two tangents of the piece, as
/// [`piece_bound`] describes, and how many of its coefficients have a
/// known direction.
fn turn<const N: usize>(
    piece: &[WeightedPoint<N>],
    origin: [f64; N],
    reach: Reach,
    numerator: &mut Vec<[f64; N]>,
) -> (f64, usize) {
    let m = piece.len();
    let p = m - 1;
    let pf = p as f64;
    // Weights within `[1, 2)` at most, by an exact power of two, and
    // controls measured from the piece's start, which moves no tangent.
    let heaviest = piece.iter().map(|h| h.weight).fold(0.0, f64::max);
    let factor = reciprocal_power_of_two(heaviest);
    let a = |i: usize| -> [f64; N] {
        core::array::from_fn(|k| (piece[i].weighted[k] - piece[i].weight * origin[k]) * factor)
    };
    let w = |i: usize| piece[i].weight * factor;
    // `A' w - A w'` as a Bezier polynomial of degree `2p - 1`: the
    // products of each hodograph with the other polynomial.
    numerator.clear();
    numerator.resize(2 * p, [0.0; N]);
    let degree = 2 * p - 1;
    for i in 0..=p {
        for j in 0..=p {
            let ai = a(i);
            if i < p {
                let a1 = sub(a(i + 1), ai);
                let c = pf * binomial(p - 1, i) * binomial(p, j) / binomial(degree, i + j);
                for k in 0..N {
                    numerator[i + j][k] += c * a1[k] * w(j);
                }
            }
            if j < p {
                let w1 = w(j + 1) - w(j);
                let c = pf * binomial(p, i) * binomial(p - 1, j) / binomial(degree, i + j);
                for k in 0..N {
                    numerator[i + j][k] -= c * ai[k] * w1;
                }
            }
        }
    }
    let largest_a = (0..=p).map(|i| magnitude(a(i))).fold(0.0, upper);
    let largest_w = (0..=p).map(w).fold(1.0, f64::max);
    // Each weighted control, and the start it is measured from, may be off
    // by the cutting's rounding at the span's scale, and each weight by
    // its share; each coefficient sums at most `p` such differences, with
    // Bernstein product weights summing to one, then rounds itself.
    let levels = 8.0 * m as f64 * f64::EPSILON;
    let [span_heaviest, _] = reach.weights;
    let control_error =
        factor * levels * (reach.weighted + span_heaviest * reach.points) + f64::from_bits(1);
    let weight_error = factor * levels * span_heaviest;
    let noise = pf * (2.0 * control_error * largest_w + 2.0 * largest_a * weight_error)
        + 16.0 * pf * pf * f64::EPSILON * largest_a * largest_w
        + 16.0 * f64::from_bits(1);
    let known_directions = numerator
        .iter()
        .filter(|&&n| known(n, noise).is_some())
        .count();
    if known_directions < numerator.len() {
        return (f64::INFINITY, known_directions);
    }
    let mut widest = 0.0_f64;
    for (i, &n) in numerator.iter().enumerate() {
        let Some((u, slack)) = known(n, noise) else {
            return (f64::INFINITY, known_directions);
        };
        for &other in &numerator[i + 1..] {
            let Some((v, other_slack)) = known(other, noise) else {
                return (f64::INFINITY, known_directions);
            };
            let gap = magnitude(sub(u, v));
            let angle = 2.0 * Real::asin((0.5 * gap).min(1.0)) + slack + other_slack;
            widest = upper(widest, angle);
        }
    }
    let angle = if widest <= core::f64::consts::FRAC_PI_2 {
        widest
    } else {
        f64::INFINITY
    };
    (angle, known_directions)
}

/// `n`'s direction and how far rounding by `noise` may turn it, or `None`
/// when `n` is no longer than the noise.
fn known<const N: usize>(n: [f64; N], noise: f64) -> Option<([f64; N], f64)> {
    let length = magnitude(n);
    if length.is_nan() || length <= noise || length.is_infinite() {
        return None;
    }
    Some((n.map(|c| c / length), Real::asin(noise / length)))
}

/// Stations along a chain of cubic spans, refined until each interval
/// meets `tolerance`: each span starts as its two ends and intervals are
/// split at their parameter midpoints in rounds, every failing interval in
/// a round at once, so the result does not depend on the order spans are
/// visited and refinement spreads evenly when the cap stops it.
///
/// The stations are listed as [`stations`](crate::stations) lists them
/// (each span's start and interior, then an open chain's end), so they
/// sample positions and per-control values alike with
/// [`sample()`](crate::sample()). Never fewer than one station per span
/// (plus the end of an open chain), however small `max_stations`; past
/// that, a round that would exceed `max_stations` is not taken, and the
/// report says so ([`AdaptiveReport::capped`]). Explicit counts remain the
/// default way to sample; this is for a caller that asks for geometric
/// quality instead.
///
/// The tests are on positions only: a swept surface whose radius or width
/// varies along the chain may need more stations than its centerline.
///
/// # Errors
///
/// [`ControlOutOfRange`] when a span names a control past `controls`.
pub fn adaptive_stations(
    basis: CubicBasis,
    controls: &[[f64; 3]],
    spans: &[Span],
    closed: bool,
    tolerance: Tolerance,
    max_stations: u32,
) -> Result<(Vec<Station>, AdaptiveReport), ControlOutOfRange> {
    let mut out = Vec::new();
    let report = adaptive_stations_into(
        basis,
        controls,
        spans,
        closed,
        tolerance,
        max_stations,
        &mut Workspace::new(),
        &mut out,
    )?;
    Ok((out, report))
}

/// [`adaptive_stations`] into a caller's buffer, which is cleared first,
/// with `workspace` as scratch.
///
/// # Errors
///
/// As [`adaptive_stations`]; `out` is left empty.
#[expect(
    clippy::too_many_arguments,
    reason = "the allocating form's inputs plus the two reused buffers"
)]
pub fn adaptive_stations_into(
    basis: CubicBasis,
    controls: &[[f64; 3]],
    spans: &[Span],
    closed: bool,
    tolerance: Tolerance,
    max_stations: u32,
    workspace: &mut Workspace,
    out: &mut Vec<Station>,
) -> Result<AdaptiveReport, ControlOutOfRange> {
    out.clear();
    let count = span_index(spans.len());
    // Every span's end is listed; an open chain outputs only the last.
    let dropped = if closed {
        count
    } else {
        count.saturating_sub(1)
    };
    let numerator = &mut workspace.numerator;
    let report = refine(
        count,
        dropped,
        |span, u| {
            let picked = span_controls(controls, spans, span)?;
            Ok(Evaluated {
                u,
                param: u,
                point: basis.evaluate(picked, u),
                derivative: basis.derivative(picked, u),
            })
        },
        // A cubic piece's Bezier controls from its ends' points and
        // derivatives: `P(a) + h/3 P'(a)` and `P(b) - h/3 P'(b)`.
        |a, b| {
            let third = (b.u - a.u) / 3.0;
            let piece = [
                a.point,
                add(a.point, scale(a.derivative, third)),
                sub(b.point, scale(b.derivative, third)),
                b.point,
            ]
            .map(|p| WeightedPoint::new(p, 1.0));
            // The span's points and derivatives weigh its controls by at
            // most 8 in all, the bases' largest row sums with the
            // derivative's factor of three.
            let reach = span_controls(controls, spans, a.span).map_or(f64::INFINITY, |picked| {
                8.0 * picked.iter().map(|c| magnitude(*c)).fold(0.0, upper)
            });
            let reach = Reach::of(&[WeightedPoint::new([reach, 0.0, 0.0], 1.0)]);
            piece_bound(&piece, reach, numerator)
        },
        tolerance,
        max_stations,
        [&mut workspace.refined, &mut workspace.refining],
    )?;
    let last = count.saturating_sub(1);
    out.extend(
        workspace
            .refined
            .iter()
            .filter(|r| r.u < 1.0 || (!closed && r.span == last))
            .map(|r| Station {
                span: r.span,
                u: r.u,
            }),
    );
    Ok(report)
}

impl<const N: usize> PreparedNurbs<N> {
    /// Samples the whole curve refined until each interval meets
    /// `tolerance`, as [`adaptive_stations`] refines a chain: each prepared
    /// span starts as its two ends, and intervals split at their parameter
    /// midpoints in rounds until they pass or a round would exceed
    /// `max_samples`. Never fewer than one sample per span, the curve's
    /// end, and the end of each piece before a [break](Self::breaks); the
    /// samples' [`NurbsSamples::breaks`] mark them as uniform sampling's
    /// do.
    #[must_use]
    pub fn adaptive_sample(
        &self,
        tolerance: Tolerance,
        max_samples: u32,
    ) -> (NurbsSamples<N>, AdaptiveReport) {
        let mut out = NurbsSamples::default();
        let report =
            self.adaptive_sample_into(tolerance, max_samples, &mut Workspace::new(), &mut out);
        (out, report)
    }

    /// [`Self::adaptive_sample`] into a caller's samples, whose buffers are
    /// cleared first, with `workspace` as scratch.
    pub fn adaptive_sample_into(
        &self,
        tolerance: Tolerance,
        max_samples: u32,
        workspace: &mut Workspace<N>,
        out: &mut NurbsSamples<N>,
    ) -> AdaptiveReport {
        out.params.clear();
        out.points.clear();
        out.breaks.clear();
        let spans = span_index(self.spans());
        out.spans = spans;
        let report = self.refine_curve(tolerance, max_samples, workspace);
        let breaks = self.breaks();
        let keeps_end = |span: u32| span + 1 == spans || breaks.contains(&(span as usize + 1));
        for r in &workspace.refined {
            if r.u >= 1.0 && !keeps_end(r.span) {
                continue;
            }
            if r.u == 0.0 && breaks.contains(&(r.span as usize)) {
                out.breaks.push(out.params.len());
            }
            out.params.push(r.param);
            out.points.push(r.point);
        }
        report
    }

    /// Refines the prepared spans into `workspace`'s list, as
    /// [`Self::adaptive_sample`] describes, and reports how it went.
    fn refine_curve(
        &self,
        tolerance: Tolerance,
        max_samples: u32,
        workspace: &mut Workspace<N>,
    ) -> AdaptiveReport {
        let spans = span_index(self.spans());
        let breaks = self.breaks();
        // A span's end is output when a break or the domain's end follows.
        let keeps_end = |span: u32| span + 1 == spans || breaks.contains(&(span as usize + 1));
        let kept = (0..spans).filter(|&s| keeps_end(s)).count();
        let dropped = spans.saturating_sub(span_index(kept));
        let scratch = &mut workspace.homogeneous;
        let piece = &mut workspace.piece;
        let numerator = &mut workspace.numerator;
        let evaluation = &mut workspace.evaluation;
        evaluation.clear();
        evaluation.resize(self.order(), WeightedPoint::ZERO);
        let refined = refine::<N, core::convert::Infallible>(
            spans,
            dropped,
            |span, u| {
                let span = span as usize;
                let [a, b] = self.span_domain(span).unwrap_or(self.domain());
                let t = if u >= 1.0 { b } else { a + (b - a) * u };
                let e = self.in_span_with(span, t, evaluation);
                // The span's own parameter as evaluation found it from `t`,
                // which the piece is cut at too.
                Ok(Evaluated {
                    u: (t - a) / (b - a),
                    param: t,
                    point: e.point,
                    derivative: e.first,
                })
            },
            // The piece's weighted Bezier controls over `[a.u, b.u]` of the
            // span: control `i` is the blossom at `p - i` copies of `a.u`
            // and `i` of `b.u`, by de Casteljau's scheme with one parameter
            // per level.
            |a, b| {
                let Some(span) = self.span_controls(a.span as usize) else {
                    return PieceBound {
                        chord: f64::INFINITY,
                        angle: f64::INFINITY,
                        uncertain: false,
                        directionless: false,
                    };
                };
                let order = span.len();
                piece.clear();
                for i in 0..order {
                    scratch.clear();
                    scratch.extend_from_slice(span);
                    for r in 1..order {
                        let t = if r < order - i { a.u } else { b.u };
                        for j in 0..order - r {
                            scratch[j] = WeightedPoint::lerp(scratch[j], scratch[j + 1], t);
                        }
                    }
                    piece.push(scratch[0]);
                }
                piece_bound(piece, Reach::of(span), numerator)
            },
            tolerance,
            max_samples,
            [&mut workspace.refined, &mut workspace.refining],
        );
        match refined {
            Ok(report) => report,
            Err(never) => match never {},
        }
    }

    /// The intervals between [adaptive samples](Self::adaptive_sample),
    /// each with the bounds that settled it: how far the curve strays from
    /// its chord and how far its tangent turns, allowing for rounding.
    ///
    /// Intervals are listed in order, one per pair of consecutive samples
    /// within a prepared span. An interval that met the tolerance has
    /// bounds within it; one too narrow to split has the bounds that
    /// failed ([`AdaptiveReport::unresolved`]); one left when the sample
    /// cap stopped refinement has infinite bounds, being unproven
    /// ([`AdaptiveReport::capped`]).
    #[must_use]
    pub fn adaptive_intervals(
        &self,
        tolerance: Tolerance,
        max_samples: u32,
    ) -> (Vec<Interval>, AdaptiveReport) {
        let mut out = Vec::new();
        let report =
            self.adaptive_intervals_into(tolerance, max_samples, &mut Workspace::new(), &mut out);
        (out, report)
    }

    /// [`Self::adaptive_intervals`] into a caller's buffer, which is
    /// cleared first, with `workspace` as scratch.
    pub fn adaptive_intervals_into(
        &self,
        tolerance: Tolerance,
        max_samples: u32,
        workspace: &mut Workspace<N>,
        out: &mut Vec<Interval>,
    ) -> AdaptiveReport {
        out.clear();
        let report = self.refine_curve(tolerance, max_samples, workspace);
        for pair in workspace.refined.windows(2) {
            let [a, b] = [pair[0], pair[1]];
            if a.span == b.span {
                out.push(Interval {
                    span: a.span,
                    range: [a.param, b.param],
                    chord: a.bound[0],
                    angle: a.bound[1],
                });
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Nurbs, sample};

    const HALF_SQRT2: f64 = core::f64::consts::FRAC_1_SQRT_2;

    fn quarter_circle() -> PreparedNurbs {
        Nurbs {
            order: 3,
            knots: &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            points: &[[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            weights: Some(&[1.0, HALF_SQRT2, 1.0]),
        }
        .validate()
        .unwrap()
        .prepare()
    }

    #[test]
    fn arc_lengths_converge_on_known_lengths() {
        // A line with bunched handles: length 3, at uneven speed.
        let line = [[0.0; 3], [0.1, 0.0, 0.0], [0.2, 0.0, 0.0], [3.0, 0.0, 0.0]];
        assert!((CubicBasis::Bezier.arc_length(line, 1) - 3.0).abs() < 1e-14);
        // Its speed is a polynomial there, so the rule is exact, and the
        // parameter at a length lands on that length.
        for target in [0.0, 0.5, 1.5, 2.9, 3.0, 4.0] {
            let u = CubicBasis::Bezier.parameter_at_length(line, 1, target);
            let x = CubicBasis::Bezier.evaluate(line, u)[0];
            assert!((x - target.min(3.0)).abs() < 1e-11, "{target}: {x}");
        }
        // The quarter circle is pi / 2 long; more pieces converge.
        let circle = quarter_circle();
        let coarse = circle.lengths(1)[1];
        let fine = circle.lengths(8)[1];
        let exact = core::f64::consts::FRAC_PI_2;
        assert!((fine - exact).abs() < 1e-12, "{fine}");
        assert!((coarse - exact).abs() < 1e-5 && (coarse - exact).abs() > (fine - exact).abs());
        // Halfway along the arc is the 45 degree point.
        let lengths = circle.lengths(8);
        let t = circle.parameter_at_length(&lengths, 8, exact / 2.0);
        let p = circle.evaluate(t);
        assert!((p[0] - HALF_SQRT2).abs() < 1e-11 && (p[1] - HALF_SQRT2).abs() < 1e-11);
    }

    #[test]
    fn chains_accumulate_and_find_their_stations() {
        let controls: Vec<[f64; 3]> = (0..7).map(|i| [f64::from(i), 0.0, 0.0]).collect();
        let spans = [[0, 1, 2, 3], [3, 4, 5, 6]];
        let lengths = chain_lengths(CubicBasis::Bezier, &controls, &spans, 2).unwrap();
        assert_eq!(lengths.len(), 3);
        assert!((lengths[1] - 3.0).abs() < 1e-14 && (lengths[2] - 6.0).abs() < 1e-14);
        let at = |length| {
            chain_station_at_length(CubicBasis::Bezier, &controls, &spans, &lengths, 2, length)
                .unwrap()
        };
        assert_eq!(at(0.0), Station { span: 0, u: 0.0 });
        assert_eq!(at(10.0), Station { span: 1, u: 1.0 });
        let middle = at(4.5);
        assert_eq!(middle.span, 1);
        assert!((middle.u - 0.5).abs() < 1e-12);
        assert_eq!(
            chain_lengths(CubicBasis::Bezier, &controls[..5], &spans, 2),
            Err(ControlOutOfRange {
                span: 1,
                control: 5,
                controls: 5
            })
        );
    }

    /// A Catmull-Rom chain through eight points of the unit circle.
    fn circle_chain() -> (Vec<[f64; 3]>, Vec<Span>) {
        let controls = (0..8)
            .map(|k| {
                let (sin, cos) = Real::sin_cos(core::f64::consts::TAU * f64::from(k) / 8.0);
                [cos, sin, 0.0]
            })
            .collect();
        let spans = (0..8)
            .map(|s| [s, (s + 1) % 8, (s + 2) % 8, (s + 3) % 8])
            .collect();
        (controls, spans)
    }

    #[test]
    fn adaptive_stations_meet_the_tolerance_or_report_the_cap() {
        let (controls, spans) = circle_chain();
        let tolerance = Tolerance {
            chord: 1e-3,
            angle: f64::INFINITY,
        };
        let (at, report) = adaptive_stations(
            CubicBasis::CatmullRom,
            &controls,
            &spans,
            true,
            tolerance,
            10_000,
        )
        .unwrap();
        assert!(!report.capped);
        assert_eq!(report.samples as usize, at.len());
        assert!(at.len() > 8 && report.rounds > 0);
        // Every interval's midpoint is within the chord tolerance.
        let points = sample(CubicBasis::CatmullRom, &controls, &spans, &at).unwrap();
        for (i, station) in at.iter().enumerate() {
            let next = at.get(i + 1).copied().unwrap_or(Station {
                span: station.span,
                u: 1.0,
            });
            let end_u = if next.span == station.span {
                next.u
            } else {
                1.0
            };
            let picked = span_controls(&controls, &spans, station.span).unwrap();
            let middle = CubicBasis::CatmullRom.evaluate(picked, 0.5 * (station.u + end_u));
            let end = CubicBasis::CatmullRom.evaluate(picked, end_u);
            let chord = sub(end, points[i]);
            let offset = sub(middle, points[i]);
            let s = dot(offset, chord) / dot(chord, chord);
            assert!(norm(sub(offset, scale(chord, s))) <= 1e-3);
        }
        // A cap below what the tolerance needs stops at a whole round.
        let (capped, report) = adaptive_stations(
            CubicBasis::CatmullRom,
            &controls,
            &spans,
            true,
            tolerance,
            20,
        )
        .unwrap();
        assert!(report.capped && capped.len() <= 20 && capped.len() >= 8);
        // Straight spans need no splitting; the minimum is always given.
        let line: Vec<[f64; 3]> = (0..4).map(|i| [f64::from(i), 0.0, 0.0]).collect();
        let (at, report) = adaptive_stations(
            CubicBasis::Bezier,
            &line,
            &[[0, 1, 2, 3]],
            false,
            tolerance,
            0,
        )
        .unwrap();
        assert_eq!(
            at,
            [Station { span: 0, u: 0.0 }, Station { span: 0, u: 1.0 }]
        );
        assert_eq!(report.rounds, 0);
        assert!(!report.capped);
    }

    #[test]
    fn adaptive_nurbs_samples_turn_by_at_most_the_angle() {
        let circle = quarter_circle();
        let tolerance = Tolerance {
            chord: f64::INFINITY,
            angle: 0.1,
        };
        let (samples, report) = circle.adaptive_sample(tolerance, 1000);
        assert!(!report.capped);
        assert_eq!(samples.params.first(), Some(&0.0));
        assert_eq!(samples.params.last(), Some(&1.0));
        for pair in samples.points.windows(2) {
            // Consecutive points on the unit circle less than 0.1 radians
            // apart.
            assert!(dot(pair[0], pair[1]) > 0.995);
        }
        // A break keeps both sides.
        let jump = Nurbs {
            order: 2,
            knots: &[0.0, 0.0, 1.0, 1.0, 2.0, 2.0],
            points: &[
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [11.0, 0.0, 0.0],
            ],
            weights: None,
        }
        .validate()
        .unwrap()
        .prepare();
        let (samples, _) = jump.adaptive_sample(tolerance, 100);
        assert_eq!(samples.params, [0.0, 1.0, 1.0, 2.0]);
        assert_eq!(samples.breaks, [2]);
        assert_eq!(samples.points[1], [1.0, 0.0, 0.0]);
        assert_eq!(samples.points[2], [10.0, 0.0, 0.0]);
    }

    #[test]
    fn an_excursion_between_matching_ends_and_middle_is_seen() {
        // Ends, middle and end tangents all agree, yet the curve swings
        // out to about 0.2887 between them.
        let controls = [[0.0; 3], [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0; 3]];
        let tolerance = Tolerance {
            chord: 1e-3,
            angle: 0.1,
        };
        let (at, report) = adaptive_stations(
            CubicBasis::Bezier,
            &controls,
            &[[0, 1, 2, 3]],
            false,
            tolerance,
            1000,
        )
        .unwrap();
        assert!(!report.capped);
        // Between consecutive stations the curve stays within the chord
        // tolerance of their chord, checked densely.
        for pair in at.windows(2) {
            let ends = [pair[0], pair[1]].map(|s| CubicBasis::Bezier.evaluate(controls, s.u));
            for k in 1..64 {
                let u = pair[0].u + (pair[1].u - pair[0].u) * f64::from(k) / 64.0;
                let p = CubicBasis::Bezier.evaluate(controls, u);
                let chord = sub(ends[1], ends[0]);
                let offset = sub(p, ends[0]);
                let length2 = dot(chord, chord);
                let s = if length2 > 0.0 {
                    (dot(offset, chord) / length2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                assert!(
                    norm(sub(offset, scale(chord, s))) <= 1e-3,
                    "{pair:?} at {u}"
                );
            }
        }
    }

    /// A clamped NURBS through `points`, one Bezier piece of their count.
    fn bezier(points: &[[f64; 3]], weights: Option<&[f64]>) -> PreparedNurbs {
        let order = points.len();
        let knots: Vec<f64> = (0..2 * order)
            .map(|i| if i < order { 0.0 } else { 1.0 })
            .collect();
        Nurbs {
            order: u32::try_from(order).unwrap(),
            knots: &knots,
            points,
            weights,
        }
        .validate()
        .unwrap()
        .prepare()
    }

    const ANGLE_ONLY: f64 = f64::INFINITY;

    #[test]
    fn wide_angles_are_certified_only_up_to_a_right_angle() {
        // Edges pairwise at most 3 pi / 4 apart, but the tangents at u = 0
        // and u = 2 / 3 are opposite.
        let curve = bezier(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [-1.0, 0.0, 0.0],
            ],
            None,
        );
        let tolerance = Tolerance {
            chord: ANGLE_ONLY,
            angle: 2.4,
        };
        let (samples, report) = curve.adaptive_sample(tolerance, 1000);
        assert!(samples.points.len() > 2 && !report.capped, "{report:?}");
        // Within each interval the tangent turns by at most the tolerance.
        for pair in samples.params.windows(2) {
            let first = curve.evaluation(pair[0]).first;
            for k in 1..=16 {
                let t = pair[0] + (pair[1] - pair[0]) * f64::from(k) / 16.0;
                let other = curve.evaluation(t).first;
                let cos = dot(first, other) / (norm(first) * norm(other));
                assert!(cos.acos() <= 2.4 + 1e-9, "{pair:?}");
            }
        }
    }

    #[test]
    fn a_cusp_too_narrow_to_split_is_reported() {
        // x = (3u - 1)^2: the tangent reverses at u = 1 / 3.
        let curve = bezier(&[[1.0, 0.0, 0.0], [-2.0, 0.0, 0.0], [4.0, 0.0, 0.0]], None);
        let tolerance = Tolerance {
            chord: ANGLE_ONLY,
            angle: 0.1,
        };
        let (_, report) = curve.adaptive_sample(tolerance, 1000);
        assert!(report.unresolved && !report.capped, "{report:?}");
        // A tolerance met everywhere reports nothing unresolved.
        let (_, report) = quarter_circle().adaptive_sample(tolerance, 1000);
        assert!(!report.unresolved, "{report:?}");
    }

    #[test]
    fn tiny_edges_keep_their_angles() {
        let curve = bezier(
            &[
                [0.0, 0.0, 0.0],
                [1e-100, 0.0, 0.0],
                [2e-100, 1e-100, 0.0],
                [3e-100, 2e-100, 0.0],
            ],
            None,
        );
        let tolerance = Tolerance {
            chord: ANGLE_ONLY,
            angle: 0.1,
        };
        let (samples, _) = curve.adaptive_sample(tolerance, 1000);
        assert!(samples.points.len() > 2);
    }

    #[test]
    fn longer_lengths_never_give_earlier_parameters() {
        // A line whose weights crowd its parameter toward the far end.
        let curve = Nurbs {
            order: 2,
            knots: &[0.0, 0.0, 1.0, 1.0],
            points: &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            weights: Some(&[1000.0, 1.0]),
        }
        .validate()
        .unwrap()
        .prepare();
        for pieces in [1, 4] {
            let lengths = curve.lengths(pieces);
            let total = lengths[1];
            let mut last = 0.0;
            for k in 0..=200 {
                let t = curve.parameter_at_length(&lengths, pieces, total * f64::from(k) / 200.0);
                assert!(t >= last, "{pieces} pieces, step {k}: {t} after {last}");
                last = t;
            }
            assert_eq!(last, 1.0);
        }
    }

    #[test]
    fn huge_parameters_stay_inside_their_domain() {
        let curve = Nurbs {
            order: 2,
            knots: &[1e308, 1e308, 1.1e308, 1.1e308],
            points: &[[0.0, 0.0, 0.0], [1e307, 0.0, 0.0]],
            weights: Some(&[100.0, 1.0]),
        }
        .validate()
        .unwrap()
        .prepare();
        let lengths = curve.lengths(1);
        let t = curve.parameter_at_length(&lengths, 1, 0.5 * lengths[1]);
        assert!((1e308..=1.1e308).contains(&t), "{t}");
    }

    #[test]
    fn adaptive_intervals_hold_their_bounds() {
        let weighted = bezier(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 2.0, 0.5],
                [3.0, 2.5, -1.0],
                [4.0, -1.0, 0.0],
            ],
            Some(&[1.0, 0.5, 2.0, 1.5]),
        );
        let tolerance = Tolerance {
            chord: 1e-3,
            angle: 0.1,
        };
        for curve in [quarter_circle(), weighted] {
            let (intervals, report) = curve.adaptive_intervals(tolerance, 10_000);
            assert!(!report.capped && !report.unresolved, "{report:?}");
            let (samples, _) = curve.adaptive_sample(tolerance, 10_000);
            assert_eq!(intervals.len() + 1, samples.params.len());
            let [start, end] = curve.domain();
            assert_eq!(intervals[0].range[0], start);
            assert_eq!(intervals[intervals.len() - 1].range[1], end);
            for (interval, pair) in intervals.iter().zip(samples.params.windows(2)) {
                assert_eq!(interval.range, [pair[0], pair[1]]);
                assert!(interval.chord <= tolerance.chord && interval.angle <= tolerance.angle);
                // Dense samples stay within both bounds.
                let [t0, t1] = interval.range;
                let (a, b) = (curve.evaluate(t0), curve.evaluate(t1));
                let first = curve.evaluation(t0).first;
                for i in 0..=64 {
                    let t = t0 + (t1 - t0) * f64::from(i) / 64.0;
                    let e = curve.evaluation(t);
                    let d = sub(b, a);
                    let s = (dot(sub(e.point, a), d) / dot(d, d)).clamp(0.0, 1.0);
                    let off = norm(sub(sub(e.point, a), scale(d, s)));
                    assert!(off <= interval.chord, "{off} past {interval:?}");
                    let cos = dot(e.first, first) / (norm(e.first) * norm(first));
                    assert!(cos.min(1.0).acos() <= interval.angle + 1e-7, "{interval:?}");
                }
            }
        }
    }

    #[test]
    fn capped_intervals_keep_the_bounds_they_met() {
        let tolerance = Tolerance {
            chord: 1e-9,
            angle: 0.01,
        };
        let (intervals, report) = quarter_circle().adaptive_intervals(tolerance, 20);
        assert!(report.capped, "{report:?}");
        for interval in &intervals {
            let met = interval.chord <= tolerance.chord && interval.angle <= tolerance.angle;
            assert!(met || interval.chord.is_infinite(), "{interval:?}");
        }
    }

    #[test]
    fn a_stationary_point_has_no_angle_bound() {
        // x = (3u - 1)^2 stops and turns back at u = 1 / 3.
        let curve = bezier(&[[1.0, 0.0, 0.0], [-2.0, 0.0, 0.0], [4.0, 0.0, 0.0]], None);
        let tolerance = Tolerance {
            chord: ANGLE_ONLY,
            angle: 0.1,
        };
        let (intervals, report) = curve.adaptive_intervals(tolerance, 1000);
        assert!(report.unresolved, "{report:?}");
        let stuck: Vec<&Interval> = intervals.iter().filter(|i| i.angle > 0.1).collect();
        assert!(!stuck.is_empty());
        // Only beside the stop, where the speed falls toward its rounding.
        for interval in stuck {
            let [t0, t1] = interval.range;
            assert!((t0 - 1.0 / 3.0).abs() < 1e-3 && (t1 - 1.0 / 3.0).abs() < 1e-3);
            assert!(interval.angle.is_infinite(), "{interval:?}");
        }
    }

    #[test]
    fn rounding_at_the_span_scale_is_allowed_for() {
        // Unit steps far from the origin: cutting rounds every control to
        // a whole number, so the pieces' own controls hide their turns.
        let b = 2.0_f64.powi(52);
        let far = bezier(
            &[
                [b, b, 0.0],
                [b + 1.0, b + 1.0, 0.0],
                [b + 2.0, b + 1.0, 0.0],
                [b + 3.0, b, 0.0],
            ],
            None,
        );
        let tolerance = Tolerance {
            chord: 1000.0,
            angle: 0.1,
        };
        let (intervals, report) = far.adaptive_intervals(tolerance, 1000);
        assert!(report.unresolved || report.capped, "{report:?}");
        for interval in intervals.iter().filter(|i| i.angle <= tolerance.angle) {
            // Any angle claimed holds against the exact curve, which turns
            // a right angle overall.
            assert!(interval.angle.is_infinite() || interval.range[1] - interval.range[0] < 1e-3);
        }
        // A line near the origin cut from a unit span: a chord tolerance
        // below the span's rounding is never claimed.
        let line = bezier(&[[-1.0, -1.0 / 3.0, 0.0], [1.0, 1.0 / 3.0, 0.0]], None);
        let fine = Tolerance {
            chord: 3e-18,
            angle: f64::INFINITY,
        };
        let (intervals, report) = line.adaptive_intervals(fine, 10_000);
        assert!(report.unresolved || report.capped, "{report:?}");
        assert!(intervals.iter().all(|i| i.chord > 3e-18), "{intervals:?}");
    }

    #[test]
    fn stops_at_samples_and_constant_curves_are_unresolved() {
        // x = (2u - 1)^2: it stops and turns back exactly at the middle.
        let dyadic = bezier(&[[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]], None);
        let tolerance = Tolerance {
            chord: ANGLE_ONLY,
            angle: 0.1,
        };
        let (intervals, report) = dyadic.adaptive_intervals(tolerance, 1000);
        assert!(report.unresolved, "{report:?}");
        // Neither side's angle beside the stop is claimed, and little
        // refinement is spent trying.
        let beside: Vec<&Interval> = intervals.iter().filter(|i| i.angle.is_infinite()).collect();
        assert!(!beside.is_empty());
        for interval in beside {
            assert!(
                interval.range[0] <= 0.5 && 0.5 <= interval.range[1] + 1e-3,
                "{interval:?}"
            );
        }
        assert!(intervals.len() < 64, "{}", intervals.len());
        let constant = bezier(&[[1.0, 2.0, 3.0]; 4], None);
        let (_, report) = constant.adaptive_intervals(tolerance, 1000);
        assert!(report.unresolved, "{report:?}");
    }

    /// Every interval's bounds hold against dense samples of `curve`.
    fn assert_intervals_hold(curve: &PreparedNurbs, intervals: &[Interval]) {
        for interval in intervals {
            let [t0, t1] = interval.range;
            let span = interval.span as usize;
            let at = |t: f64| curve.evaluation_in(span, t).unwrap();
            let (a, b, first) = (at(t0).point, at(t1).point, at(t0).first);
            for i in 0..=64 {
                let e = at(t0 + (t1 - t0) * f64::from(i) / 64.0);
                let d = sub(b, a);
                let s = if dot(d, d) > 0.0 {
                    (dot(sub(e.point, a), d) / dot(d, d)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let off = norm(sub(sub(e.point, a), scale(d, s)));
                assert!(off <= interval.chord, "{off} past {interval:?}");
                let cos = dot(e.first, first) / (norm(e.first) * norm(first));
                assert!(cos.min(1.0).acos() <= interval.angle + 1e-7, "{interval:?}");
            }
        }
    }

    #[test]
    fn slight_weights_and_heavy_ends_keep_honest_intervals() {
        // Weights of 2^-256: each half still turns by atan(1/8).
        let w = f64::from_bits(0x2FF0_0000_0000_0000);
        let slight = bezier(
            &[[0.0; 3], [8.0, 1.0, 0.0], [16.0, 0.0, 0.0]],
            Some(&[w, w, w]),
        );
        let tolerance = Tolerance {
            chord: 0.1,
            angle: 0.1,
        };
        let (intervals, report) = slight.adaptive_intervals(tolerance, 1000);
        assert!(!report.unresolved && !report.capped, "{report:?}");
        assert!(intervals.len() > 2);
        assert_intervals_hold(&slight, &intervals);
        // A straight line with one heavy end: uncertain at first, settled
        // by splitting.
        let heavy = bezier(
            &[[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]],
            Some(&[1.0, 1.0, 1.0, 1e8]),
        );
        let tolerance = Tolerance {
            chord: 1e-3,
            angle: 0.1,
        };
        let (intervals, report) = heavy.adaptive_intervals(tolerance, 1000);
        assert!(!report.unresolved && !report.capped, "{report:?}");
        assert_intervals_hold(&heavy, &intervals);
    }

    #[test]
    fn intervals_report_the_parameters_they_were_cut_at() {
        // A span one unit long at 2^52 has no parameter inside it: its one
        // interval cannot be split, and says so.
        let a = 2.0_f64.powi(52);
        let curve = Nurbs {
            order: 3,
            knots: &[a, a, a, a + 1.0, a + 1.0, a + 1.0],
            points: &[[0.0; 3], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
            weights: None,
        }
        .validate()
        .unwrap()
        .prepare();
        let tolerance = Tolerance {
            chord: 0.1,
            angle: f64::INFINITY,
        };
        let (intervals, report) = curve.adaptive_intervals(tolerance, 1000);
        assert!(report.unresolved, "{report:?}");
        assert_eq!(intervals.len(), 1);
        assert_eq!(intervals[0].range, [a, a + 1.0]);
        assert!(intervals[0].chord > 0.1);
    }
}
