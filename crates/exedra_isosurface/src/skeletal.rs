// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Skeletal fields: capsules, round cones, and their N-ary smooth union.
//!
//! These fields describe shapes grown around line segments, such as branches
//! meeting at a fork. [`CapsuleField`] and [`RoundConeField`] are exact signed
//! distances with gradients and sound intervals. [`SmoothUnionN`] blends any
//! number of children symmetrically and, with
//! [`SmoothUnionN::with_distance_culling`], skips children that provably
//! cannot contribute at a point or in a cell.
//!
//! All three evaluate in `f64` and round the result to `f32`, so a value does
//! not depend on how points are batched, and interval bounds can be widened
//! outward by an explicit margin instead of relying on `f32` rounding luck.
//! See ADR-0005.

use alloc::vec::Vec;
use core::cell::Cell;

use exedra_math::{add, dot, norm, scale, sub};
use exedra_spatial::Aabb;

use crate::ScalarField;

/// Capsule signed-distance field: all points within `radius` of the segment
/// from `a` to `b`.
///
/// A zero-length segment is a sphere. [`ScalarField::eval_interval`] returns
/// `None` for non-finite parameters or a negative radius.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CapsuleField {
    /// First segment endpoint.
    pub a: [f32; 3],
    /// Second segment endpoint.
    pub b: [f32; 3],
    /// Radius around the segment.
    pub radius: f32,
}

/// Round-cone signed-distance field: the convex hull of a sphere of
/// `radius_a` at `a` and a sphere of `radius_b` at `b`.
///
/// Where one sphere contains the other (`|radius_a - radius_b|` at least the
/// segment length), the field is that sphere's distance.
/// [`ScalarField::eval_interval`] returns `None` for non-finite parameters or
/// a negative radius.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RoundConeField {
    /// First segment endpoint.
    pub a: [f32; 3],
    /// Second segment endpoint.
    pub b: [f32; 3],
    /// Radius at `a`.
    pub radius_a: f32,
    /// Radius at `b`.
    pub radius_b: f32,
}

/// A conservative lower bound on a field's values by distance to a box.
///
/// For every point `p`, the field value is at least
/// `distance(p, core) - radius`, where the distance is zero inside `core`.
/// Exact distance fields of shapes contained in `core` grown by `radius`
/// satisfy this, including below zero.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DistanceBound {
    /// Box the bound is measured from.
    pub core: Aabb,
    /// Amount subtracted from the distance to `core`.
    pub radius: f32,
}

/// A field that can state a [`DistanceBound`].
///
/// [`SmoothUnionN::with_distance_culling`] uses the bound to skip children
/// that cannot affect a point or a cell. An incorrect bound makes culling
/// change results, so implement this only when the inequality holds for every
/// point.
pub trait BoundedField: ScalarField {
    /// Returns the field's distance bound.
    fn distance_bound(&self) -> DistanceBound;
}

impl<F: BoundedField + ?Sized> BoundedField for &F {
    fn distance_bound(&self) -> DistanceBound {
        (**self).distance_bound()
    }
}

impl<F: BoundedField + ?Sized> BoundedField for alloc::boxed::Box<F> {
    fn distance_bound(&self) -> DistanceBound {
        (**self).distance_bound()
    }
}

