// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Solids combined: unions, intersections and differences, sharp or
//! blended.
//!
//! Sharp combinations take the `min` or `max` of their parts, which marks
//! the creases where the parts' surfaces meet.

use crate::claims::{Claims, Metric, Support};
use crate::formula::Formula;
use crate::number::Number;
use crate::vector::positive;
use crate::{FieldError, check_positive};

/// Everything inside either part.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Union<A, B> {
    /// One part.
    pub left: A,
    /// The other part.
    pub right: B,
}

/// Everything inside both parts.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Intersection<A, B> {
    /// One part.
    pub left: A,
    /// The other part.
    pub right: B,
}

/// Everything inside `left` and outside `right`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Difference<A, B> {
    /// The part cut from.
    pub left: A,
    /// The part cut away.
    pub right: B,
}

impl<A, B> Union<A, B> {
    /// The union of `left` and `right`.
    #[must_use]
    pub const fn new(left: A, right: B) -> Self {
        Self { left, right }
    }
}

impl<A, B> Intersection<A, B> {
    /// The intersection of `left` and `right`.
    #[must_use]
    pub const fn new(left: A, right: B) -> Self {
        Self { left, right }
    }
}

impl<A, B> Difference<A, B> {
    /// `left` with `right` cut away.
    #[must_use]
    pub const fn new(left: A, right: B) -> Self {
        Self { left, right }
    }
}

impl<A: Formula, B: Formula> Formula for Union<A, B> {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        self.left.eval(p).min(self.right.eval(p))
    }

    fn claims(&self) -> Claims {
        let (left, right) = (self.left.claims(), self.right.claims());
        Claims {
            metric: left.metric.combined(right.metric),
            support: left.support.union(right.support),
        }
    }
}

impl<A: Formula, B: Formula> Formula for Intersection<A, B> {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        self.left.eval(p).max(self.right.eval(p))
    }

    fn claims(&self) -> Claims {
        let (left, right) = (self.left.claims(), self.right.claims());
        Claims {
            metric: left.metric.combined(right.metric),
            support: left.support.intersection(right.support),
        }
    }
}

impl<A: Formula, B: Formula> Formula for Difference<A, B> {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        self.left.eval(p).max(-self.right.eval(p))
    }

    fn claims(&self) -> Claims {
        let (left, right) = (self.left.claims(), self.right.claims());
        Claims {
            metric: left.metric.combined(right.metric),
            support: left.support.without(&right.support),
        }
    }
}

/// A part tagged with a region, so that consumers can tell which part of a
/// combination shapes the surface at a point.
///
/// The tag changes no value. [`ScalarField::eval_crease`](crate::ScalarField::eval_crease)
/// reports it wherever the combination takes this part's value: through the
/// side `min`, `max` and `select` choose, and the least part of a smooth
/// union. Surface extraction can then label each face, as with a mesh's
/// face regions. Zero means no region.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Tagged<F> {
    /// The part.
    pub field: F,
    /// Its region.
    pub region: u32,
}

impl<F> Tagged<F> {
    /// `field` tagged with `region`.
    #[must_use]
    pub const fn new(field: F, region: u32) -> Self {
        Self { field, region }
    }
}

impl<F: Formula> Formula for Tagged<F> {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        self.field.eval(p).tagged(self.region)
    }

    fn claims(&self) -> Claims {
        self.field.claims()
    }
}

/// The union of two parts with the crease where they meet rounded away.
///
/// Quilez's polynomial smooth minimum: within `radius` of a tie, the parts
/// blend along a quadratic, lowering the value by at most `radius / 4`;
/// elsewhere it is the plain minimum. The blend is smooth, so it marks no
/// crease.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SmoothUnion<A, B> {
    left: A,
    right: B,
    radius: f64,
}

impl<A, B> SmoothUnion<A, B> {
    /// The union of `left` and `right`, blended over `radius`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the radius is not finite and positive.
    pub fn new(left: A, right: B, radius: f64) -> Result<Self, FieldError> {
        check_positive("radius", radius)?;
        Ok(Self {
            left,
            right,
            radius,
        })
    }

    /// One part.
    #[must_use]
    pub const fn left(&self) -> &A {
        &self.left
    }

    /// The other part.
    #[must_use]
    pub const fn right(&self) -> &B {
        &self.right
    }

    /// The blend radius.
    #[must_use]
    pub const fn radius(&self) -> f64 {
        self.radius
    }
}

