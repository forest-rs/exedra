// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The evaluation boundary: what consumers ask of a field.

use alloc::boxed::Box;

use crate::Aabb;
use crate::bounds::length_up;
use crate::claims::Claims;
use crate::number::{Dual, Interval};

/// A scalar field: values negative inside and positive outside, whose zero
/// set is a surface.
///
/// This is what consumers such as surface extraction ask of a field. Every
/// [`Formula`](crate::Formula) is one, answering each query from its single
/// definition. Fields that cannot be written as formulas, such as compiled
/// expressions, sampled data or caller code, implement it directly and
/// answer what they can; the defaults answer the rest soundly from
/// [`claims`](Self::claims), and such a field joins formulas through
/// [`Opaque`](crate::Opaque).
pub trait ScalarField {
    /// The field's metric and support claims.
    ///
    /// The default claims nothing, which is always truthful. Override it
    /// only with claims that hold at every point.
    fn claims(&self) -> Claims {
        Claims::UNKNOWN
    }

    /// Evaluates the field at `points`, one value per point.
    ///
    /// # Panics
    ///
    /// Implementations may panic when `out.len()` differs from
    /// `points.len()`.
    fn eval_points(&self, points: &[[f64; 3]], out: &mut [f64]);

    /// Evaluates the field and its gradient at `points`, one row per point.
    ///
    /// Gradients may be NaN where the field is not differentiable.
    ///
    /// # Panics
    ///
    /// Implementations may panic when `out.len()` differs from
    /// `points.len()`.
    fn eval_gradients(&self, points: &[[f64; 3]], out: &mut [Dual]);

    /// Bounds on the field's values over `region`, which may be flat along
    /// some axes.
    ///
    /// The interval must contain every value in `region`. The default bounds
    /// a Lipschitz field by its value at the center and its claim, and is
    /// otherwise [`Interval::ENTIRE`].
    fn eval_interval(&self, region: &Aabb) -> Interval {
        self.claims()
            .metric
            .lipschitz()
            .map_or(Interval::ENTIRE, |lipschitz| {
                let mut value = [0.0];
                self.eval_points(&[region.center()], &mut value);
                around(value[0], lipschitz, region)
            })
    }

    /// Bounds on the field's values and gradients over `region`.
    ///
    /// Where a derivative's bounds keep one sign, the field is monotone
    /// along that axis throughout `region`, which lets consumers prove a box
    /// holds no surface. The default takes the value bounds from
    /// [`eval_interval`](Self::eval_interval), and bounds each derivative of
    /// a Lipschitz field by its claim.
    fn eval_gradient_interval(&self, region: &Aabb) -> Dual<Interval> {
        let slope = self
            .claims()
            .metric
            .lipschitz()
            .map_or(Interval::ENTIRE, |lipschitz| {
                Interval::new(-lipschitz, lipschitz)
            });
        Dual {
            value: self.eval_interval(region),
            gradient: [slope; 3],
        }
    }

    /// The field at `point`, the creases nearest it, and the region of the
    /// part that shapes it.
    ///
    /// The default reports the field's value and gradient, no creases and
    /// no region.
    fn eval_crease(&self, point: [f64; 3]) -> Crease {
        let mut row = [Dual::default()];
        self.eval_gradients(&[point], &mut row);
        Crease {
            field: row[0],
            ties: [None; 2],
            region: 0,
        }
    }
}

/// A field's value and gradient at a point, with the creases nearest it
/// and the region of the part that shapes it.
///
/// Where the field and a tie are both zero, the surface has an edge; where
/// the field and two ties are, a corner.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Crease {
    /// The field's value and gradient.
    pub field: Dual,
    /// Up to two ties, nearest first.
    pub ties: [Option<Tie>; 2],
    /// The region of the [`Tagged`](crate::Tagged) part whose value the
    /// field takes at the point, or zero where no tagged part does.
    pub region: u32,
}