/// Symmetric smooth union of any number of children with blend radius `k`.
///
/// The value `F` is defined implicitly by
///
/// ```text
/// sum_i w((v_i - F) / k) = 1,   w(x) = (1 - x/2)^2 for x < 2, else 0
/// ```
///
/// over the child values `v_i`. `w` is continuously differentiable with
/// compact support, so `F` is continuously differentiable wherever the
/// children are, including along ties of any number of children and where a
/// child enters or leaves the blend. Its gradient is the implicit derivative
///
/// ```text
/// grad F = sum_i c_i grad v_i / sum_i c_i,   c_i = -w'((v_i - F) / k) >= 0
/// ```
///
/// With `m` the smallest child value and `n` the number of children within
/// `2k` of it, `m - 2k (1 - 1/sqrt(n)) <= F <= m`: one child in the band is
/// returned unchanged, and two equal children lower the value by
/// `2k (1 - 1/sqrt(2))`. Children more than `2k` above the minimum contribute
/// exactly nothing. The equation is piecewise quadratic in `F`, so it is solved
/// in closed form, not iteratively. This is not a fold of the binary
/// [`SmoothUnion`](crate::analytic::SmoothUnion), whose polynomial blend
/// differs even for two children.
///
/// Children in the band are ordered by value, then gradient, before any
/// sum, so value and gradient bits do not depend on the order of the
/// children. NaN child values are ignored; a union whose children are all
/// NaN or `+inf` has value `+inf`. A non-positive or non-finite `k` is a
/// hard minimum, as is a `-inf` minimum. An empty union has value `+inf`
/// everywhere.
///
/// Evaluation counts its work in [`SmoothUnionStats`] through interior
/// mutability, so the union is not `Sync`.
#[derive(Clone, Debug)]
pub struct SmoothUnionN<C> {
    children: Vec<C>,
    k: f32,
    bounds: Option<Vec<DistanceBound>>,
    stats: Cell<SmoothUnionStats>,
}

/// Work counted by a [`SmoothUnionN`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct SmoothUnionStats {
    /// Points evaluated through `eval_points` or `eval_gradients`.
    pub points: u64,
    /// Child evaluations at those points.
    pub child_evaluations: u64,
    /// Children skipped at those points by distance culling.
    pub child_culls: u64,
    /// Interval queries.
    pub intervals: u64,
    /// Child interval queries made by those queries.
    pub child_intervals: u64,
    /// Children skipped in those queries by distance culling.
    pub child_interval_culls: u64,
}

/// Relative widening of the `2k` blend band used when culling, so rounding in
/// a child's `f32` value cannot let a culled child carry a nonzero weight.
const CULL_MARGIN: f64 = 1.0 / 1024.0;

/// Returns the level a child's lower bound must exceed to be culled against a
/// value `reach` already known to be reachable by the minimum.
///
/// The slack covers the band margin, `f32` rounding of values near `reach`,
/// and an absolute term so that, even for a hard minimum (`band` zero) at
/// `reach` zero, a culled child's `f32` value is strictly above `reach`: it
/// can neither be the minimum nor tie with it.
fn cull_threshold(reach: f64, band: f64) -> f64 {
    let slack = CULL_MARGIN * band
        + 4.0 * f64::from(f32::EPSILON) * (reach.abs() + band)
        + f64::from(f32::MIN_POSITIVE);
    reach + band + slack
}

impl<C> SmoothUnionN<C> {
    /// Creates a union of `children` with blend radius `k`, evaluating every
    /// child at every point.
    #[must_use]
    pub fn new(children: Vec<C>, k: f32) -> Self {
        Self {
            children,
            k,
            bounds: None,
            stats: Cell::new(SmoothUnionStats::default()),
        }
    }

    /// Enables distance culling from each child's [`DistanceBound`].
    ///
    /// A child is skipped at a point, or in a cell, when its bound exceeds a
    /// value already known to reach the minimum by more than the `2k` blend
    /// band, widened by `2^-10` of the band, by `f32` rounding at that value,
    /// and by the smallest normal `f32`. Such a child's value is strictly
    /// above the minimum and outside the band, so it can neither be the
    /// minimum, tie with it, nor carry weight. Culling therefore returns the
    /// same value and gradient bits as evaluating every child whenever the
    /// bounds hold (NaN values are ignored either way). Intervals are
    /// identical for this crate's fields, whose interval lower bounds already
    /// include their distance bound; a child whose `eval_interval` is looser
    /// than its [`DistanceBound`] can get a tighter, still sound, interval
    /// with culling. Work saved is visible in [`Self::stats`].
    #[must_use]
    pub fn with_distance_culling(mut self) -> Self
    where
        C: BoundedField,
    {
        self.bounds = Some(self.children.iter().map(C::distance_bound).collect());
        self
    }

    /// Returns the children in evaluation order.
    #[must_use]
    pub fn children(&self) -> &[C] {
        &self.children
    }

