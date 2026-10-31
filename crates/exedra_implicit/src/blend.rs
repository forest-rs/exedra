// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A symmetric smooth union of any number of parts.

use alloc::vec::Vec;

use exedra_math::Real;

use crate::bounds::length_down;
use crate::claims::{Claims, Metric, Support};
use crate::formula::Formula;
use crate::number::{Dual, Interval, Number};
use crate::{Aabb, FieldError, check_positive};

/// Symmetric smooth union of any number of parts with blend radius `k`.
///
/// The value `F` is defined implicitly by
///
/// ```text
/// sum_i w((v_i - F) / k) = 1,   w(x) = (1 - x/2)^2 for x < 2, else 0
/// ```
///
/// over the parts' values `v_i`. `w` is continuously differentiable with
/// compact support, so `F` is continuously differentiable wherever the
/// parts are, including along ties of any number of parts and where a part
/// enters or leaves the blend; it marks no crease. Its gradient is the
/// implicit derivative, a convex combination of the parts' gradients:
///
/// ```text
/// grad F = sum_i c_i grad v_i / sum_i c_i,   c_i = -w'((v_i - F) / k) >= 0
/// ```
///
/// With `m` the smallest value and `n` the number of parts within `2k` of
/// it, `m - 2k (1 - 1/sqrt(n)) <= F <= m`: one part in the band is returned
/// unchanged, and two equal parts lower the value by `2k (1 - 1/sqrt(2))`.
/// Parts more than `2k` above the minimum contribute exactly nothing. The
/// equation is piecewise quadratic in `F`, so it is solved in closed form.
///
/// Parts claiming exact distance and a bounded interior are skipped where
/// that bound shows them out of the band, without changing the result. The
/// sum does not depend on the order of the parts. An empty union is
/// `+inf` everywhere.
#[derive(Clone, Debug, PartialEq)]
pub struct SmoothUnionN<C> {
    parts: Vec<C>,
    radius: f64,
    /// The interior box of each part whose value bounds its distance to
    /// it, for skipping parts far from a point.
    reaches: Vec<Option<Aabb>>,
}

impl<C: Formula> SmoothUnionN<C> {
    /// The union of `parts` blended over `radius`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the radius is not finite and positive.
    pub fn new(parts: Vec<C>, radius: f64) -> Result<Self, FieldError> {
        check_positive("radius", radius)?;
        let reaches = parts
            .iter()
            .map(|part| {
                let claims = part.claims();
                match (claims.metric, claims.support.bounds()) {
                    (Metric::ExactDistance, Some(bounds)) => Some(*bounds),
                    _ => None,
                }
            })
            .collect();
        Ok(Self {
            parts,
            radius,
            reaches,
        })
    }
}

impl<C> SmoothUnionN<C> {
    /// The parts, in evaluation order.
    #[must_use]
    pub fn parts(&self) -> &[C] {
        &self.parts
    }

    /// The blend radius.
    #[must_use]
    pub const fn radius(&self) -> f64 {
        self.radius
    }
}

