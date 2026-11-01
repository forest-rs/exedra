// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Outward proof hulls of original bounded curves.
//!
//! This module owns rounding in native source evaluation and distance/support
//! bounds. It does not own closest-point search or a subdivision schedule.
//! Every arithmetic operation uses the intersection kernel's intervals. A
//! finite f64 input denotes its exact real value; overflow or lost positive
//! denominators are refused. Square roots use IEEE 754 square root followed by
//! outward rounding, as the crate's other arithmetic does.

use crate::{ValidNurbs, intersect::interval::I};
use alloc::vec::Vec;
use exedra_math::Real;

/// An original curve-parameter interval and its normalized traversal direction.
///
/// `parameters` use the defining curve's units. Enclosure query fractions use
/// `[0,1]` along this traversal. This plain input value performs no validation,
/// preparation or allocation; each operation retains its explicit input checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceRange {
    /// Increasing bounds in original curve-parameter units.
    pub parameters: [f64; 2],
    /// Whether normalized traversal opposes increasing original parameters.
    pub reversed: bool,
}
impl SourceRange {
    /// Forward traversal over original parameters `[0,1]`.
    pub const UNIT: Self = Self {
        parameters: [0.0, 1.0],
        reversed: false,
    };
}

/// Explicit record and work admission limits for source proof preparation.
/// This plain value has no defaults or hidden preparation work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparationLimits {
    /// Maximum original defining control records admitted before inspection.
    pub max_controls: usize,
    /// Maximum conservative preparation work bound, including metadata scans,
    /// control loads and rational blends. Query callback work is separate.
    pub max_work: u64,
}

/// Why a source enclosure could not be completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnclosureError {
    /// Invalid parameter bounds, fractions or nonfinite input.
    InvalidInput,
    /// Finite outward arithmetic or a strictly positive denominator was lost.
    NumericLimit,
    /// The next control load or rational blend was not admitted.
    WorkLimit,
    /// A preparation control limit or supplied scratch capacity is insufficient.
    Capacity,
}
impl core::fmt::Display for EnclosureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "native curve enclosure: {self:?}")
    }
}
impl core::error::Error for EnclosureError {}

fn checked(i: I) -> Result<I, EnclosureError> {
    i.valid().then_some(i).ok_or(EnclosureError::NumericLimit)
}
fn divide(a: I, b: I) -> Result<I, EnclosureError> {
    if !b.excludes_zero() {
        return Err(EnclosureError::NumericLimit);
    }
    let q = [a.lo / b.lo, a.lo / b.hi, a.hi / b.lo, a.hi / b.hi];
    if q.iter().any(|x| !x.is_finite()) {
        return Err(EnclosureError::NumericLimit);
    }
    checked(I {
        lo: q.into_iter().fold(f64::INFINITY, f64::min).next_down(),
        hi: q.into_iter().fold(f64::NEG_INFINITY, f64::max).next_up(),
    })
}
fn exact<const N: usize>(v: [f64; N]) -> Result<[I; N], EnclosureError> {
    if v.iter().any(|v| !v.is_finite()) {
        return Err(EnclosureError::InvalidInput);
    }
    Ok(v.map(I::p))
}
fn fractions(v: [f64; 2]) -> Result<I, EnclosureError> {
    (v[0].is_finite() && v[0] >= 0.0 && v[0] <= v[1] && v[1] <= 1.0)
        .then_some(I { lo: v[0], hi: v[1] })
        .ok_or(EnclosureError::InvalidInput)
}
fn parameter(range: [f64; 2], reversed: bool, fraction: I) -> Result<I, EnclosureError> {
    if !range.iter().all(|v| v.is_finite()) || range[0] >= range[1] {
        return Err(EnclosureError::InvalidInput);
    }
    let t = if reversed {
        (I::ONE - fraction)
            .intersect(I::UNIT)
            .ok_or(EnclosureError::NumericLimit)?
    } else {
        fraction
    };
    if t.lo == t.hi && (t.lo == 0.0 || t.lo == 1.0) {
        return Ok(I::p(range[usize::from(t.lo == 1.0)]));
    }
    checked(I::p(range[0]) + t * (I::p(range[1]) - I::p(range[0])))?
        .intersect(I {
            lo: range[0],
            hi: range[1],
        })
        .ok_or(EnclosureError::NumericLimit)
}
fn norm<const N: usize>(v: [I; N]) -> Result<I, EnclosureError> {
    let scale = v
        .iter()
        .map(|i| i.lo.abs().max(i.hi.abs()))
        .fold(0.0, f64::max);
    if scale == 0.0 {
        return Ok(I::ZERO);
    }
    if !scale.is_finite() {
        return Err(EnclosureError::NumericLimit);
    }
    let mut sum = I::ZERO;
    for i in v {
        sum = checked(sum + divide(i, I::p(scale))?.square())?;
    }
    checked(
        I {
            lo: <f64 as Real>::sqrt(sum.lo.max(0.0)).next_down().max(0.0),
            hi: <f64 as Real>::sqrt(sum.hi).next_up(),
        } * I::p(scale),
    )
}
fn blend(a: I, b: I, t: I) -> Result<I, EnclosureError> {
    // Every caller supplies a unit coefficient. Convex combinations of two
    // copies of one interval stay in that interval, including non-point boxes.
    // Explicit affine arithmetic gives this same intersection when finite;
    // the hull proof also avoids unnecessary overflow for large equal boxes.
    if a.lo == b.lo && a.hi == b.hi {
        fractions([t.lo, t.hi])?;
        return checked(a);
    }
    // The exact convex combination lies in the original union as well. The
    // intersection avoids dependency widening when t covers the whole unit.
    checked((I::ONE - t) * a + t * b)?
        .intersect(a.union(b))
        .ok_or(EnclosureError::NumericLimit)
}