    /// Returns the blend radius.
    #[must_use]
    pub const fn k(&self) -> f32 {
        self.k
    }

    /// Returns true when distance culling is enabled.
    #[must_use]
    pub const fn is_culling(&self) -> bool {
        self.bounds.is_some()
    }

    /// Returns the work counted since creation or the last reset.
    #[must_use]
    pub fn stats(&self) -> SmoothUnionStats {
        self.stats.get()
    }

    /// Resets the work counters.
    pub fn reset_stats(&self) {
        self.stats.set(SmoothUnionStats::default());
    }

    fn blend(&self) -> Option<f64> {
        let k = f64::from(self.k);
        (k.is_finite() && k > 0.0).then_some(k)
    }

    fn count(&self, update: impl FnOnce(&mut SmoothUnionStats)) {
        let mut stats = self.stats.get();
        update(&mut stats);
        self.stats.set(stats);
    }
}

impl<C: ScalarField> SmoothUnionN<C> {
    /// Fills `values` (and `gradients`, when given) for each child at `point`,
    /// with `+inf` for children culled there, and returns the number culled.
    ///
    /// The child with the smallest bound is evaluated first; every other child
    /// is then evaluated, in order, only while its bound is within the culling
    /// threshold of the smallest value seen so far. The threshold only falls,
    /// so a skipped child stays skippable, and the minimum is always found.
    fn culled_values(
        &self,
        bounds: &[DistanceBound],
        point: [f32; 3],
        values: &mut [f32],
        mut gradients: Option<&mut [[f32; 4]]>,
        lowers: &mut Vec<f64>,
    ) -> u64 {
        let p = widen(point);
        lowers.clear();
        lowers.extend(
            bounds
                .iter()
                .map(|bound| point_box_distance(p, &bound.core) - f64::from(bound.radius)),
        );
        values.fill(f32::INFINITY);
        let Some(first) = (0..lowers.len()).min_by(|x, y| lowers[*x].total_cmp(&lowers[*y])) else {
            return 0;
        };
        let band = self.blend().map_or(0.0, |k| 2.0 * k);
        let mut minimum = f64::INFINITY;
        let mut culled = 0;
        let mut evaluate = |index: usize, minimum: &mut f64| {
            let value = match gradients.as_deref_mut() {
                Some(gradients) => {
                    let mut row = [[0.0_f32; 4]];
                    self.children[index].eval_gradients(&[point], &mut row);
                    gradients[index] = row[0];
                    row[0][0]
                }
                None => {
                    let mut one = [0.0_f32];
                    self.children[index].eval_points(&[point], &mut one);
                    one[0]
                }
            };
            values[index] = value;
            *minimum = minimum.min(f64::from(value));
        };
        evaluate(first, &mut minimum);
        for (index, lower) in lowers.iter().enumerate() {
            if index == first {
                continue;
            }
            if *lower > cull_threshold(minimum, band) {
                culled += 1;
            } else {
                evaluate(index, &mut minimum);
            }
        }
        culled
    }

    fn record_points(&self, points: usize, culled: u64) {
        let children = self.children.len() as u64;
        let points = points as u64;
        self.count(|stats| {
            stats.points += points;
            stats.child_evaluations += points * children - culled;
            stats.child_culls += culled;
        });
    }
}