impl<C: Formula> Formula for SmoothUnionN<C> {
    /// Evaluates the part nearest by its bound first, then each other part
    /// whose bound does not put it out of the band of the least upper bound
    /// so far. That bound only falls, so the minimum is always found.
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        let region = Aabb::new(p.map(|c| N::span(c).low()), p.map(|c| N::span(c).high()));
        let lower = |index: usize| match (&self.reaches[index], &region) {
            (Some(reach), Some(region)) => least_value(region, reach),
            _ => f64::NEG_INFINITY,
        };
        let lowers: Vec<f64> = (0..self.parts.len()).map(lower).collect();
        let Some(first) = (0..lowers.len()).min_by(|&a, &b| lowers[a].total_cmp(&lowers[b])) else {
            return N::constant(f64::INFINITY);
        };
        let band = 2.0 * self.radius;
        let mut values = Vec::with_capacity(self.parts.len());
        let mut ceiling = f64::INFINITY;
        let mut evaluate = |index: usize, ceiling: &mut f64| {
            let value = self.parts[index].eval(p);
            *ceiling = ceiling.min(N::span(value).high());
            values.push(value);
        };
        evaluate(first, &mut ceiling);
        for (index, lower) in lowers.iter().enumerate() {
            if index != first && *lower <= cull_threshold(ceiling, band) {
                evaluate(index, &mut ceiling);
            }
        }
        N::smooth_union(&values, self.radius)
    }

    /// The blend is a convex combination of the parts' gradients, so it
    /// changes no faster than the fastest part. It lowers values by less
    /// than `2k`, which for exact distances grows the interior by less than
    /// that.
    fn claims(&self) -> Claims {
        let mut claims = self.parts.iter().map(Formula::claims);
        let Some(first) = claims.next() else {
            return Claims {
                metric: Metric::Unknown,
                support: Support::Empty,
            };
        };
        let (metric, support, exact) = claims.fold(
            (
                first.metric,
                first.support,
                first.metric == Metric::ExactDistance,
            ),
            |(metric, support, exact), part| {
                (
                    metric.combined(part.metric),
                    support.union(part.support),
                    exact && part.metric == Metric::ExactDistance,
                )
            },
        );
        let metric = match metric {
            Metric::ExactDistance if self.parts.len() > 1 => Metric::Lipschitz(1.0),
            metric => metric,
        };
        let support = if exact {
            support.grown(2.0 * self.radius)
        } else if matches!(support, Support::Unbounded) {
            Support::Unbounded
        } else {
            Support::Unknown
        };
        Claims { metric, support }
    }
}

/// The level a part's lower bound must exceed to be skipped against a value
/// `ceiling` already known to be reached: past the band, with slack for
/// rounding, so a skipped part is strictly out of the band.
fn cull_threshold(ceiling: f64, band: f64) -> f64 {
    ceiling + band + 1.0e-9 * (ceiling.abs() + band) + f64::MIN_POSITIVE
}

/// A lower bound over `region` on an exact distance whose interior lies in
/// `reach`: the gap between the boxes where they are apart. Where they
/// meet, a point inside is no deeper than half the box's least extent,
/// since every path out of the box crosses the surface.
fn least_value(region: &Aabb, reach: &Aabb) -> f64 {
    let gap: [f64; 3] = core::array::from_fn(|i| {
        (reach.min[i] - region.max[i])
            .max(region.min[i] - reach.max[i])
            .max(0.0)
    });
    if gap == [0.0; 3] {
        let depth = reach.extent().into_iter().fold(f64::INFINITY, f64::min);
        return -(0.5 * depth).next_up();
    }
    length_down(gap)
}

/// Relative accuracy assumed of the closed-form solve when widening
/// intervals. At the selected bracket the quadratic's discriminant is at
/// least one, so the only rounding loss is the cancellation in
/// `sum^2 - n * excess`, far below this.
const SOLVE_TOLERANCE: f64 = 1.0e-9;

/// A part inside the blend band at one point.
#[derive(Copy, Clone, Debug)]
struct Member {
    /// `1 - (v - m) / (2k)`: one at the minimum, zero at the band's edge.
    height: f64,
    /// The part's gradient, or zero when gradients are not needed.
    gradient: [f64; 3],
}

/// The blend of plain values and gradients at one point, with the weight
/// of each part in the gradient.
pub(crate) fn blend(
    values: impl Iterator<Item = (f64, [f64; 3])> + Clone,
    radius: f64,
) -> (f64, [f64; 3]) {
    let minimum = least(values.clone().map(|(value, _)| value));
    if !minimum.is_finite() {
        // No part, or a part at -inf: the minimum, with its gradient.
        let gradient = values
            .filter(|(value, _)| *value == minimum)
            .map(|(_, gradient)| gradient)
            .reduce(|a, b| if order(&b, &a).is_lt() { b } else { a })
            .unwrap_or([f64::NAN; 3]);
        return (minimum, gradient);
    }
    let members = gather(values, minimum, radius);
    let u = solve(&members);
    // Implicit differentiation: each member weighs by -w' at its offset,
    // which is `h_i - u`.
    let mut gradient = [0.0; 3];
    let mut total = 0.0;
    for member in &members {
        let weight = member.height - u;
        if weight > 0.0 {
            total += weight;
            for (sum, part) in gradient.iter_mut().zip(member.gradient) {
                *sum += part * weight;
            }
        }
    }
    (minimum - 2.0 * radius * u, gradient.map(|g| g / total))
}