impl<A: Formula, B: Formula> Formula for SmoothUnion<A, B> {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        let (a, b) = (self.left.eval(p), self.right.eval(p));
        let k = self.radius;
        // min(a, b) - k h² / 4, h = max(k - |a - b|, 0) / k, joined by
        // `select` so the blend marks no crease and keeps the lesser part.
        let d = a - b;
        let h = d.select(d + k, -d + k) / k;
        // Clamped below one only past it, so at a tie h keeps its slope.
        let h = positive((-h + 1.0).select(N::constant(1.0), h));
        d.select(a, b) - h * h * (k / 4.0)
    }

    /// The blend is a convex combination of the parts' gradients, so it
    /// changes no faster than the faster part. It can only add interior
    /// where a part's value is below `radius / 4`, which for exact
    /// distances lies within that distance of the union.
    fn claims(&self) -> Claims {
        let (left, right) = (self.left.claims(), self.right.claims());
        let union = left.support.union(right.support);
        let support = match (left.metric, right.metric) {
            (Metric::ExactDistance, Metric::ExactDistance) => union.grown(self.radius / 4.0),
            _ if matches!(union, Support::Unbounded) => Support::Unbounded,
            _ => Support::Unknown,
        };
        Claims {
            metric: left.metric.combined(right.metric),
            support,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Difference, Intersection, SmoothUnion, Tagged, Union};
    use crate::{Cuboid, Cylinder, Dual, Formula, Metric, ScalarField, Sphere, Support};

    fn spheres() -> (Sphere, Sphere) {
        (
            Sphere::new([-0.5, 0.0, 0.0], 1.0).unwrap(),
            Sphere::new([0.5, 0.0, 0.0], 1.0).unwrap(),
        )
    }

    #[test]
    fn sharp_combinations_choose_and_mark_creases() {
        let (a, b) = spheres();
        let union = Union::new(a, b);
        assert_eq!(union.eval([-1.5, 0.0, 0.0]), 0.0);
        let rim = [0.0, 0.75_f64.sqrt(), 0.0];
        assert!(union.eval(rim).abs() < 1e-15);
        let crease = union.eval_crease(rim);
        assert!(crease.ties[0].unwrap().function.value.abs() < 1e-15);
        assert_eq!(Formula::claims(&union).metric, Metric::Lipschitz(1.0));

        let lens = Intersection::new(a, b);
        assert!(lens.eval([0.0; 3]) < 0.0 && lens.eval([1.2, 0.0, 0.0]) > 0.0);
        let Support::Bounded(bounds) = Formula::claims(&lens).support else {
            panic!("bounded");
        };
        assert!(bounds.max[0] < 0.6 && bounds.min[0] > -0.6);

        let bite = Difference::new(Cuboid::new([0.0; 3], [1.0; 3]).unwrap(), b);
        assert!(bite.eval([-0.9, 0.0, 0.0]) < 0.0 && bite.eval([0.9, 0.0, 0.0]) > 0.0);
    }

    #[test]
    fn smooth_unions_blend_without_creases() {
        let (a, b) = spheres();
        let blend = SmoothUnion::new(a, b, 0.4).unwrap();
        let rim = [0.0, 0.75_f64.sqrt(), 0.0];
        // The blend fills the crease: the old rim is now inside.
        assert!(blend.eval(rim) < -0.05);
        assert_eq!(blend.eval_crease(rim).ties, [None, None]);
        // Far from the tie it is the plain minimum, gradient and all.
        assert_eq!(blend.eval([-2.0, 0.0, 0.0]), a.eval([-2.0, 0.0, 0.0]));
        let mut row = [Dual::default()];
        blend.eval_gradients(&[rim], &mut row);
        assert!(
            row[0].gradient[0].abs() < 1e-12
                && (row[0].gradient[1] - 0.75_f64.sqrt()).abs() < 1e-12
        );
        assert!(SmoothUnion::new(a, b, 0.0).is_err());
    }

    #[test]
    fn regions_follow_the_part_that_shapes_the_surface() {
        let drilled = Difference::new(
            Tagged::new(Cuboid::new([0.0; 3], [0.6; 3]).unwrap(), 10),
            Tagged::new(
                Cylinder::new([0.0; 3], [0.0, 0.0, 1.0], 0.3, 1.0).unwrap(),
                20,
            ),
        );
        assert_eq!(drilled.eval_crease([0.6, 0.1, 0.2]).region, 10);
        assert_eq!(drilled.eval_crease([0.3, 0.0, 0.2]).region, 20);
        // Tags change no value.
        let untagged = Difference::new(drilled.left.field, drilled.right.field);
        let p = [0.25, 0.1, 0.4];
        assert_eq!(drilled.eval(p), untagged.eval(p));
        assert_eq!(untagged.eval_crease(p).region, 0);

        let (a, b) = spheres();
        let blend = SmoothUnion::new(Tagged::new(a, 1), Tagged::new(b, 2), 0.4).unwrap();
        assert_eq!(blend.eval_crease([-1.4, 0.1, 0.0]).region, 1);
        assert_eq!(blend.eval_crease([1.4, 0.1, 0.0]).region, 2);
        // Within the blend, the lesser part's region, even when it has none.
        let half = SmoothUnion::new(Tagged::new(a, 7), b, 0.4).unwrap();
        assert_eq!(half.eval_crease([0.1, 0.0, 0.0]).region, 0);
        assert_eq!(half.eval_crease([-0.1, 0.0, 0.0]).region, 7);
    }
}
