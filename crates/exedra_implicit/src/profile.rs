// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Two-dimensional profiles, and solids lifted from them.

use crate::bounds::{outward, outward2};
use crate::claims::{Claims, Metric, ProfileClaims, Support};
use crate::formula::{Formula, Profile};
use crate::number::Number;
use crate::vector::box_distance;
use crate::{FieldError, check_finite, check_positive};

/// A disk.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Circle {
    center: [f64; 2],
    radius: f64,
}

impl Circle {
    /// The disk of `radius` about `center`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a parameter is not finite or the radius
    /// is not positive.
    pub fn new(center: [f64; 2], radius: f64) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        check_positive("radius", radius)?;
        Ok(Self { center, radius })
    }

    /// The center.
    #[must_use]
    pub const fn center(&self) -> [f64; 2] {
        self.center
    }

    /// The radius.
    #[must_use]
    pub const fn radius(&self) -> f64 {
        self.radius
    }
}

impl Profile for Circle {
    fn eval<N: Number>(&self, [x, y]: [N; 2]) -> N {
        (x - self.center[0]).hypot(y - self.center[1]) - self.radius
    }

    fn claims(&self) -> ProfileClaims {
        exact_about(self.center, [self.radius; 2])
    }
}

/// An axis-aligned rectangle, whose sides meet in creases.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rectangle {
    center: [f64; 2],
    half_extents: [f64; 2],
}

impl Rectangle {
    /// The rectangle reaching `half_extents` from `center` along each axis.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a parameter is not finite or a half
    /// extent is not positive.
    pub fn new(center: [f64; 2], half_extents: [f64; 2]) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        for extent in half_extents {
            check_positive("half_extents", extent)?;
        }
        Ok(Self {
            center,
            half_extents,
        })
    }

    /// The center.
    #[must_use]
    pub const fn center(&self) -> [f64; 2] {
        self.center
    }

    /// The distance from the center to each side.
    #[must_use]
    pub const fn half_extents(&self) -> [f64; 2] {
        self.half_extents
    }
}

impl Profile for Rectangle {
    fn eval<N: Number>(&self, p: [N; 2]) -> N {
        box_distance::<N, 2>(core::array::from_fn(|i| {
            (p[i] - self.center[i]).abs() - self.half_extents[i]
        }))
    }

    fn claims(&self) -> ProfileClaims {
        exact_about(self.center, self.half_extents)
    }
}

/// The plane on one side of a line.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct HalfPlane {
    point: [f64; 2],
    normal: [f64; 2],
}

impl HalfPlane {
    /// Everything behind the line through `point` facing `normal`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a parameter is not finite or the normal
    /// has no direction.
    pub fn new(point: [f64; 2], normal: [f64; 2]) -> Result<Self, FieldError> {
        check_finite("point", point)?;
        check_finite("normal", normal)?;
        let [x, y, _] = crate::check_direction("normal", [normal[0], normal[1], 0.0])?;
        Ok(Self {
            point,
            normal: [x, y],
        })
    }

    /// A point on the line.
    #[must_use]
    pub const fn point(&self) -> [f64; 2] {
        self.point
    }

    /// The unit outward normal.
    #[must_use]
    pub const fn normal(&self) -> [f64; 2] {
        self.normal
    }
}

impl Profile for HalfPlane {
    fn eval<N: Number>(&self, [x, y]: [N; 2]) -> N {
        (x - self.point[0]) * self.normal[0] + (y - self.point[1]) * self.normal[1]
    }

    fn claims(&self) -> ProfileClaims {
        Claims {
            metric: Metric::ExactDistance,
            support: Support::Unbounded,
        }
    }
}

/// A profile in the `x`-`y` plane extruded along `z`, capped at both ends.
///
/// The caps meet the profile's sides in creases.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Extrude<P> {
    profile: P,
    half_height: f64,
}

impl<P> Extrude<P> {
    /// `profile` extruded from `-half_height` to `half_height` along `z`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the half height is not finite and
    /// positive.
    pub fn new(profile: P, half_height: f64) -> Result<Self, FieldError> {
        check_positive("half_height", half_height)?;
        Ok(Self {
            profile,
            half_height,
        })
    }

    /// The profile extruded.
    #[must_use]
    pub const fn profile(&self) -> &P {
        &self.profile
    }

    /// The distance from the middle to each cap.
    #[must_use]
    pub const fn half_height(&self) -> f64 {
        self.half_height
    }
}

impl<P: Profile> Formula for Extrude<P> {
    fn eval<N: Number>(&self, [x, y, z]: [N; 3]) -> N {
        box_distance([self.profile.eval([x, y]), z.abs() - self.half_height])
    }