impl<C: ScalarField> ScalarField for SmoothUnionN<C> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let k = self.blend();
        let band = k.map_or(0.0, |k| 2.0 * k);
        let mut order: Vec<(f64, usize)> = match &self.bounds {
            Some(child_bounds) => child_bounds
                .iter()
                .enumerate()
                .map(|(index, bound)| {
                    let lower = box_box_distance(bounds, &bound.core) - f64::from(bound.radius);
                    (lower, index)
                })
                .collect(),
            None => (0..self.children.len())
                .map(|index| (f64::NEG_INFINITY, index))
                .collect(),
        };
        order.sort_unstable_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));

        // Children are ordered by their bound, so once one is beyond the
        // culling threshold of the smallest upper bound so far, every later
        // one is too: it is out of the band at every point of the cell.
        let mut lowers = Vec::with_capacity(order.len());
        let mut uppers = Vec::with_capacity(order.len());
        let mut upper = f64::INFINITY;
        let mut culled = 0;
        let mut failed = false;
        for (position, &(bound, index)) in order.iter().enumerate() {
            if bound > cull_threshold(upper, band) {
                culled = (order.len() - position) as u64;
                break;
            }
            let Some(interval) = self.children[index].eval_interval(bounds) else {
                failed = true;
                break;
            };
            lowers.push(f64::from(interval[0]));
            uppers.push(f64::from(interval[1]));
            upper = upper.min(f64::from(interval[1]));
        }
        let evaluated = lowers.len() as u64 + u64::from(failed);
        self.count(|stats| {
            stats.intervals += 1;
            stats.child_intervals += evaluated;
            stats.child_interval_culls += culled;
        });
        if failed {
            return None;
        }
        if lowers.is_empty() {
            return Some([f32::INFINITY; 2]);
        }

        // The union is nondecreasing in every child value, so its values over
        // the cell lie between the union of the lower bounds and the union of
        // the upper bounds. Culled children are out of the band for both.
        let mut members = Vec::with_capacity(lowers.len());
        let lower = blend_value(&lowers, k, &mut members);
        let upper = blend_value(&uppers, k, &mut members).min(upper);
        // Widen by the error of the closed-form solve (see `solve`). Infinite
        // bounds are exact and stay infinite; widening them would yield NaN.
        let slack = |value: f64| match k {
            Some(k) if value.is_finite() => {
                SOLVE_TOLERANCE * (value.abs() + k) + f64::from(f32::MIN_POSITIVE)
            }
            _ => 0.0,
        };
        Some([
            round_down(lower - slack(lower)),
            round_up(upper + slack(upper)),
        ])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let count = self.children.len();
        let mut values = alloc::vec![0.0_f32; count];
        let mut members = Vec::with_capacity(count);
        let mut culled = 0;
        if let Some(bounds) = &self.bounds {
            let mut lowers = Vec::with_capacity(count);
            for (point, value) in points.iter().zip(out.iter_mut()) {
                culled += self.culled_values(bounds, *point, &mut values, None, &mut lowers);
                *value = self.combine(&values, None, &mut members).0;
            }
        } else {
            // Child-major batches; each value is independent of batching.
            let mut columns = alloc::vec![0.0_f32; count * points.len()];
            for (child, column) in self
                .children
                .iter()
                .zip(columns.chunks_mut(points.len().max(1)))
            {
                child.eval_points(points, column);
            }
            for (row, value) in out.iter_mut().enumerate() {
                for (index, slot) in values.iter_mut().enumerate() {
                    *slot = columns[index * points.len() + row];
                }
                *value = self.combine(&values, None, &mut members).0;
            }
        }
        self.record_points(points.len(), culled);
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        let count = self.children.len();
        let mut values = alloc::vec![0.0_f32; count];
        let mut gradients = alloc::vec![[0.0_f32; 4]; count];
        let mut lowers = Vec::with_capacity(count);
        let mut members = Vec::with_capacity(count);
        let mut one = [[0.0_f32; 4]];
        let mut culled = 0;
        for (point, row) in points.iter().zip(out.iter_mut()) {
            if let Some(bounds) = &self.bounds {
                // Culled children keep stale gradient rows from earlier
                // points; their values are `+inf`, so `combine` never reads
                // those rows.
                culled += self.culled_values(
                    bounds,
                    *point,
                    &mut values,
                    Some(&mut gradients),
                    &mut lowers,
                );
            } else {
                for (index, child) in self.children.iter().enumerate() {
                    child.eval_gradients(&[*point], &mut one);
                    gradients[index] = one[0];
                    values[index] = one[0][0];
                }
            }
            let (value, gradient) = self.combine(&values, Some(&gradients), &mut members);
            row[0] = value;
            row[1..4].copy_from_slice(&gradient);
        }
        self.record_points(points.len(), culled);
    }
}