/// An opaque box enclosing native positions, including evaluation roundoff.
#[derive(Clone, Copy, Debug)]
pub struct PointEnclosure<const N: usize = 3> {
    coordinates: [I; N],
}
impl<const N: usize> PointEnclosure<N> {
    /// An exact finite point.
    pub fn point(point: [f64; N]) -> Result<Self, EnclosureError> {
        Ok(Self {
            coordinates: exact(point)?,
        })
    }
    /// Coordinate lower and upper bounds.
    #[must_use]
    pub fn bounds(self) -> [[f64; N]; 2] {
        [
            self.coordinates.map(|i| i.lo),
            self.coordinates.map(|i| i.hi),
        ]
    }
    /// A representative inside the box; it is not claimed to lie on the curve.
    #[must_use]
    pub fn midpoint(self) -> [f64; N] {
        self.coordinates.map(|i| i.mid().clamp(i.lo, i.hi))
    }
    /// Include an additional representative without weakening enclosure safety.
    pub fn including(mut self, point: [f64; N]) -> Result<Self, EnclosureError> {
        for (i, p) in self.coordinates.iter_mut().zip(exact(point)?) {
            *i = i.union(p);
        }
        Ok(self)
    }
    /// Encloses every affine blend at fractions within the supplied unit range.
    pub fn affine_blend(self, other: Self, fraction: [f64; 2]) -> Result<Self, EnclosureError> {
        let t = fractions(fraction)?;
        let mut coordinates = [I::ZERO; N];
        for (k, i) in coordinates.iter_mut().enumerate() {
            *i = blend(self.coordinates[k], other.coordinates[k], t)?;
        }
        Ok(Self { coordinates })
    }
    /// Distance bounds from a target to every enclosed point, in model units.
    /// The lower bound applies to the whole box, and the upper to any point in it.
    pub fn distance_bounds(self, target: [f64; N]) -> Result<[f64; 2], EnclosureError> {
        let target = exact(target)?;
        let residual: [I; N] = core::array::from_fn(|k| self.coordinates[k] - target[k]);
        let d = norm(residual)?;
        Ok([d.lo.max(0.0), d.hi])
    }
    /// Distance bounds for every pair of points in the two enclosures.
    /// In particular, the upper bound covers disagreement between two exact
    /// native positions whose representatives have independent roundoff.
    pub fn distance_bounds_to(self, other: Self) -> Result<[f64; 2], EnclosureError> {
        let residual: [I; N] = core::array::from_fn(|k| self.coordinates[k] - other.coordinates[k]);
        let distance = norm(residual)?;
        Ok([distance.lo.max(0.0), distance.hi])
    }
}

/// An enclosure of `(curve-target)·direction`, retaining its direction norm.
/// Positive separation gives a lower Euclidean distance bound without assuming
/// the direction is a normal, or that a stationary point is globally nearest.
#[derive(Clone, Copy, Debug)]
pub struct ProjectionEnclosure<const N: usize = 3> {
    value: I,
    target: [f64; N],
    direction: [f64; N],
    length: I,
}
impl<const N: usize> ProjectionEnclosure<N> {
    /// Encloses convex combinations from two sources projected using the same
    /// target and direction. Mismatched projections are refused.
    pub fn affine_blend(self, other: Self, fraction: [f64; 2]) -> Result<Self, EnclosureError> {
        if self.target != other.target || self.direction != other.direction {
            return Err(EnclosureError::InvalidInput);
        }
        Ok(Self {
            value: blend(self.value, other.value, fractions(fraction)?)?,
            ..self
        })
    }
    /// A conservative Euclidean distance lower bound for all projected points.
    pub fn distance_lower(self) -> Result<f64, EnclosureError> {
        let separation = self.value.lo.max(-self.value.hi).max(0.0);
        if separation == 0.0 || self.length.hi == 0.0 {
            return Ok(0.0);
        }
        Ok(divide(I::p(separation), I::p(self.length.hi))?.lo.max(0.0))
    }
}
fn projected<const N: usize>(
    p: [I; N],
    target: [f64; N],
    direction: [f64; N],
) -> Result<I, EnclosureError> {
    let (target, direction) = (exact(target)?, exact(direction)?);
    let mut value = I::ZERO;
    for k in 0..N {
        value = checked(value + (p[k] - target[k]) * direction[k])?;
    }
    Ok(value)
}

/// Enclose an original affine source over a bounded normalized traversal.
pub fn line_enclosure<const N: usize>(
    origin: [f64; N],
    direction: [f64; N],
    source: SourceRange,
    fraction: [f64; 2],
) -> Result<PointEnclosure<N>, EnclosureError> {
    let SourceRange {
        parameters: range,
        reversed,
    } = source;
    let t = parameter(range, reversed, fractions(fraction)?)?;
    let (origin, direction) = (exact(origin)?, exact(direction)?);
    let mut coordinates = [I::ZERO; N];
    for k in 0..N {
        coordinates[k] = checked(origin[k] + t * direction[k])?;
    }
    Ok(PointEnclosure { coordinates })
}

/// Project an original affine source before interval evaluation, preserving
/// cancellation between its coordinate directions.
pub fn line_projection<const N: usize>(
    origin: [f64; N],
    axis: [f64; N],
    source: SourceRange,
    fraction: [f64; 2],
    target: [f64; N],
    direction: [f64; N],
) -> Result<ProjectionEnclosure<N>, EnclosureError> {
    let SourceRange {
        parameters: range,
        reversed,
    } = source;
    let t = parameter(range, reversed, fractions(fraction)?)?;
    let a = projected(exact(origin)?, target, direction)?;
    let b = projected(exact(axis)?, [0.0; N], direction)?;
    let value = checked(a + t * b)?;
    Ok(ProjectionEnclosure {
        value,
        target,
        direction,
        length: norm(exact(direction)?)?,
    })
}

fn restriction_parameter_error(
    range: [f64; 2],
    reversed: bool,
    domain: [f64; 2],
    child: [f64; 2],
    child_reversed: bool,
) -> Result<I, EnclosureError> {
    fractions(domain)?;
    if !child.iter().all(|v| v.is_finite())
        || child[0] >= child[1]
        || child[0] < range[0]
        || child[1] > range[1]
    {
        return Err(EnclosureError::InvalidInput);
    }
    let mut upper = 0.0_f64;
    for endpoint in 0..2 {
        let parent = parameter(range, reversed, I::p(domain[endpoint]))?;
        let actual = I::p(child[endpoint ^ usize::from(child_reversed)]);
        let delta = checked(parent - actual)?;
        upper = upper.max(delta.lo.abs()).max(delta.hi.abs());
    }
    Ok(I { lo: 0.0, hi: upper })
}