/// A tie between two of a field's pieces: a function, with its gradient,
/// that is zero where they meet, as where a `max` of two surfaces switches
/// from one to the other.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Tie {
    /// The tie's value and gradient.
    pub function: Dual,
    /// How far away the crease counts as: at least the tie's first-order
    /// distance, `|value| / |gradient|`, and further for a crease within a
    /// piece that does not shape the field at the point, which matters only
    /// beyond the tie that would make it.
    pub distance: f64,
}

/// `value`, the field at `region`'s computed center, widened by a Lipschitz
/// `factor` over the farthest any point of `region` lies from that center,
/// with a margin for the rounding of the value.
pub(crate) fn around(value: f64, factor: f64, region: &Aabb) -> Interval {
    let center = region.center();
    let half: [f64; 3] = core::array::from_fn(|i| {
        (center[i] - region.min[i])
            .max(region.max[i] - center[i])
            .next_up()
    });
    let radius = length_up(half);
    let reach = (factor * radius).next_up();
    let margin = 64.0 * f64::EPSILON * (value.abs() + reach);
    let interval = Interval::new(value - reach - margin, value + reach + margin);
    if interval.low().is_nan() || interval.high().is_nan() {
        Interval::ENTIRE
    } else {
        interval
    }
}

impl ScalarField for &dyn ScalarField {
    fn claims(&self) -> Claims {
        (**self).claims()
    }

    fn eval_points(&self, points: &[[f64; 3]], out: &mut [f64]) {
        (**self).eval_points(points, out);
    }

    fn eval_gradients(&self, points: &[[f64; 3]], out: &mut [Dual]) {
        (**self).eval_gradients(points, out);
    }

    fn eval_interval(&self, region: &Aabb) -> Interval {
        (**self).eval_interval(region)
    }

    fn eval_gradient_interval(&self, region: &Aabb) -> Dual<Interval> {
        (**self).eval_gradient_interval(region)
    }

    fn eval_crease(&self, point: [f64; 3]) -> Crease {
        (**self).eval_crease(point)
    }
}

impl ScalarField for Box<dyn ScalarField + '_> {
    fn claims(&self) -> Claims {
        (**self).claims()
    }

    fn eval_points(&self, points: &[[f64; 3]], out: &mut [f64]) {
        (**self).eval_points(points, out);
    }

    fn eval_gradients(&self, points: &[[f64; 3]], out: &mut [Dual]) {
        (**self).eval_gradients(points, out);
    }

    fn eval_interval(&self, region: &Aabb) -> Interval {
        (**self).eval_interval(region)
    }

    fn eval_gradient_interval(&self, region: &Aabb) -> Dual<Interval> {
        (**self).eval_gradient_interval(region)
    }

    fn eval_crease(&self, point: [f64; 3]) -> Crease {
        (**self).eval_crease(point)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Aabb, HalfSpace, ScalarField};

    #[test]
    fn bounds_hold_at_tiny_scales() {
        let plane = HalfSpace::new([0.0; 3], [1.0, 0.0, 0.0]).unwrap();
        for scale in [1e-200, 1e-310, 1e200] {
            let region = Aabb::new([-scale, 0.0, 0.0], [scale, 0.0, 0.0]).unwrap();
            let bounds = plane.eval_interval(&region);
            assert!(bounds.contains(-scale) && bounds.contains(scale), "{scale}");
        }
    }

    #[test]
    fn bounds_hold_where_the_center_rounds() {
        // At this magnitude the box's center rounds to its lower corner.
        let plane = HalfSpace::new([1.0e16, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap();
        let region = Aabb::new([1.0e16, 0.0, 0.0], [1.0e16 + 2.0, 0.0, 0.0]).unwrap();
        let bounds = plane.eval_interval(&region);
        for corner in region.corners() {
            let mut value = [0.0];
            plane.eval_points(&[corner], &mut value);
            assert!(bounds.contains(value[0]), "{bounds:?} misses {}", value[0]);
        }
    }
}