/// Relative accuracy assumed of the closed-form solve when widening
/// intervals. At the selected bracket the quadratic's discriminant is at
/// least one, so the only rounding loss is the cancellation in
/// `sum^2 - n * excess`; `1e-9` of the value's scale is far below `f32`
/// resolution yet covers it with a wide margin.
const SOLVE_TOLERANCE: f64 = 1.0e-9;

/// A child inside the blend band at one point.
#[derive(Copy, Clone, Debug)]
struct Member {
    /// `1 - (v - m) / (2k)`: one at the minimum, zero at the band's edge.
    height: f64,
    /// The child's gradient, or zero when gradients are not needed.
    gradient: [f64; 3],
}

/// Collects the children within the band of the minimum `minimum`, sorted
/// by descending height, then by gradient, so every later sum is independent
/// of child order.
fn gather(
    values: impl Iterator<Item = (f64, [f64; 3])>,
    minimum: f64,
    k: f64,
    members: &mut Vec<Member>,
) {
    members.clear();
    for (value, gradient) in values {
        let height = 1.0 - (value - minimum) / (2.0 * k);
        if height > 0.0 {
            members.push(Member { height, gradient });
        }
    }
    members.sort_unstable_by(|x, y| {
        y.height
            .total_cmp(&x.height)
            .then(x.gradient[0].total_cmp(&y.gradient[0]))
            .then(x.gradient[1].total_cmp(&y.gradient[1]))
            .then(x.gradient[2].total_cmp(&y.gradient[2]))
    });
}

/// Solves `sum_i max(h_i - u, 0)^2 = 1` for `u` in `[0, 1)`, with the heights
/// `h_i` in descending order and `h_0 = 1`. The union's value is then
/// `m - 2 k u`.
///
/// The left side falls monotonically in `u`, and between consecutive heights
/// it is a quadratic over the members still above `u`. The first bracket whose
/// lower end already reaches one holds the root, which is the quadratic's
/// smaller root, computed in its cancellation-free form.
fn solve(members: &[Member]) -> f64 {
    let mut sum = 0.0;
    let mut sum_squares = 0.0;
    for (index, member) in members.iter().enumerate() {
        sum += member.height;
        sum_squares += member.height * member.height;
        let active = (index + 1) as f64;
        let high = member.height;
        let low = members.get(index + 1).map_or(0.0, |next| next.height);
        let at_low = sum_squares - 2.0 * low * sum + active * low * low;
        if at_low >= 1.0 || index + 1 == members.len() {
            let excess = sum_squares - 1.0;
            if excess <= 0.0 {
                return low.max(0.0).min(high);
            }
            let discriminant = (sum * sum - active * excess).max(0.0);
            let root = excess / (sum + sqrt(discriminant));
            return root.clamp(low, high);
        }
    }
    0.0
}

/// The least non-NaN value, or `+inf` when there is none.
///
/// Ordered by `total_cmp`, so `-0.0` wins over `+0.0` whichever comes first:
/// `f64::min` leaves the sign of a zero tie unspecified, which would let child
/// order leak into the hard minimum's bits.
fn least(values: impl Iterator<Item = f64>) -> f64 {
    values
        .filter(|value| !value.is_nan())
        .fold(f64::INFINITY, |least, value| {
            if value.total_cmp(&least).is_lt() {
                value
            } else {
                least
            }
        })
}

/// The union of plain values, for interval bounds.
fn blend_value(values: &[f64], k: Option<f64>, members: &mut Vec<Member>) -> f64 {
    let minimum = least(values.iter().copied());
    let Some(k) = k.filter(|_| minimum.is_finite()) else {
        return minimum;
    };
    gather(
        values.iter().map(|value| (*value, [0.0; 3])),
        minimum,
        k,
        members,
    );
    minimum - 2.0 * k * solve(members)
}