/// Bounds pointwise disagreement between two affine parameter maps of the same
/// original line. `direction` is its derivative in original parameter units;
/// translation cancels. The parent follows `source` over `domain` of normalized
/// U, while `child` describes its original-parameter traversal over the full
/// unit interval.
///
/// The exact parameter difference is affine, so its largest absolute value is
/// at an endpoint. Admission precedes the mapping and direction-norm test; no
/// allocation or unguarded floating error multiplier is used.
pub fn line_restriction_error<const N: usize>(
    direction: [f64; N],
    source: SourceRange,
    domain: [f64; 2],
    child: SourceRange,
    mut admit: impl FnMut() -> bool,
) -> Result<f64, EnclosureError> {
    let SourceRange {
        parameters: range,
        reversed,
    } = source;
    let SourceRange {
        parameters: child_range,
        reversed: child_reversed,
    } = child;
    if !admit() {
        return Err(EnclosureError::WorkLimit);
    }
    let direction = exact(direction)?;
    let error = restriction_parameter_error(range, reversed, domain, child_range, child_reversed)?;
    if domain == [0.0, 1.0] && range == child_range && reversed == child_reversed {
        return Ok(0.0);
    }
    Ok(checked(error * norm(direction)?)?.hi)
}

#[derive(Clone, Copy, Debug)]
struct RationalControl<const N: usize> {
    point: [I; N],
    weight: I,
}
impl<const N: usize> RationalControl<N> {
    const ZERO: Self = Self {
        point: [I::ZERO; N],
        weight: I::ZERO,
    };
}