    /// The profile's distance and the distance along `z` meet at right
    /// angles, so the extrusion changes no faster than the faster of them.
    fn claims(&self) -> Claims {
        let claims = self.profile.claims();
        let metric = match claims.metric {
            Metric::ExactDistance => Metric::ExactDistance,
            metric => metric.combined(Metric::ExactDistance),
        };
        let h = self.half_height;
        let support = match claims.support {
            Support::Unbounded => Support::Unbounded,
            support => support.map(|b| outward([b.min[0], b.min[1], -h], [b.max[0], b.max[1], h])),
        };
        Claims { metric, support }
    }
}

/// A profile in the radius-height plane revolved about the `y` axis.
///
/// The profile is evaluated at the distance from the axis and the height,
/// so only its part at non-negative radius shapes the solid.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Revolve<P> {
    profile: P,
}

impl<P> Revolve<P> {
    /// `profile` revolved about the `y` axis.
    #[must_use]
    pub const fn new(profile: P) -> Self {
        Self { profile }
    }

    /// The profile revolved.
    #[must_use]
    pub const fn profile(&self) -> &P {
        &self.profile
    }
}

impl<P: Profile> Formula for Revolve<P> {
    fn eval<N: Number>(&self, [x, y, z]: [N; 3]) -> N {
        self.profile.eval([x.hypot(z), y])
    }

    /// The distance from the axis changes no faster than position, so the
    /// profile's Lipschitz factor carries over; exactness does not, since a
    /// profile reaching past the axis is folded onto itself.
    fn claims(&self) -> Claims {
        let claims = self.profile.claims();
        let metric = match claims.metric {
            Metric::ExactDistance => Metric::Lipschitz(1.0),
            metric => metric,
        };
        let support = match claims.support {
            Support::Unbounded => Support::Unknown,
            support => support.map(|b| {
                let r = b.min[0].abs().max(b.max[0].abs());
                outward([-r, b.min[1], -r], [r, b.max[1], r])
            }),
        };
        Claims { metric, support }
    }
}

/// Exact-distance claims for a profile bounded by `center` and `extent`.
fn exact_about(center: [f64; 2], extent: [f64; 2]) -> ProfileClaims {
    let min = core::array::from_fn(|i| center[i] - extent[i]);
    let max = core::array::from_fn(|i| center[i] + extent[i]);
    outward2(min, max).map_or(Claims::UNKNOWN, Claims::exact)
}

#[cfg(test)]
mod tests {
    use super::{Circle, Extrude, HalfPlane, Rectangle, Revolve};
    use crate::{Formula, Metric, Profile, ScalarField, Support};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn profiles_are_exact_distances() {
        let circle = Circle::new([1.0, 0.0], 0.5).unwrap();
        assert!(close(circle.eval([1.0, 2.0]), 1.5));
        let rectangle = Rectangle::new([0.0; 2], [1.0, 2.0]).unwrap();
        assert!(close(rectangle.eval([2.0, 3.0]), 2.0_f64.sqrt()));
        assert!(close(rectangle.eval([0.5, 0.0]), -0.5));
        let line = HalfPlane::new([0.0, 1.0], [0.0, 2.0]).unwrap();
        assert!(close(line.eval([3.0, 4.0]), 3.0));
        assert!(HalfPlane::new([0.0; 2], [0.0; 2]).is_err());
    }

    #[test]
    fn extrusions_cap_their_profiles() {
        let rod = Extrude::new(Circle::new([0.0; 2], 1.0).unwrap(), 2.0).unwrap();
        assert!(close(rod.eval([2.0, 0.0, 0.0]), 1.0));
        assert!(close(rod.eval([0.0, 0.0, 3.0]), 1.0));
        assert!(close(rod.eval([2.0, 0.0, 3.0]), 2.0_f64.sqrt()));
        assert!(close(rod.eval([0.0, 0.0, 1.5]), -0.5));
        let claims = Formula::claims(&rod);
        assert_eq!(claims.metric, Metric::ExactDistance);
        let Support::Bounded(bounds) = claims.support else {
            panic!("bounded");
        };
        assert!(bounds.contains([1.0, 0.0, 2.0]) && !bounds.contains([0.0, 0.0, 2.1]));
        // The caps meet the side in a crease.
        let rim = rod.eval_crease([1.0, 0.0, 2.0]);
        assert!(close(rim.ties[0].unwrap().function.value, 0.0));
    }

    #[test]
    fn revolutions_turn_profiles_about_the_axis() {
        let ring = Revolve::new(Circle::new([2.0, 0.0], 0.5).unwrap());
        assert!(close(ring.eval([0.0, 0.0, 2.5]), 0.0));
        assert!(close(ring.eval([-2.0, 0.0, 0.0]), -0.5));
        let Support::Bounded(bounds) = Formula::claims(&ring).support else {
            panic!("bounded");
        };
        assert!(bounds.contains([-2.5, 0.0, 0.0]) && bounds.contains([0.0, 0.5, 2.5]));
    }
}