impl<C> SmoothUnionN<C> {
    /// Blends child values (and gradients) at one point.
    fn combine(
        &self,
        values: &[f32],
        gradients: Option<&[[f32; 4]]>,
        members: &mut Vec<Member>,
    ) -> (f32, [f32; 3]) {
        let minimum = least(values.iter().map(|value| f64::from(*value)));
        if minimum == f64::INFINITY {
            return (f32::INFINITY, [f32::NAN; 3]);
        }
        let gradient_of = |index: usize| {
            gradients.map_or([0.0; 3], |g| {
                [g[index][1], g[index][2], g[index][3]].map(f64::from)
            })
        };
        let entries = || {
            values
                .iter()
                .enumerate()
                .map(move |(index, value)| (f64::from(*value), gradient_of(index)))
        };
        let Some(k) = self.blend().filter(|_| minimum.is_finite()) else {
            // Hard minimum: of the children at the minimum, the one first in
            // gradient order, so ties do not depend on child order.
            members.clear();
            members.extend(entries().filter(|(value, _)| *value == minimum).map(
                |(_, gradient)| Member {
                    height: 1.0,
                    gradient,
                },
            ));
            members.sort_unstable_by(|x, y| {
                x.gradient[0]
                    .total_cmp(&y.gradient[0])
                    .then(x.gradient[1].total_cmp(&y.gradient[1]))
                    .then(x.gradient[2].total_cmp(&y.gradient[2]))
            });
            return (narrow(minimum), members[0].gradient.map(narrow));
        };

        gather(entries(), minimum, k, members);
        let u = solve(members);
        let value = minimum - 2.0 * k * u;
        if gradients.is_none() {
            return (narrow(value), [0.0; 3]);
        }
        // Implicit differentiation: each member weighs by -w' at its offset,
        // which is `h_i - u`.
        let mut gradient = [0.0; 3];
        let mut total = 0.0;
        for member in members.iter() {
            let weight = member.height - u;
            if weight > 0.0 {
                total += weight;
                gradient = add(gradient, scale(member.gradient, weight));
            }
        }
        (narrow(value), scale(gradient, 1.0 / total).map(narrow))
    }
}

impl ScalarField for CapsuleField {
    // Intervals go through the round-cone path with equal radii, which is the
    // same distance with different rounding; the interval margin covers the
    // difference from `eval_points`.
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let shape = Segment::new(self.a, self.b, self.radius, self.radius)?;
        shape.interval(bounds)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let shape = Segment::raw(self.a, self.b, self.radius, self.radius);
        for (point, value) in points.iter().zip(out.iter_mut()) {
            *value = narrow(shape.capsule(widen(*point)).0);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        let shape = Segment::raw(self.a, self.b, self.radius, self.radius);
        for (point, row) in points.iter().zip(out.iter_mut()) {
            let (value, gradient) = shape.capsule(widen(*point));
            write_row(row, value, gradient);
        }
    }
}

impl BoundedField for CapsuleField {
    fn distance_bound(&self) -> DistanceBound {
        DistanceBound {
            core: segment_box(self.a, self.b),
            radius: self.radius,
        }
    }
}

impl ScalarField for RoundConeField {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let shape = Segment::new(self.a, self.b, self.radius_a, self.radius_b)?;
        shape.interval(bounds)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let shape = Segment::raw(self.a, self.b, self.radius_a, self.radius_b);
        for (point, value) in points.iter().zip(out.iter_mut()) {
            *value = narrow(shape.round_cone(widen(*point)).0);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        let shape = Segment::raw(self.a, self.b, self.radius_a, self.radius_b);
        for (point, row) in points.iter().zip(out.iter_mut()) {
            let (value, gradient) = shape.round_cone(widen(*point));
            write_row(row, value, gradient);
        }
    }
}

impl BoundedField for RoundConeField {
    fn distance_bound(&self) -> DistanceBound {
        DistanceBound {
            core: segment_box(self.a, self.b),
            radius: self.radius_a.max(self.radius_b),
        }
    }
}

impl BoundedField for crate::analytic::SphereField {
    fn distance_bound(&self) -> DistanceBound {
        DistanceBound {
            core: Aabb {
                min: self.center,
                max: self.center,
            },
            radius: self.radius,
        }
    }
}