/// Reusable de Casteljau rows; queries never grow this storage.
#[derive(Debug)]
pub struct EnclosureScratch<const N: usize = 3> {
    row: Vec<RationalControl<N>>,
    left: Vec<RationalControl<N>>,
    right: Vec<RationalControl<N>>,
}
impl<const N: usize> EnclosureScratch<N> {
    /// Allocate three rows for sources of at most this order.
    pub fn new(max_order: usize) -> Result<Self, EnclosureError> {
        fn row<const N: usize>(n: usize) -> Result<Vec<RationalControl<N>>, EnclosureError> {
            let mut row = Vec::new();
            row.try_reserve_exact(n)
                .map_err(|_| EnclosureError::Capacity)?;
            row.resize(n, RationalControl::ZERO);
            Ok(row)
        }
        Ok(Self {
            row: row(max_order)?,
            left: row(max_order)?,
            right: row(max_order)?,
        })
    }
    /// Retained payload excluding allocator headers.
    #[must_use]
    pub fn capacity_bytes(&self) -> usize {
        (self.row.capacity() + self.left.capacity() + self.right.capacity())
            * size_of::<RationalControl<N>>()
    }
    fn split(
        &mut self,
        n: usize,
        t: I,
        weights: I,
        admit: &mut impl FnMut() -> bool,
    ) -> Result<(), EnclosureError> {
        self.left[0] = self.row[0];
        self.right[n - 1] = self.row[n - 1];
        for level in 1..n {
            let width = n - level;
            for j in 0..width {
                if !admit() {
                    return Err(EnclosureError::WorkLimit);
                }
                self.row[j] = rational_blend(self.row[j], self.row[j + 1], t, weights)?;
            }
            self.left[level] = self.row[0];
            self.right[width - 1] = self.row[width - 1];
        }
        Ok(())
    }
}
fn rational_blend<const N: usize>(
    a: RationalControl<N>,
    b: RationalControl<N>,
    alpha: I,
    weights: I,
) -> Result<RationalControl<N>, EnclosureError> {
    if alpha.lo == 0. && alpha.hi == 0. {
        return Ok(a);
    }
    if alpha.lo == 1. && alpha.hi == 1. {
        return Ok(b);
    }
    let weight = blend(a.weight, b.weight, alpha)?
        .intersect(weights)
        .ok_or(EnclosureError::NumericLimit)?;
    let beta = divide(alpha * b.weight, weight)?
        .intersect(I::UNIT)
        .ok_or(EnclosureError::NumericLimit)?;
    let mut point = [I::ZERO; N];
    for (c, value) in point.iter_mut().enumerate() {
        *value = blend(a.point[c], b.point[c], beta)?;
    }
    Ok(RationalControl { point, weight })
}
fn alpha(t: I, left: f64, right: f64) -> Result<I, EnclosureError> {
    if t.lo == t.hi && t.lo == left {
        return Ok(I::ZERO);
    }
    if t.lo == t.hi && t.lo == right {
        return Ok(I::ONE);
    }
    divide(t - I::p(left), I::p(right) - I::p(left))?
        .intersect(I::UNIT)
        .ok_or(EnclosureError::NumericLimit)
}
fn preparation_requirements<const N: usize>(
    source: ValidNurbs<'_, N>,
) -> Result<(u64, usize), EnclosureError> {
    let curve = source.curve();
    let order = usize::try_from(curve.order).expect("validated order fits controls");
    let spans = (order - 1..curve.points.len())
        .filter(|&k| curve.knots[k] < curve.knots[k + 1])
        .count();
    let n = u64::from(curve.order);
    let metadata = u64::try_from(curve.points.len())
        .map_err(|_| EnclosureError::Capacity)?
        .checked_mul(2)
        .ok_or(EnclosureError::Capacity)?;
    let per = n
        .checked_add(n.checked_mul(n - 1).ok_or(EnclosureError::Capacity)? / 2)
        .ok_or(EnclosureError::Capacity)?;
    let work = metadata
        .checked_add(
            u64::try_from(spans)
                .map_err(|_| EnclosureError::Capacity)?
                .checked_mul(n)
                .and_then(|v| v.checked_mul(per))
                .ok_or(EnclosureError::Capacity)?,
        )
        .ok_or(EnclosureError::Capacity)?;
    Ok((work, spans))
}
/// Immutable outward control hulls of the ORIGINAL bounded NURBS source.
///
/// Original knots and controls define every proof. Preparation evaluates the
/// original spline's blossom with outward arithmetic to enclose each exact
/// rational Bezier control; it never substitutes rounded extracted geometry.
/// Queries restrict these proof hulls by outward de Casteljau and visit every
/// touched original knot span. A positive common weight scaling changes no
/// exact point. The source range and reversal remain authoritative.
#[derive(Debug)]
pub struct PreparedNurbsEnclosure<'a, const N: usize = 3> {
    source: ValidNurbs<'a, N>,
    range: [f64; 2],
    reversed: bool,
    controls: Vec<RationalControl<N>>,
    spans: Vec<usize>,
    weights: I,
    preparation_work_bound: u64,
}
impl<'a, const N: usize> PreparedNurbsEnclosure<'a, N> {
    /// Required conservative preparation-work bound from original source
    /// metadata, without allocating or evaluating any control hulls. This is one
    /// linear knot-inspection pass; callers should admit source record counts
    /// before requesting it and account for an additional eligibility pass if
    /// they also call [`Self::prepare`]. The returned formula is the SAME bound
    /// enforced and reported by preparation, not measured elapsed work.
    pub fn preparation_work_required(source: ValidNurbs<'a, N>) -> Result<u64, EnclosureError> {
        Ok(preparation_requirements(source)?.0)
    }
    /// Prepare exact-source proof hulls. Both original control count and a
    /// conservative preparation-work bound are checked before control loads,
    /// blossom evaluation or allocating the output hulls. Work counts source
    /// metadata/weight scans, control loads and rational blends. The temporary
    /// row is one source order; no per-control heap allocation occurs.
    pub fn prepare(
        source: ValidNurbs<'a, N>,
        traversal: SourceRange,
        limits: PreparationLimits,
    ) -> Result<Self, EnclosureError> {
        let SourceRange {
            parameters: range,
            reversed,
        } = traversal;
        let PreparationLimits {
            max_controls,
            max_work,
        } = limits;
        let curve = source.curve();
        let domain = source.domain();
        if range[0] < domain[0] || range[1] > domain[1] {
            return Err(EnclosureError::InvalidInput);
        }
        parameter(range, reversed, I::UNIT)?;
        let count = curve.points.len();
        if count > max_controls {
            return Err(EnclosureError::Capacity);
        }
        let count64 = u64::try_from(count).map_err(|_| EnclosureError::Capacity)?;
        let metadata = count64.checked_mul(2).ok_or(EnclosureError::Capacity)?;
        if metadata > max_work {
            return Err(EnclosureError::WorkLimit);
        }
        let order = usize::try_from(curve.order).expect("validated order fits controls");
        let (work, span_count) = preparation_requirements(source)?;
        if work > max_work {
            return Err(EnclosureError::WorkLimit);
        }
        let mut spans = Vec::new();
        spans
            .try_reserve_exact(span_count)
            .map_err(|_| EnclosureError::Capacity)?;
        let (mut minimum, mut scale) = (f64::INFINITY, 0.0_f64);
        // Combined source-record scan: weights and span indices together.
        // Alongside the first metadata count pass this remains bounded by 2*N.
        for k in 0..count {
            let weight = curve.weights.map_or(1.0, |w| w[k]);
            minimum = minimum.min(weight);
            scale = scale.max(weight);
            if k >= order - 1 && curve.knots[k] < curve.knots[k + 1] {
                spans.push(k);
            }
        }
        let weights = I {
            lo: divide(I::p(minimum), I::p(scale))?.lo,
            hi: 1.,
        };
        if weights.lo <= 0. {
            return Err(EnclosureError::NumericLimit);
        }
        let size = spans
            .len()
            .checked_mul(order)
            .ok_or(EnclosureError::Capacity)?;
        let mut controls = Vec::new();
        controls
            .try_reserve_exact(size)
            .map_err(|_| EnclosureError::Capacity)?;
        let mut scratch = EnclosureScratch::<N>::new(order)?;
        for &k in &spans {
            for index in 0..order {
                for j in 0..order {
                    let c = k + j + 1 - order;
                    let w = curve.weights.map_or(1., |w| w[c]);
                    let weight = if w == scale {
                        I::ONE
                    } else {
                        divide(I::p(w), I::p(scale))?
                    };
                    scratch.row[j] = RationalControl {
                        point: exact(curve.points[c])?,
                        weight,
                    };
                }
                for r in 1..order {
                    let t = if r < order - index {
                        curve.knots[k]
                    } else {
                        curve.knots[k + 1]
                    };
                    for j in (r..order).rev() {
                        let a = alpha(
                            I::p(t),
                            curve.knots[j + k + 1 - order],
                            curve.knots[j + k + 1 - r],
                        )?;
                        scratch.row[j] =
                            rational_blend(scratch.row[j - 1], scratch.row[j], a, weights)?;
                    }
                }
                controls.push(scratch.row[order - 1]);
            }
        }
        Ok(Self {
            source,
            range,
            reversed,
            controls,
            spans,
            weights,
            preparation_work_bound: work,
        })
    }
    /// Source order, determining scratch row capacity.
    #[must_use]
    pub fn order(&self) -> usize {
        usize::try_from(self.source.curve().order).expect("validated order fits controls")
    }
    /// Original defining control count, rather than proof-hull record count.
    #[must_use]
    pub fn controls(&self) -> usize {
        self.source.curve().points.len()
    }
    /// Admitted upper bound on preparation scans, control loads and blends.
    #[must_use]
    pub const fn preparation_work_bound(&self) -> u64 {
        self.preparation_work_bound
    }
    /// Retained payload excluding borrowed geometry and allocator headers.
    #[must_use]
    pub fn capacity_bytes(&self) -> usize {
        self.controls.capacity() * size_of::<RationalControl<N>>()
            + self.spans.capacity() * size_of::<usize>()
    }
    /// A rigorous bound on `|dC/du|` over this bounded source's NORMALIZED unit
    /// traversal. Reversal changes no speed. This is a Lipschitz bound on
    /// positions, including continuous knot corners. Full-multiplicity knots
    /// touching the bounded range permit jumps and are refused, even if their
    /// particular endpoints coincide.
    ///
    /// On each original knot span, with outward rational Bezier proof controls
    /// `(P_i,w_i)`, choose a fixed center `c`, let `R=max|P_i-c|`, and bound
    /// `N'=degree*max|w_(i+1)(P_(i+1)-c)-w_i(P_i-c)|` and
    /// `W'=degree*max|w_(i+1)-w_i|`. Then
    /// `|C'| <= (N' + R*W')/min(w_i)` in that span's unit parameter. Scale by
    /// the original source range width divided by the original knot span width.
    /// All differences, norms, products and divisions are outward-rounded.
    ///
    /// Admission precedes the span lookup, each span visit, each control norm
    /// and each adjacent-control test. The operation allocates nothing.
    pub fn speed_upper(&self, mut admit: impl FnMut() -> bool) -> Result<f64, EnclosureError> {
        if !admit() {
            return Err(EnclosureError::WorkLimit);
        }
        let curve = self.source.curve();
        let order = self.order();
        let degree = I::p(f64::from(curve.order - 1));
        let source_width = checked(I::p(self.range[1]) - I::p(self.range[0]))?;
        let first = self
            .spans
            .partition_point(|&k| curve.knots[k + 1] <= self.range[0]);
        let mut maximum = 0.0_f64;
        for (span, &k) in self.spans.iter().enumerate().skip(first) {
            if curve.knots[k] >= self.range[1] {
                break;
            }
            if !admit() {
                return Err(EnclosureError::WorkLimit);
            }
            // ValidNurbs permits jumps. A finite span derivative does not
            // provide a Lipschitz bound across or at such a discontinuity.
            for (left, right) in [
                span.checked_sub(1).map(|p| (p, span)),
                (span + 1 < self.spans.len()).then_some((span, span + 1)),
            ]
            .into_iter()
            .flatten()
            {
                let knot = curve.knots[self.spans[right]];
                if self.range[0] <= knot
                    && knot <= self.range[1]
                    && self.spans[right] - self.spans[left] >= order
                {
                    return Err(EnclosureError::InvalidInput);
                }
            }
            let controls = &self.controls[span * order..(span + 1) * order];
            let center = controls[0].point.map(|i| I::p(i.mid().clamp(i.lo, i.hi)));
            let mut radius = 0.0_f64;
            let mut minimum = f64::INFINITY;
            for p in controls {
                if !admit() {
                    return Err(EnclosureError::WorkLimit);
                }
                let residual = core::array::from_fn::<_, N, _>(|c| p.point[c] - center[c]);
                radius = radius.max(norm(residual)?.hi);
                minimum = minimum.min(p.weight.lo);
            }
            let mut numerator = 0.0_f64;
            let mut denominator = 0.0_f64;
            for pair in controls.windows(2) {
                if !admit() {
                    return Err(EnclosureError::WorkLimit);
                }
                let [a, b] = [pair[0], pair[1]];
                let residual = core::array::from_fn::<_, N, _>(|c| {
                    (b.point[c] - center[c]) * b.weight - (a.point[c] - center[c]) * a.weight
                });
                numerator = numerator.max(norm(residual)?.hi);
                let delta = checked(b.weight - a.weight)?;
                denominator = denominator.max(delta.lo.abs()).max(delta.hi.abs());
            }
            let native = divide(
                checked(degree * (I::p(numerator) + I::p(radius) * I::p(denominator)))?,
                I::p(minimum),
            )?;
            let span_width = checked(I::p(curve.knots[k + 1]) - I::p(curve.knots[k]))?;
            let scale = divide(source_width, span_width)?;
            maximum = maximum.max(checked(native * scale)?.hi);
        }
        Ok(maximum)
    }
    /// Bounds full pointwise position disagreement between the original source
    /// restricted to a normalized `domain` and a child traversal over the
    /// original-parameter `child` traversal.
    ///
    /// Their exact parameter maps are affine, so endpoint disagreement bounds
    /// the whole parameter interval. Convert that displacement to normalized
    /// source units and multiply by [`Self::speed_upper`]. Both paths refer to
    /// this SAME defining source curve; this is not a comparison of arbitrary
    /// curves or of their representative floating evaluators. The identity full
    /// unit map returns exactly zero without speed work. Admission precedes the
    /// mapping test and every subsequent speed-bound work unit.
    pub fn restriction_error(
        &self,
        domain: [f64; 2],
        child: SourceRange,
        mut admit: impl FnMut() -> bool,
    ) -> Result<f64, EnclosureError> {
        let SourceRange {
            parameters: child_range,
            reversed: child_reversed,
        } = child;
        if !admit() {
            return Err(EnclosureError::WorkLimit);
        }
        let error = restriction_parameter_error(
            self.range,
            self.reversed,
            domain,
            child_range,
            child_reversed,
        )?;
        if domain == [0.0, 1.0] && self.range == child_range && self.reversed == child_reversed {
            return Ok(0.0);
        }
        let width = checked(I::p(self.range[1]) - I::p(self.range[0]))?;
        let normalized = divide(error, width)?;
        let speed = self.speed_upper(&mut admit)?;
        Ok(checked(normalized * I::p(speed))?.hi)
    }
    /// Enclose a normalized interval. Admission precedes each control load,
    /// rational blend and hull visit; false retains no partial answer.
    pub fn enclose(
        &self,
        fraction: [f64; 2],
        scratch: &mut EnclosureScratch<N>,
        mut admit: impl FnMut() -> bool,
    ) -> Result<PointEnclosure<N>, EnclosureError> {
        Ok(PointEnclosure {
            coordinates: self.evaluate(fraction, scratch, None, &mut admit)?,
        })
    }
    /// Enclose a scalar projection BEFORE subdivision, preserving cancellation
    /// between coordinates. This is a support bound, not a nearest-point claim.
    pub fn project(
        &self,
        fraction: [f64; 2],
        target: [f64; N],
        direction: [f64; N],
        scratch: &mut EnclosureScratch<N>,
        mut admit: impl FnMut() -> bool,
    ) -> Result<ProjectionEnclosure<N>, EnclosureError> {
        let length = norm(exact(direction)?)?;
        exact(target)?;
        let value = self.evaluate(fraction, scratch, Some((target, direction)), &mut admit)?[0];
        Ok(ProjectionEnclosure {
            value,
            target,
            direction,
            length,
        })
    }
    fn evaluate(
        &self,
        fraction: [f64; 2],
        scratch: &mut EnclosureScratch<N>,
        projection: Option<([f64; N], [f64; N])>,
        admit: &mut impl FnMut() -> bool,
    ) -> Result<[I; N], EnclosureError> {
        let parameter = parameter(self.range, self.reversed, fractions(fraction)?)?;
        let order = self.order();
        if N == 0 || scratch.row.len() < order {
            return Err(EnclosureError::Capacity);
        }
        let curve = self.source.curve();
        let mut out: Option<[I; N]> = None;
        let start = self
            .spans
            .partition_point(|&k| curve.knots[k + 1] < parameter.lo);
        for (span, &k) in self.spans.iter().enumerate().skip(start) {
            if curve.knots[k] > parameter.hi {
                break;
            }
            let Some(t) = parameter.intersect(I {
                lo: curve.knots[k],
                hi: curve.knots[k + 1],
            }) else {
                continue;
            };
            let local = alpha(t, curve.knots[k], curve.knots[k + 1])?;
            for j in 0..order {
                if !admit() {
                    return Err(EnclosureError::WorkLimit);
                }
                let mut p = self.controls[span * order + j];
                if let Some((target, direction)) = projection {
                    let value = projected(p.point, target, direction)?;
                    p.point = [I::ZERO; N];
                    p.point[0] = value;
                }
                scratch.row[j] = p;
            }
            if local.lo > 0. {
                scratch.split(order, I::p(local.lo), self.weights, admit)?;
                for j in 0..order {
                    if !admit() {
                        return Err(EnclosureError::WorkLimit);
                    }
                    scratch.row[j] = scratch.right[j];
                }
            }
            let relative = if local.lo == 1. {
                I::ZERO
            } else {
                divide(I::p(local.hi) - I::p(local.lo), I::ONE - I::p(local.lo))?
                    .intersect(I::UNIT)
                    .ok_or(EnclosureError::NumericLimit)?
            };
            if local.hi < 1. || local.lo == 1. {
                scratch.split(order, relative, self.weights, admit)?;
                for j in 0..order {
                    if !admit() {
                        return Err(EnclosureError::WorkLimit);
                    }
                    scratch.row[j] = scratch.left[j];
                }
            }
            for p in &scratch.row[..order] {
                if !admit() {
                    return Err(EnclosureError::WorkLimit);
                }
                out = Some(match out {
                    None => p.point,
                    Some(previous) => core::array::from_fn(|c| previous[c].union(p.point[c])),
                });
            }
        }
        out.ok_or(EnclosureError::NumericLimit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NurbsCurve;
    use alloc::vec;

    #[test]
    fn normalized_rational_speed_bound_uses_positive_original_weights_and_centering() {
        for offset in [0.0, 1e12] {
            let curve = NurbsCurve::new(
                2,
                vec![2., 2., 5., 5.],
                vec![[offset + 1., 0., 0.], [offset + 5., 0., 0.]],
                Some(vec![1., 4.]),
            )
            .unwrap();
            for reversed in [false, true] {
                let prepared = PreparedNurbsEnclosure::prepare(
                    curve.view(),
                    SourceRange {
                        parameters: [2., 5.],
                        reversed,
                    },
                    PreparationLimits {
                        max_controls: 2,
                        max_work: 100,
                    },
                )
                .unwrap();
                let bound = prepared.speed_upper(|| true).unwrap();
                // In normalized U: C=(1+19u)/(1+3u), C'=16/(1+3u)^2.
                // The proved control bound is 28, and independent translation
                // changes no exact derivative or source weights.
                assert!((16.0..=28.0 + 1e-10).contains(&bound), "{bound:?}");
                for i in 0..=128 {
                    let t = 2. + 3. * f64::from(i) / 128.;
                    let e = curve.view().evaluation(t);
                    assert!(e.first[0].abs() * 3. <= bound);
                }
            }
        }
    }

    #[test]
    fn speed_covers_continuous_knot_corners_but_refuses_source_jumps() {
        let curve = NurbsCurve::new(
            2,
            vec![0., 0., 0.5, 1., 1.],
            vec![[0., 0., 0.], [1., 1., 0.], [2., 0., 0.]],
            None,
        )
        .unwrap();
        let prepared = PreparedNurbsEnclosure::prepare(
            curve.view(),
            SourceRange {
                parameters: [0., 1.],
                reversed: true,
            },
            PreparationLimits {
                max_controls: 3,
                max_work: 1000,
            },
        )
        .unwrap();
        let bound = prepared.speed_upper(|| true).unwrap();
        let exact = 2. * <f64 as Real>::sqrt(2.);
        assert!(bound >= exact && bound <= exact + 1e-12);
        let jump = NurbsCurve::new(
            2,
            vec![0., 0., 0.5, 0.5, 1., 1.],
            vec![[0., 0., 0.], [1., 0., 0.], [3., 0., 0.], [4., 0., 0.]],
            None,
        )
        .unwrap();
        for range in [[0., 1.], [0., 0.5], [0.5, 1.]] {
            let prepared = PreparedNurbsEnclosure::prepare(
                jump.view(),
                SourceRange {
                    parameters: range,
                    reversed: false,
                },
                PreparationLimits {
                    max_controls: 4,
                    max_work: 1000,
                },
            )
            .unwrap();
            assert!(matches!(
                prepared.speed_upper(|| true),
                Err(EnclosureError::InvalidInput)
            ));
        }
        let prepared = PreparedNurbsEnclosure::prepare(
            jump.view(),
            SourceRange {
                parameters: [0., 0.4],
                reversed: false,
            },
            PreparationLimits {
                max_controls: 4,
                max_work: 1000,
            },
        )
        .unwrap();
        assert!(prepared.speed_upper(|| true).unwrap() >= 0.8);
    }

    #[test]
    fn affine_restriction_disagreement_is_bounded_at_endpoints_in_original_units() {
        let line = line_restriction_error(
            [3., 4., 0.],
            SourceRange {
                parameters: [2., 6.],
                reversed: false,
            },
            [0.25, 0.75],
            SourceRange {
                parameters: [3.125, 5.125],
                reversed: false,
            },
            || true,
        )
        .unwrap();
        assert!((0.625..=0.625 + 1e-12).contains(&line));
        let curve = NurbsCurve::new(
            2,
            vec![2., 2., 5., 5.],
            vec![[1., 0., 0.], [5., 0., 0.]],
            Some(vec![1., 4.]),
        )
        .unwrap();
        let prepared = PreparedNurbsEnclosure::prepare(
            curve.view(),
            SourceRange {
                parameters: [2., 5.],
                reversed: true,
            },
            PreparationLimits {
                max_controls: 2,
                max_work: 100,
            },
        )
        .unwrap();
        let same = prepared
            .restriction_error(
                [0.25, 0.75],
                SourceRange {
                    parameters: [2.75, 4.25],
                    reversed: true,
                },
                || true,
            )
            .unwrap();
        assert!(same < 1e-12, "{same:?}");
        let backwards = prepared
            .restriction_error(
                [0.25, 0.75],
                SourceRange {
                    parameters: [2.75, 4.25],
                    reversed: false,
                },
                || true,
            )
            .unwrap();
        assert!(backwards > 1.);
        let shifted = prepared
            .restriction_error(
                [0.25, 0.75],
                SourceRange {
                    parameters: [2.875, 4.375],
                    reversed: true,
                },
                || true,
            )
            .unwrap();
        for i in 0..=128 {
            let t = f64::from(i) / 128.;
            let a = curve.view().evaluate(5. - 3. * (0.25 + 0.5 * t));
            let b = curve.view().evaluate(4.375 - 1.5 * t);
            assert!((a[0] - b[0]).abs() <= shifted);
        }
        let mut count = 0;
        let identity = prepared
            .restriction_error(
                [0., 1.],
                SourceRange {
                    parameters: [2., 5.],
                    reversed: true,
                },
                || {
                    count += 1;
                    true
                },
            )
            .unwrap();
        assert_eq!(identity, 0.);
        assert_eq!(count, 1);
        assert!(matches!(
            prepared.restriction_error(
                [0., 1.],
                SourceRange {
                    parameters: [1., 5.],
                    reversed: true
                },
                || true
            ),
            Err(EnclosureError::InvalidInput)
        ));
    }

    #[test]
    fn speed_and_mapping_refuse_unresolved_denominators_and_preserve_underflow_allowance() {
        let curve = NurbsCurve::new(
            2,
            vec![0., 0., 1e-320, 1e-320],
            vec![[0., 0., 0.], [1e-200, 0., 0.]],
            None,
        )
        .unwrap();
        let prepared = PreparedNurbsEnclosure::prepare(
            curve.view(),
            SourceRange {
                parameters: [0., 1e-320],
                reversed: false,
            },
            PreparationLimits {
                max_controls: 2,
                max_work: 100,
            },
        )
        .unwrap();
        let bound = prepared
            .restriction_error(
                [0.1, 0.9],
                SourceRange {
                    parameters: [1e-321, 9e-321],
                    reversed: false,
                },
                || true,
            )
            .unwrap();
        assert!(bound > 0. && bound.is_finite());
        assert!(
            line_restriction_error(
                [1., 0., 0.],
                SourceRange {
                    parameters: [0., 1e-320],
                    reversed: false
                },
                [0.1, 0.9],
                SourceRange {
                    parameters: [1e-321, 9e-321],
                    reversed: false
                },
                || true
            )
            .unwrap()
                > 0.
        );
        assert!(matches!(
            prepared.speed_upper(|| false),
            Err(EnclosureError::WorkLimit)
        ));
        assert!(matches!(
            prepared.restriction_error(
                [0.1, 0.9],
                SourceRange {
                    parameters: [1e-321, 9e-321],
                    reversed: false
                },
                || false
            ),
            Err(EnclosureError::WorkLimit)
        ));
        let tiny = f64::from_bits(1);
        let curve = NurbsCurve::new(
            2,
            vec![0., 0., tiny, tiny],
            vec![[0., 0., 0.], [1e-200, 0., 0.]],
            None,
        )
        .unwrap();
        let prepared = PreparedNurbsEnclosure::prepare(
            curve.view(),
            SourceRange {
                parameters: [0., tiny],
                reversed: false,
            },
            PreparationLimits {
                max_controls: 2,
                max_work: 100,
            },
        )
        .unwrap();
        assert!(matches!(
            prepared.speed_upper(|| true),
            Err(EnclosureError::NumericLimit)
        ));
    }

    #[test]
    fn point_to_point_enclosures_bound_independent_endpoint_rounding() {
        let a = line_enclosure([1e10, 0., 0.], [3., 4., 0.], SourceRange::UNIT, [1., 1.]).unwrap();
        let b = PointEnclosure::point([1e10 + 3., 4., 0.]).unwrap();
        let bounds = a.distance_bounds_to(b).unwrap();
        assert_eq!(bounds[0], 0.);
        assert!(bounds[1] > 0. && bounds[1] < 1e-4);
        let distance = PointEnclosure::point([0., 0., 0.])
            .unwrap()
            .distance_bounds_to(PointEnclosure::point([3., 4., 0.]).unwrap())
            .unwrap();
        assert!(distance[0] <= 5. && distance[1] >= 5.);
    }

    #[test]
    fn identical_interval_blends_keep_the_exact_convex_hull() {
        for a in [
            I::ZERO,
            I::p(f64::from_bits(1)),
            I { lo: -3., hi: 7. },
            I {
                lo: 1e100,
                hi: 2e100,
            },
        ] {
            for t in [I::ZERO, I::ONE, I::p(0.37), I::UNIT] {
                let exhaustive = checked((I::ONE - t) * a + t * a)
                    .unwrap()
                    .intersect(a)
                    .unwrap();
                let found = blend(a, a, t).unwrap();
                assert_eq!([found.lo, found.hi], [a.lo, a.hi]);
                assert_eq!([found.lo, found.hi], [exhaustive.lo, exhaustive.hi]);
            }
        }
        // The shortcut must retain refusal of invalid interval storage.
        let invalid = I { lo: 2., hi: 1. };
        assert!(matches!(
            blend(invalid, invalid, I::UNIT),
            Err(EnclosureError::NumericLimit)
        ));
        let large = I {
            lo: f64::MAX * 0.5,
            hi: f64::MAX,
        };
        assert!(checked((I::ONE - I::UNIT) * large + I::UNIT * large).is_err());
        let found = blend(large, large, I::UNIT).unwrap();
        assert_eq!([found.lo, found.hi], [large.lo, large.hi]);
        for t in [
            I::p(f64::NAN),
            I::p(f64::INFINITY),
            I::p(-0.1),
            I { lo: 0.8, hi: 0.2 },
            I { lo: 0., hi: 1.1 },
        ] {
            assert!(matches!(
                blend(I::ZERO, I::ZERO, t),
                Err(EnclosureError::InvalidInput)
            ));
        }
    }

    #[test]
    fn representatives_stay_inside_subnormal_large_and_opposite_sign_boxes() {
        for value in [f64::from_bits(1), -f64::from_bits(1), 1e300, -1e300] {
            let box_ = PointEnclosure::point([value; 3]).unwrap();
            assert_eq!(box_.midpoint(), [value; 3]);
            assert!(box_.distance_bounds(box_.midpoint()).unwrap()[1] >= 0.);
        }
        let a = PointEnclosure::point([-1e300, 1e300, 0.]).unwrap();
        let b = PointEnclosure::point([1e300, -1e300, 0.]).unwrap();
        let box_ = a.affine_blend(b, [0., 1.]).unwrap();
        let p = box_.midpoint();
        let [lo, hi] = box_.bounds();
        for k in 0..3 {
            assert!(lo[k] <= p[k] && p[k] <= hi[k]);
        }
    }

    #[test]
    fn preparation_work_and_legal_extreme_weights_are_explicit() {
        let curve = NurbsCurve::new(
            2,
            vec![0., 0., 1., 1.],
            vec![[0., 0., 0.], [1., 1., 1.]],
            Some(vec![2.0_f64.powi(-256), 2.0_f64.powi(256)]),
        )
        .unwrap();
        assert!(matches!(
            PreparedNurbsEnclosure::prepare(
                curve.view(),
                SourceRange::UNIT,
                PreparationLimits {
                    max_controls: 2,
                    max_work: 0
                }
            ),
            Err(EnclosureError::WorkLimit)
        ));
        let prepared = PreparedNurbsEnclosure::prepare(
            curve.view(),
            SourceRange::UNIT,
            PreparationLimits {
                max_controls: 2,
                max_work: 100,
            },
        )
        .unwrap();
        assert!(prepared.preparation_work_bound() <= 100);
        let mut scratch = EnclosureScratch::new(2).unwrap();
        let bounds = prepared
            .enclose([0.25, 0.75], &mut scratch, || true)
            .unwrap()
            .bounds();
        assert!(bounds.iter().flatten().all(|v| v.is_finite()));
        assert!(bounds[0][0] <= 1. && bounds[1][0] >= 1.);
        assert!(
            NurbsCurve::new(
                2,
                vec![0., 0., 1., 1.],
                vec![[0.; 3], [1.; 3]],
                Some(vec![1e-300, 1e300])
            )
            .is_err()
        );
    }

    #[test]
    fn original_span_crossings_ranges_and_reversal_enclose_native_evaluations() {
        let curve = NurbsCurve::new(
            4,
            vec![2., 2., 2., 2., 3., 4., 5., 5., 5., 5.],
            vec![
                [0., 0., 0.],
                [1., 2., 1.],
                [2., -1., 2.],
                [3., 3., 1.],
                [4., -2., 0.],
                [5., 1., 3.],
            ],
            Some(vec![1., 0.5, 2., 1., 0.75, 1.]),
        )
        .unwrap();
        for range in [[2., 5.], [2.5, 4.5], [3., 4.]] {
            for reversed in [false, true] {
                let prepared = PreparedNurbsEnclosure::prepare(
                    curve.view(),
                    SourceRange {
                        parameters: range,
                        reversed,
                    },
                    PreparationLimits {
                        max_controls: 6,
                        max_work: 100_000,
                    },
                )
                .unwrap();
                let mut scratch = EnclosureScratch::new(4).unwrap();
                let capacity = scratch.capacity_bytes();
                for fraction in [[0., 0.], [0.2, 0.8], [0., 1.], [0.5, 0.5], [1., 1.]] {
                    let enclosure = prepared
                        .enclose(fraction, &mut scratch, || true)
                        .unwrap()
                        .bounds();
                    for i in 0..=128 {
                        let u = fraction[0] + (fraction[1] - fraction[0]) * f64::from(i) / 128.;
                        let t =
                            range[0] + (range[1] - range[0]) * if reversed { 1. - u } else { u };
                        let p = curve.view().evaluate(t);
                        for k in 0..3 {
                            assert!(
                                p[k] >= enclosure[0][k] && p[k] <= enclosure[1][k],
                                "{range:?} {reversed} {fraction:?} {p:?} {enclosure:?}"
                            );
                        }
                    }
                }
                assert_eq!(scratch.capacity_bytes(), capacity);
            }
        }
    }

    #[test]
    fn rational_linear_oracle_and_directional_cancellation_are_enclosed() {
        let curve = NurbsCurve::new(
            2,
            vec![2., 2., 5., 5.],
            vec![[1., 1., 0.], [5., 5., 0.]],
            Some(vec![1., 4.]),
        )
        .unwrap();
        let prepared = PreparedNurbsEnclosure::prepare(
            curve.view(),
            SourceRange {
                parameters: [2., 5.],
                reversed: false,
            },
            PreparationLimits {
                max_controls: 2,
                max_work: 100_000,
            },
        )
        .unwrap();
        let mut scratch = EnclosureScratch::new(2).unwrap();
        let point = prepared
            .enclose([0.25, 0.25], &mut scratch, || true)
            .unwrap();
        let bounds = point.bounds();
        let exact = 23. / 7.;
        assert!(bounds[0][0] <= exact && exact <= bounds[1][0]);
        let projection = prepared
            .project([0., 1.], [0., 1., 0.], [1., -1., 0.], &mut scratch, || true)
            .unwrap();
        let lower = projection.distance_lower().unwrap();
        assert!(lower > 0.7 && lower <= <f64 as Real>::sqrt(0.5));
        assert_eq!(
            point.affine_blend(point, [0., 1.]).unwrap().bounds(),
            bounds
        );
    }

    #[test]
    fn controls_and_blends_are_charged_before_work_and_capacity_is_checked() {
        let curve = NurbsCurve::new(
            3,
            vec![0., 0., 0., 1., 1., 1.],
            vec![[0.; 3], [1.; 3], [2.; 3]],
            None,
        )
        .unwrap();
        assert!(matches!(
            PreparedNurbsEnclosure::prepare(
                curve.view(),
                SourceRange::UNIT,
                PreparationLimits {
                    max_controls: 2,
                    max_work: 100_000
                }
            ),
            Err(EnclosureError::Capacity)
        ));
        let prepared = PreparedNurbsEnclosure::prepare(
            curve.view(),
            SourceRange::UNIT,
            PreparationLimits {
                max_controls: 3,
                max_work: 100_000,
            },
        )
        .unwrap();
        let mut scratch = EnclosureScratch::new(3).unwrap();
        for maximum in 0..6 {
            let mut count = 0;
            let result = prepared.enclose([0., 1.], &mut scratch, || {
                if count == maximum {
                    false
                } else {
                    count += 1;
                    true
                }
            });
            assert_eq!(count, maximum);
            assert!(matches!(result, Err(EnclosureError::WorkLimit)));
        }
        assert!(matches!(
            prepared.enclose([0., 1.], &mut EnclosureScratch::new(2).unwrap(), || true),
            Err(EnclosureError::Capacity)
        ));
        assert!(prepared.enclose([0., 1.], &mut scratch, || true).is_ok());
    }

    #[test]
    fn distance_and_affine_bounds_cover_analytic_plane_and_extreme_scales() {
        for scale in [1e-100, 1., 1e100] {
            let a = PointEnclosure::point([0., 0., 0.]).unwrap();
            let b = PointEnclosure::point([4. * scale, 0., 2. * scale]).unwrap();
            let bounds = a
                .affine_blend(b, [0.25, 0.75])
                .unwrap()
                .distance_bounds([2. * scale, 3. * scale, scale])
                .unwrap();
            assert!(bounds[0] <= 3. * scale && bounds[1] >= 3. * scale);
            assert!(bounds[0] > 2.99 * scale && bounds[1] < 4. * scale);
        }
        assert!(
            PointEnclosure::point([f64::MAX, 0., 0.])
                .unwrap()
                .distance_bounds([-f64::MAX, 0., 0.])
                .is_err()
        );
    }
}