/// The blend of intervals: the union is nondecreasing in every part's
/// value, so it lies between the blends of the ends.
pub(crate) fn blend_interval(values: &[Interval], radius: f64) -> Interval {
    let end =
        |end: fn(Interval) -> f64| blend(values.iter().map(|v| (end(*v), [0.0; 3])), radius).0;
    let (low, high) = (end(Interval::low), end(Interval::high));
    let slack = |value: f64| {
        if value.is_finite() {
            SOLVE_TOLERANCE * (value.abs() + radius) + f64::MIN_POSITIVE
        } else {
            0.0
        }
    };
    Interval::new(low - slack(low), high + slack(high))
}

/// The blend of dual intervals: the gradient is a convex combination of
/// the gradients of parts that may be in the band.
pub(crate) fn blend_dual_interval(values: &[Dual<Interval>], radius: f64) -> Dual<Interval> {
    let value = blend_interval(&values.iter().map(|v| v.value).collect::<Vec<_>>(), radius);
    let ceiling = values
        .iter()
        .map(|v| v.value.high())
        .fold(f64::INFINITY, f64::min);
    let threshold = cull_threshold(ceiling, 2.0 * radius);
    let gradient = values
        .iter()
        .filter(|v| v.value.low() <= threshold)
        .map(|v| v.gradient)
        .reduce(|a, b| core::array::from_fn(|i| a[i].hull(b[i])))
        .unwrap_or([Interval::ENTIRE; 3]);
    Dual { value, gradient }
}

/// Whether a part with this value is in the band of the blend at `minimum`.
pub(crate) fn in_band(value: f64, minimum: f64, radius: f64) -> bool {
    value - minimum < 2.0 * radius
}