/// A segment with radii at both ends, in `f64`.
#[derive(Copy, Clone, Debug)]
struct Segment {
    a: [f64; 3],
    b: [f64; 3],
    radius_a: f64,
    radius_b: f64,
}

impl Segment {
    fn raw(a: [f32; 3], b: [f32; 3], radius_a: f32, radius_b: f32) -> Self {
        Self {
            a: widen(a),
            b: widen(b),
            radius_a: f64::from(radius_a),
            radius_b: f64::from(radius_b),
        }
    }

    /// Returns the segment when its parameters are finite and its radii are
    /// not negative.
    fn new(a: [f32; 3], b: [f32; 3], radius_a: f32, radius_b: f32) -> Option<Self> {
        let valid = a.iter().chain(&b).all(|c| c.is_finite())
            && radius_a.is_finite()
            && radius_b.is_finite()
            && radius_a >= 0.0
            && radius_b >= 0.0;
        valid.then(|| Self::raw(a, b, radius_a, radius_b))
    }

    fn capsule(&self, p: [f64; 3]) -> (f64, [f64; 3]) {
        let span = sub(self.b, self.a);
        let pa = sub(p, self.a);
        let length2 = dot(span, span);
        let t = if length2 > 0.0 {
            (dot(pa, span) / length2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let delta = sub(pa, scale(span, t));
        let distance = norm(delta);
        (distance - self.radius_a, unit_or_nan(delta, distance))
    }

    /// Exact round-cone distance (Quilez's region classification) with
    /// region-wise gradients; a contained sphere degenerates to the larger
    /// sphere.
    fn round_cone(&self, p: [f64; 3]) -> (f64, [f64; 3]) {
        let span = sub(self.b, self.a);
        let length2 = dot(span, span);
        let rr = self.radius_a - self.radius_b;
        if length2 <= rr * rr {
            let (center, radius) = if self.radius_a >= self.radius_b {
                (self.a, self.radius_a)
            } else {
                (self.b, self.radius_b)
            };
            let delta = sub(p, center);
            let distance = norm(delta);
            return (distance - radius, unit_or_nan(delta, distance));
        }
        let a2 = length2 - rr * rr;
        let pa = sub(p, self.a);
        let y = dot(pa, span);
        let z = y - length2;
        let perpendicular = sub(scale(pa, length2), scale(span, y));
        let x2 = dot(perpendicular, perpendicular);
        let y2 = y * y * length2;
        let z2 = z * z * length2;
        let k = rr.signum() * rr * rr * x2;
        if z.signum() * a2 * z2 > k {
            let delta = sub(p, self.b);
            let distance = norm(delta);
            return (distance - self.radius_b, unit_or_nan(delta, distance));
        }
        if y.signum() * a2 * y2 < k {
            let distance = norm(pa);
            return (distance - self.radius_a, unit_or_nan(pa, distance));
        }
        let length = sqrt(length2);
        let axis = scale(span, 1.0 / length);
        let q = sub(pa, scale(axis, y / length));
        let q_length = norm(q);
        let cos = sqrt(a2) / length;
        let sin = rr / length;
        let value = q_length * cos + (y / length) * sin - self.radius_a;
        let gradient = add(scale(unit_or_nan(q, q_length), cos), scale(axis, sin));
        (value, gradient)
    }

    fn eval(&self, p: [f64; 3]) -> f64 {
        self.round_cone(p).0
    }

    /// Sound bounds over `bounds`: the field is a convex, 1-Lipschitz
    /// distance, so its maximum over a box is at a corner and its minimum is
    /// at least the center value less the half diagonal. The distance to the
    /// segment's box less the larger radius is a second lower bound. Both are
    /// widened by an `f64` error margin scaled by the side region's
    /// conditioning, then rounded outward to `f32`.
    fn interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let min = widen(bounds.min);
        let max = widen(bounds.max);
        if !min.iter().chain(&max).all(|c| c.is_finite()) || (0..3).any(|i| min[i] > max[i]) {
            return None;
        }
        let center = scale(add(min, max), 0.5);
        let half_diagonal = 0.5 * norm(sub(max, min));
        let core = Aabb {
            min: narrow_min(self.a, self.b),
            max: narrow_max(self.a, self.b),
        };
        let radius = self.radius_a.max(self.radius_b);
        let lower =
            (self.eval(center) - half_diagonal).max(box_box_distance(bounds, &core) - radius);
        let mut upper = f64::NEG_INFINITY;
        for x in [min[0], max[0]] {
            for y in [min[1], max[1]] {
                for z in [min[2], max[2]] {
                    upper = upper.max(self.eval([x, y, z]));
                }
            }
        }
        let magnitude = min
            .iter()
            .chain(&max)
            .chain(&self.a)
            .chain(&self.b)
            .fold(radius, |largest, c| largest.max(c.abs()));
        // The side region's `cos = sqrt(l^2 - (ra - rb)^2) / l` loses relative
        // accuracy as `l^2 / (l^2 - (ra - rb)^2)` when the cone approaches
        // containment, so the margin grows by that conditioning.
        let span = sub(self.b, self.a);
        let length2 = dot(span, span);
        let rr = self.radius_a - self.radius_b;
        let conditioning = if length2 > rr * rr {
            length2 / (length2 - rr * rr)
        } else {
            1.0
        };
        let margin = 64.0 * f64::EPSILON * magnitude * conditioning;
        let interval = [round_down(lower - margin), round_up(upper + margin)];
        interval.iter().all(|v| !v.is_nan()).then_some(interval)
    }
}

fn sqrt(value: f64) -> f64 {
    exedra_math::Real::sqrt(value)
}

/// Rounds an `f64` result to the nearest `f32`, the crate's value type.
#[expect(
    clippy::cast_possible_truncation,
    reason = "fields compute in f64 and return f32 by design; rounding is intended"
)]
fn narrow(value: f64) -> f32 {
    value as f32
}

