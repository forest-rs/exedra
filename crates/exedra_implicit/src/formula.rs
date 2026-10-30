// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fields written once over any [`Number`].

use alloc::boxed::Box;

use crate::Aabb;
use crate::claims::{Claims, ProfileClaims};
use crate::field::{Crease, ScalarField, around};
use crate::number::{Creased, Dual, Interval, Number};

/// A field written once over any [`Number`].
///
/// One definition answers every question a consumer asks: evaluated in
/// `f64` it gives values, in [`Dual`] exact gradients, in [`Interval`]
/// bounds over a box, in `Dual<Interval>` bounds on the gradient, and on an
/// internal crease-tracking number the creases its `min`, `max` and `abs`
/// mark. Every formula is therefore a [`ScalarField`].
///
/// See [`crate::number`] for what a formula may use: arithmetic, `min`,
/// `max`, `abs`, `select`, `sqrt`, `hypot`, `sin` and `cos`, and no
/// branches on values.
///
/// ```
/// use exedra_implicit::{Formula, Metric, Number, ScalarField, Claims, Support};
///
/// // A gyroid sheet: each gradient component is at most sqrt(2), and
/// // together at most sqrt(3).
/// struct Gyroid;
///
/// impl Formula for Gyroid {
///     fn eval<N: Number>(&self, [x, y, z]: [N; 3]) -> N {
///         x.sin() * y.cos() + y.sin() * z.cos() + z.sin() * x.cos()
///     }
///
///     fn claims(&self) -> Claims {
///         Claims { metric: Metric::Lipschitz(3.0_f64.sqrt()), support: Support::Unknown }
///     }
/// }
///
/// let mut row = [Default::default()];
/// Gyroid.eval_gradients(&[[0.0; 3]], &mut row);
/// assert_eq!(row[0].gradient, [1.0, 1.0, 1.0]);
/// ```
pub trait Formula {
    /// The field's value at `p`, negative inside.
    fn eval<N: Number>(&self, p: [N; 3]) -> N;

    /// The field's metric and support claims.
    ///
    /// The default claims nothing, which is always truthful. Override it
    /// only with claims that hold at every point.
    fn claims(&self) -> Claims {
        Claims::UNKNOWN
    }
}

impl<F: Formula> Formula for &F {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        (**self).eval(p)
    }

    fn claims(&self) -> Claims {
        (**self).claims()
    }
}

impl<F: Formula> Formula for Box<F> {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        (**self).eval(p)
    }

    fn claims(&self) -> Claims {
        (**self).claims()
    }
}

/// A two-dimensional field written once over any [`Number`]: a profile that
/// [`Extrude`](crate::Extrude) and [`Revolve`](crate::Revolve) lift into
/// space.
pub trait Profile {
    /// The profile's value at `p`, negative inside.
    fn eval<N: Number>(&self, p: [N; 2]) -> N;

    /// The profile's metric and support claims.
    ///
    /// The default claims nothing.
    fn claims(&self) -> ProfileClaims {
        Claims::UNKNOWN
    }
}

impl<P: Profile> Profile for &P {
    fn eval<N: Number>(&self, p: [N; 2]) -> N {
        (**self).eval(p)
    }

    fn claims(&self) -> ProfileClaims {
        (**self).claims()
    }
}

impl<F: Formula> ScalarField for F {
    fn claims(&self) -> Claims {
        Formula::claims(self)
    }

    fn eval_points(&self, points: &[[f64; 3]], out: &mut [f64]) {
        assert_eq!(points.len(), out.len(), "one value per point");
        for (point, out) in points.iter().zip(out) {
            *out = self.eval(*point);
        }
    }

    fn eval_gradients(&self, points: &[[f64; 3]], out: &mut [Dual]) {
        assert_eq!(points.len(), out.len(), "one row per point");
        for (point, out) in points.iter().zip(out) {
            *out = self.eval(Dual::point(*point));
        }
    }

    /// The tightest of three sound bounds: the formula evaluated on
    /// intervals, the mean-value form `F(c) + ∇F(box)·(box - c)` about the
    /// center `c`, which tightens with the square of the box, and the
    /// Lipschitz claim.
    fn eval_interval(&self, region: &Aabb) -> Interval {
        bounds(self, region).value
    }

    fn eval_gradient_interval(&self, region: &Aabb) -> Dual<Interval> {
        bounds(self, region)
    }

    fn eval_crease(&self, point: [f64; 3]) -> Crease {
        let creased = self.eval(Creased::point(point));
        Crease {
            field: creased.value,
            ties: creased.ties.ties(),
            region: creased.region,
        }
    }
}