/// The least non-NaN value, or `+inf` when there is none.
///
/// Ordered by `total_cmp`, so `-0.0` wins over `+0.0` whichever comes first,
/// and part order cannot leak into the bits.
pub(crate) fn least(values: impl Iterator<Item = f64>) -> f64 {
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

fn order(a: &[f64; 3], b: &[f64; 3]) -> core::cmp::Ordering {
    a[0].total_cmp(&b[0])
        .then(a[1].total_cmp(&b[1]))
        .then(a[2].total_cmp(&b[2]))
}

/// The parts within the band of `minimum`, by descending height, then by
/// gradient, so every later sum is independent of part order.
fn gather(values: impl Iterator<Item = (f64, [f64; 3])>, minimum: f64, radius: f64) -> Vec<Member> {
    let mut members: Vec<Member> = values
        .filter_map(|(value, gradient)| {
            let height = 1.0 - (value - minimum) / (2.0 * radius);
            (height > 0.0).then_some(Member { height, gradient })
        })
        .collect();
    members.sort_unstable_by(|x, y| {
        y.height
            .total_cmp(&x.height)
            .then(order(&x.gradient, &y.gradient))
    });
    members
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
        #[expect(clippy::cast_precision_loss, reason = "member counts are small")]
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
            let root = excess / (sum + Real::sqrt(discriminant));
            return root.clamp(low, high);
        }
    }
    0.0
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::SmoothUnionN;
    use crate::{Aabb, Capsule, Dual, Formula, Interval, ScalarField, Sphere};

    fn branches() -> Vec<Capsule> {
        vec![
            Capsule::new([0.0; 3], [0.0, 1.0, 0.0], 0.2).unwrap(),
            Capsule::new([0.0, 1.0, 0.0], [0.5, 1.6, 0.0], 0.12).unwrap(),
            Capsule::new([0.0, 1.0, 0.0], [-0.5, 1.6, 0.0], 0.12).unwrap(),
            Capsule::new([5.0, 0.0, 0.0], [5.0, 1.0, 0.0], 0.2).unwrap(),
        ]
    }

    #[test]
    fn one_part_in_the_band_is_unchanged_and_two_equal_lower_by_the_bound() {
        let k = 0.1;
        let a = Sphere::new([-1.0, 0.0, 0.0], 0.5).unwrap();
        let b = Sphere::new([1.0, 0.0, 0.0], 0.5).unwrap();
        let union = SmoothUnionN::new(vec![a, b], k).unwrap();
        assert_eq!(union.eval([-2.0, 0.0, 0.0]), a.eval([-2.0, 0.0, 0.0]));
        let middle = union.eval([0.0; 3]);
        let expected = 0.5 - 2.0 * k * (1.0 - 1.0 / 2.0_f64.sqrt());
        assert!(
            (middle - expected).abs() < 1e-12,
            "{middle} against {expected}"
        );
    }

    #[test]
    fn order_does_not_change_the_bits() {
        let forward = SmoothUnionN::new(branches(), 0.15).unwrap();
        let mut reversed = branches();
        reversed.reverse();
        let backward = SmoothUnionN::new(reversed, 0.15).unwrap();
        for p in [
            [0.1, 1.05, 0.0],
            [0.0, 0.5, 0.1],
            [0.3, 1.3, 0.05],
            [4.9, 0.5, 0.0],
        ] {
            let (mut a, mut b) = ([Dual::default()], [Dual::default()]);
            forward.eval_gradients(&[p], &mut a);
            backward.eval_gradients(&[p], &mut b);
            assert_eq!(a, b);
        }
    }

    #[test]
    fn skipping_far_parts_changes_nothing() {
        let union = SmoothUnionN::new(branches(), 0.15).unwrap();
        // The same parts with their claims hidden cannot be skipped.
        let hidden: Vec<_> = branches().into_iter().map(crate::Opaque).collect();
        struct Unclaimed(crate::Opaque<Capsule>);
        impl Formula for Unclaimed {
            fn eval<N: crate::Number>(&self, p: [N; 3]) -> N {
                self.0.eval(p)
            }
        }
        let every = SmoothUnionN::new(hidden.into_iter().map(Unclaimed).collect(), 0.15).unwrap();
        for p in [
            [0.1, 1.05, 0.0],
            [0.0, 0.5, 0.1],
            [4.9, 0.5, 0.0],
            [2.5, 0.0, 0.0],
        ] {
            assert_eq!(union.eval(p), every.eval(p));
        }
    }

    #[test]
    fn intervals_and_gradients_bound_samples() {
        let union = SmoothUnionN::new(branches(), 0.15).unwrap();
        for region in [
            Aabb::new([-0.2, 0.8, -0.1], [0.2, 1.2, 0.1]).unwrap(),
            Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).unwrap(),
        ] {
            let bounds: Dual<Interval> = union.eval_gradient_interval(&region);
            for p in region.corners().into_iter().chain([region.center()]) {
                let mut row = [Dual::default()];
                union.eval_gradients(&[p], &mut row);
                assert!(bounds.value.contains(row[0].value), "{p:?}");
                for axis in 0..3 {
                    assert!(
                        bounds.gradient[axis].contains(row[0].gradient[axis]),
                        "{p:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn blends_mark_no_crease_where_parts_meet() {
        let union = SmoothUnionN::new(branches(), 0.15).unwrap();
        let fork = [0.0, 1.0, 0.13];
        assert!(
            union
                .eval_crease(fork)
                .ties
                .iter()
                .flatten()
                .all(|tie| tie.distance > 0.01)
        );
    }

    #[test]
    fn parts_overlapping_deep_inside_are_not_skipped() {
        let ball = Sphere::new([0.0; 3], 1.0).unwrap();
        let blend = SmoothUnionN::new(vec![ball, ball], 0.1).unwrap();
        // Two equal parts lower the value by 2k (1 - 1/sqrt(2)).
        let expected = -1.0 - 0.2 * (1.0 - 0.5_f64.sqrt());
        for p in [[0.0; 3], [0.85, 0.0, 0.0], [0.0, -0.5, 0.3]] {
            let value = blend.eval(p);
            let expected = expected + (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!((value - expected).abs() < 1e-12, "{p:?}: {value}");
        }
    }

    #[test]
    fn far_parts_are_ranked_without_overflow() {
        let near = Sphere::new([1e200, 0.0, 0.0], 1.0).unwrap();
        let far = Sphere::new([2e200, 0.0, 0.0], 1.0).unwrap();
        for parts in [vec![far, near], vec![near, far]] {
            let blend = SmoothUnionN::new(parts, 1.0).unwrap();
            let value = blend.eval([0.0; 3]);
            assert!((value / 1e200 - 1.0).abs() < 1e-12, "{value}");
        }
    }
}