fn widen(point: [f32; 3]) -> [f64; 3] {
    point.map(f64::from)
}

fn unit_or_nan(vector: [f64; 3], length: f64) -> [f64; 3] {
    if length > 0.0 {
        scale(vector, 1.0 / length)
    } else {
        [f64::NAN; 3]
    }
}

fn write_row(row: &mut [f32; 4], value: f64, gradient: [f64; 3]) {
    row[0] = narrow(value);
    row[1] = narrow(gradient[0]);
    row[2] = narrow(gradient[1]);
    row[3] = narrow(gradient[2]);
}

fn segment_box(a: [f32; 3], b: [f32; 3]) -> Aabb {
    Aabb {
        min: [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
        max: [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
    }
}

fn narrow_min(a: [f64; 3], b: [f64; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| narrow(a[i].min(b[i])))
}

fn narrow_max(a: [f64; 3], b: [f64; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| narrow(a[i].max(b[i])))
}

fn point_box_distance(p: [f64; 3], core: &Aabb) -> f64 {
    let gap = [0, 1, 2].map(|i| {
        (f64::from(core.min[i]) - p[i])
            .max(p[i] - f64::from(core.max[i]))
            .max(0.0)
    });
    norm(gap)
}

fn box_box_distance(cell: &Aabb, core: &Aabb) -> f64 {
    let gap = [0, 1, 2].map(|i| {
        (f64::from(core.min[i]) - f64::from(cell.max[i]))
            .max(f64::from(cell.min[i]) - f64::from(core.max[i]))
            .max(0.0)
    });
    norm(gap)
}

/// The largest `f32` not above `value`.
fn round_down(value: f64) -> f32 {
    let rounded = narrow(value);
    if f64::from(rounded) > value {
        rounded.next_down()
    } else {
        rounded
    }
}

/// The smallest `f32` not below `value`.
fn round_up(value: f64) -> f32 {
    let rounded = narrow(value);
    if f64::from(rounded) < value {
        rounded.next_up()
    } else {
        rounded
    }
}

fn assert_same_len(expected: usize, found: usize, label: &str) {
    assert_eq!(expected, found, "{label} slice lengths must match");
}

#[cfg(test)]
#[path = "skeletal_tests.rs"]
mod tests;