/// Bounds of `formula` and its gradient over `region`: the natural and
/// mean-value forms and the Lipschitz claim intersected.
fn bounds<F: Formula>(formula: &F, region: &Aabb) -> Dual<Interval> {
    let Dual {
        mut value,
        mut gradient,
    } = formula.eval(Dual::point(region.intervals()));
    let center = region.center();
    let at_center = formula.eval(center.map(Interval::constant));
    let mean_value = (0..3).fold(at_center, |sum, axis| {
        let offset = Interval::new(region.min[axis], region.max[axis]) - center[axis];
        sum + gradient[axis] * offset
    });
    value = value.intersect(mean_value).unwrap_or(value);
    if let Some(lipschitz) = Formula::claims(formula).metric.lipschitz() {
        let claimed = around(formula.eval(center), lipschitz, region);
        value = value.intersect(claimed).unwrap_or(value);
        let slope = Interval::new(-lipschitz, lipschitz);
        gradient = gradient.map(|g| g.intersect(slope).unwrap_or(g));
    }
    Dual { value, gradient }
}

/// A [`ScalarField`] that is not a formula, joining formulas as a leaf.
///
/// Composing a field into a formula, as with [`Union`](crate::Union) or
/// [`Translate`](crate::Translate), needs it in every number. `Opaque`
/// answers each from the field's own queries: values from
/// [`eval_points`](ScalarField::eval_points), gradients from
/// [`eval_gradients`](ScalarField::eval_gradients) through the chain rule,
/// and bounds from its interval queries, so the composite is exactly as
/// precise as the field allows.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Opaque<S>(pub S);

impl<S: ScalarField> Formula for Opaque<S> {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        N::sample(&self.0, p)
    }

    fn claims(&self) -> Claims {
        self.0.claims()
    }
}

#[cfg(test)]
mod tests {
    use exedra_math::Placement3;

    use super::Opaque;
    use crate::{
        Aabb, Claims, Cuboid, Dual, Formula, Metric, Placed, ScalarField, Sphere, Support, Union,
    };

    /// A ball answering only point queries, as compiled or sampled fields
    /// may.
    struct Sampled;

    impl ScalarField for Sampled {
        fn claims(&self) -> Claims {
            Claims {
                metric: Metric::Lipschitz(1.0),
                support: Support::Unknown,
            }
        }

        fn eval_points(&self, points: &[[f64; 3]], out: &mut [f64]) {
            for (p, out) in points.iter().zip(out) {
                *out = Sphere::new([0.0; 3], 0.5).unwrap().eval(*p);
            }
        }

        fn eval_gradients(&self, points: &[[f64; 3]], out: &mut [Dual]) {
            Sphere::new([0.0; 3], 0.5)
                .unwrap()
                .eval_gradients(points, out);
        }
    }

    #[test]
    fn opaque_leaves_compose_through_the_chain_rule() {
        let turned = Placed::new(
            Opaque(Sampled),
            Placement3::rotate_z_then_translate(0.7, 1.0, 0.0, 0.0),
        )
        .unwrap();
        let composite = Union::new(turned, Cuboid::new([-2.0, 0.0, 0.0], [0.3; 3]).unwrap());
        let p = [1.3, 0.2, -0.1];
        let mut row = [Dual::default()];
        composite.eval_gradients(&[p], &mut row);
        let reference = Sphere::new([1.0, 0.0, 0.0], 0.5).unwrap();
        let mut expected = [Dual::default()];
        reference.eval_gradients(&[p], &mut expected);
        assert!((row[0].value - expected[0].value).abs() < 1e-15);
        for axis in 0..3 {
            assert!((row[0].gradient[axis] - expected[0].gradient[axis]).abs() < 1e-12);
        }
        // Bounds come from the leaf's claim: sound, if looser than a
        // formula's own.
        let region = Aabb::new([1.2, 0.0, -0.2], [1.6, 0.3, 0.1]).unwrap();
        let bounds = composite.eval_interval(&region);
        for corner in region.corners() {
            assert!(bounds.contains(reference.eval(corner)));
        }
        let slopes = composite.eval_gradient_interval(&region);
        assert!(
            slopes
                .gradient
                .iter()
                .all(|g| g.low() >= -1.0 - 1e-9 && g.high() <= 1.0 + 1e-9)
        );
        assert_eq!(Formula::claims(&composite).metric, Metric::Lipschitz(1.0));
    }
}
