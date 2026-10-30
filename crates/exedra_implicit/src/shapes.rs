// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact distances to solid shapes.
//!
//! Each shape checks its parameters on construction and states exact
//! distance and a bounding box as its [`Claims`].

use exedra_math::Real;

use crate::bounds::outward;
use crate::claims::{Claims, Metric, Support, exact_about};
use crate::formula::Formula;
use crate::number::Number;
use crate::vector::{along, box_distance, length, less, max3, min3, offset, unit};
use crate::{FieldError, check_direction, check_finite, check_nonnegative, check_positive};

/// A solid ball.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Sphere {
    center: [f64; 3],
    radius: f64,
}

impl Sphere {
    /// The ball of `radius` about `center`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a parameter is not finite or the radius
    /// is not positive.
    pub fn new(center: [f64; 3], radius: f64) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        check_positive("radius", radius)?;
        Ok(Self { center, radius })
    }

    /// The center.
    #[must_use]
    pub const fn center(&self) -> [f64; 3] {
        self.center
    }

    /// The radius.
    #[must_use]
    pub const fn radius(&self) -> f64 {
        self.radius
    }
}

impl Formula for Sphere {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        length(offset(p, self.center)) - self.radius
    }

    fn claims(&self) -> Claims {
        exact_about(self.center, [self.radius; 3])
    }
}

/// A solid axis-aligned box, whose faces meet in creases.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Cuboid {
    center: [f64; 3],
    half_extents: [f64; 3],
}

impl Cuboid {
    /// The box reaching `half_extents` from `center` along each axis.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a parameter is not finite or a half
    /// extent is not positive.
    pub fn new(center: [f64; 3], half_extents: [f64; 3]) -> Result<Self, FieldError> {
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
    pub const fn center(&self) -> [f64; 3] {
        self.center
    }

    /// The distance from the center to each face.
    #[must_use]
    pub const fn half_extents(&self) -> [f64; 3] {
        self.half_extents
    }
}

impl Formula for Cuboid {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        let d = offset(p, self.center);
        box_distance(core::array::from_fn::<N, 3, _>(|i| {
            d[i].abs() - self.half_extents[i]
        }))
    }

    fn claims(&self) -> Claims {
        exact_about(self.center, self.half_extents)
    }
}

/// A solid finite cylinder, whose side meets its flat ends in creases.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Cylinder {
    center: [f64; 3],
    axis: [f64; 3],
    radius: f64,
    half_height: f64,
}

impl Cylinder {
    /// The cylinder of `radius` about `axis` through `center`, reaching
    /// `half_height` along the axis either way.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a parameter is not finite, the axis has
    /// no direction, or a dimension is not positive.
    pub fn new(
        center: [f64; 3],
        axis: [f64; 3],
        radius: f64,
        half_height: f64,
    ) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        let axis = check_direction("axis", axis)?;
        check_positive("radius", radius)?;
        check_positive("half_height", half_height)?;
        Ok(Self {
            center,
            axis,
            radius,
            half_height,
        })
    }

    /// The center.
    #[must_use]
    pub const fn center(&self) -> [f64; 3] {
        self.center
    }

    /// The unit axis.
    #[must_use]
    pub const fn axis(&self) -> [f64; 3] {
        self.axis
    }

    /// The radius.
    #[must_use]
    pub const fn radius(&self) -> f64 {
        self.radius
    }

    /// The distance from the center to each end.
    #[must_use]
    pub const fn half_height(&self) -> f64 {
        self.half_height
    }
}

impl Formula for Cylinder {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        let d = offset(p, self.center);
        let height = along(d, self.axis);
        let radial = length(less(d, self.axis, height));
        box_distance([radial - self.radius, height.abs() - self.half_height])
    }

    fn claims(&self) -> Claims {
        exact_about(
            self.center,
            disk_extent(self.axis, self.radius, self.half_height),
        )
    }
}

/// A solid torus: a ring of tube around an axis.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Torus {
    center: [f64; 3],
    axis: [f64; 3],
    major_radius: f64,
    minor_radius: f64,
}

impl Torus {
    /// The torus about `axis` through `center`, its tube of `minor_radius`
    /// running `major_radius` from the axis.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a parameter is not finite, the axis has
    /// no direction, or a radius is not positive.
    pub fn new(
        center: [f64; 3],
        axis: [f64; 3],
        major_radius: f64,
        minor_radius: f64,
    ) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        let axis = check_direction("axis", axis)?;
        check_positive("major_radius", major_radius)?;
        check_positive("minor_radius", minor_radius)?;
        Ok(Self {
            center,
            axis,
            major_radius,
            minor_radius,
        })
    }

    /// The center.
    #[must_use]
    pub const fn center(&self) -> [f64; 3] {
        self.center
    }

    /// The unit axis.
    #[must_use]
    pub const fn axis(&self) -> [f64; 3] {
        self.axis
    }

    /// The distance from the axis to the middle of the tube.
    #[must_use]
    pub const fn major_radius(&self) -> f64 {
        self.major_radius
    }

    /// The tube's radius.
    #[must_use]
    pub const fn minor_radius(&self) -> f64 {
        self.minor_radius
    }
}

impl Formula for Torus {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        let d = offset(p, self.center);
        let height = along(d, self.axis);
        let radial = length(less(d, self.axis, height));
        (radial - self.major_radius).hypot(height) - self.minor_radius
    }

    /// An exact distance unless the tube reaches the axis: a spindle torus
    /// folds over itself there, and the value inside, its distance to the
    /// tube's middle circle less the tube's radius, only bounds the
    /// distance to the surface.
    fn claims(&self) -> Claims {
        let claims = exact_about(
            self.center,
            disk_extent(
                self.axis,
                self.major_radius + self.minor_radius,
                self.minor_radius,
            ),
        );
        if self.minor_radius > self.major_radius {
            Claims {
                metric: Metric::Lipschitz(1.0),
                ..claims
            }
        } else {
            claims
        }
    }
}

/// The solid on one side of a plane.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct HalfSpace {
    point: [f64; 3],
    normal: [f64; 3],
}

impl HalfSpace {
    /// Everything behind the plane through `point` facing `normal`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a parameter is not finite or the normal
    /// has no direction.
    pub fn new(point: [f64; 3], normal: [f64; 3]) -> Result<Self, FieldError> {
        check_finite("point", point)?;
        let normal = check_direction("normal", normal)?;
        Ok(Self { point, normal })
    }

    /// A point on the plane.
    #[must_use]
    pub const fn point(&self) -> [f64; 3] {
        self.point
    }

    /// The unit outward normal.
    #[must_use]
    pub const fn normal(&self) -> [f64; 3] {
        self.normal
    }
}

impl Formula for HalfSpace {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        along(offset(p, self.point), self.normal)
    }

    fn claims(&self) -> Claims {
        Claims {
            metric: Metric::ExactDistance,
            support: Support::Unbounded,
        }
    }
}

/// Everything within a radius of a segment.
///
/// A zero-length segment is a ball.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Capsule {
    a: [f64; 3],
    b: [f64; 3],
    radius: f64,
}

impl Capsule {
    /// Everything within `radius` of the segment from `a` to `b`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a parameter is not finite or the radius
    /// is not positive.
    pub fn new(a: [f64; 3], b: [f64; 3], radius: f64) -> Result<Self, FieldError> {
        check_finite("a", a)?;
        check_finite("b", b)?;
        check_positive("radius", radius)?;
        Ok(Self { a, b, radius })
    }

    /// The segment's first end.
    #[must_use]
    pub const fn a(&self) -> [f64; 3] {
        self.a
    }

    /// The segment's second end.
    #[must_use]
    pub const fn b(&self) -> [f64; 3] {
        self.b
    }

    /// The radius.
    #[must_use]
    pub const fn radius(&self) -> f64 {
        self.radius
    }
}

impl Formula for Capsule {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        let pa = offset(p, self.a);
        let Some((direction, span)) = segment(self.a, self.b) else {
            return length(pa) - self.radius;
        };
        let t = unit(along(pa, direction) / span) * span;
        length(less(pa, direction, t)) - self.radius
    }

    fn claims(&self) -> Claims {
        segment_claims(self.a, self.b, self.radius)
    }
}

/// The convex hull of two balls: a cone with rounded ends.
///
/// Where one ball holds the other, it is that ball.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RoundCone {
    a: [f64; 3],
    b: [f64; 3],
    radius_a: f64,
    radius_b: f64,
}

impl RoundCone {
    /// The hull of the ball of `radius_a` about `a` and of `radius_b` about
    /// `b`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a parameter is not finite or a radius is
    /// negative.
    pub fn new(a: [f64; 3], b: [f64; 3], radius_a: f64, radius_b: f64) -> Result<Self, FieldError> {
        check_finite("a", a)?;
        check_finite("b", b)?;
        check_nonnegative("radius_a", radius_a)?;
        check_nonnegative("radius_b", radius_b)?;
        Ok(Self {
            a,
            b,
            radius_a,
            radius_b,
        })
    }

    /// The first ball's center.
    #[must_use]
    pub const fn a(&self) -> [f64; 3] {
        self.a
    }

    /// The second ball's center.
    #[must_use]
    pub const fn b(&self) -> [f64; 3] {
        self.b
    }

    /// The first ball's radius.
    #[must_use]
    pub const fn radius_a(&self) -> f64 {
        self.radius_a
    }

    /// The second ball's radius.
    #[must_use]
    pub const fn radius_b(&self) -> f64 {
        self.radius_b
    }
}

impl Formula for RoundCone {
    /// Quilez's exact round-cone distance: the ball about `b`, the ball
    /// about `a`, or the cone between them, by which region `p` lies in.
    /// The regions meet where their distances agree in value and gradient.
    ///
    /// Lengths are taken along the unit axis and compared in units of the
    /// span, so nothing underflows or overflows at any scale.
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        let rr = self.radius_a - self.radius_b;
        let pa = offset(p, self.a);
        let cap_a = length(pa) - self.radius_a;
        let cap_b = length(offset(p, self.b)) - self.radius_b;
        let segment = segment(self.a, self.b).filter(|&(_, span)| span > rr.abs());
        let Some((direction, span)) = segment else {
            return if self.radius_a >= self.radius_b {
                cap_a
            } else {
                cap_b
            };
        };
        // The cone's half-angle: its side leans by `cos` toward the axis.
        let cos = rr / span;
        let sin = Real::sqrt((1.0 - cos) * (1.0 + cos));
        let y = along(pa, direction);
        let z = y - span;
        let x = length(less(pa, direction, y));
        // Past `b` when sign(z) sin² z² > k x², before `a` when
        // sign(y) sin² y² < k x², with k = sign(cos) cos²; only the signs
        // matter, so lengths are taken in units of the span.
        let (yn, zn, xn) = (y / span, z / span, x / span);
        let x2 = xn.square() * (cos.signum() * cos * cos);
        let past_b = zn * zn.abs() * (sin * sin) - x2;
        let before_a = yn * yn.abs() * (sin * sin) - x2;
        let side = x * sin + y * cos - self.radius_a;
        (-past_b).select(cap_b, before_a.select(cap_a, side))
    }

    fn claims(&self) -> Claims {
        segment_claims(self.a, self.b, self.radius_a.max(self.radius_b))
    }
}

/// How far a disk or cylinder of `radius` about the unit `axis`, reaching
/// `half_height` along it, extends along each coordinate axis, with a
/// margin for rounding.
fn disk_extent(axis: [f64; 3], radius: f64, half_height: f64) -> [f64; 3] {
    core::array::from_fn(|i| {
        // `sqrt(1 - axis[i]^2)`, without the cancellation near one.
        let across = axis[(i + 1) % 3].hypot(axis[(i + 2) % 3]);
        (radius * across + half_height * axis[i].abs()) * (1.0 + 8.0 * f64::EPSILON)
    })
}

/// The unit direction from `a` to `b` and the distance between them, or
/// `None` where they coincide; scaled, so neither underflows nor overflows.
fn segment(a: [f64; 3], b: [f64; 3]) -> Option<([f64; 3], f64)> {
    let span: [f64; 3] = core::array::from_fn(|i| b[i] - a[i]);
    let largest = span.iter().fold(0.0_f64, |m, c| m.max(c.abs()));
    if largest == 0.0 {
        return None;
    }
    let scaled = span.map(|c| c / largest);
    let norm = Real::sqrt(exedra_math::dot(scaled, scaled));
    Some((scaled.map(|c| c / norm), largest * norm))
}

/// Exact claims for a shape within `radius` of the segment from `a` to `b`.
fn segment_claims(a: [f64; 3], b: [f64; 3], radius: f64) -> Claims {
    let min = min3(a, b).map(|v| v - radius);
    let max = max3(a, b).map(|v| v + radius);
    outward(min, max).map_or(Claims::UNKNOWN, Claims::exact)
}

#[cfg(test)]
mod tests {
    use super::{Capsule, Cuboid, Cylinder, HalfSpace, RoundCone, Sphere, Torus};
    use crate::{Aabb, Dual, Formula, Interval, Metric, ScalarField, Support};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn shapes_are_exact_distances() {
        let sphere = Sphere::new([1.0, 0.0, 0.0], 0.5).unwrap();
        assert!(close(sphere.eval([1.0, 2.0, 0.0]), 1.5));
        let cuboid = Cuboid::new([0.0; 3], [1.0, 2.0, 3.0]).unwrap();
        assert!(close(cuboid.eval([2.0, 0.0, 0.0]), 1.0));
        assert!(close(cuboid.eval([2.0, 3.0, 0.0]), 2.0_f64.sqrt()));
        assert!(close(cuboid.eval([0.5, 0.0, 0.0]), -0.5));
        let cylinder = Cylinder::new([0.0; 3], [0.0, 0.0, 2.0], 1.0, 1.0).unwrap();
        assert!(close(cylinder.eval([3.0, 0.0, 0.0]), 2.0));
        assert!(close(cylinder.eval([0.0, 0.0, 3.0]), 2.0));
        assert!(close(cylinder.eval([0.0, 0.0, 0.5]), -0.5));
        let torus = Torus::new([0.0; 3], [0.0, 1.0, 0.0], 2.0, 0.5).unwrap();
        assert!(close(torus.eval([2.0, 1.0, 0.0]), 0.5));
        let plane = HalfSpace::new([0.0, 0.0, 1.0], [0.0, 0.0, 3.0]).unwrap();
        assert!(close(plane.eval([5.0, 5.0, 3.0]), 2.0));
        let capsule = Capsule::new([0.0; 3], [2.0, 0.0, 0.0], 0.5).unwrap();
        assert!(close(capsule.eval([1.0, 1.0, 0.0]), 0.5));
        assert!(close(capsule.eval([3.0, 0.0, 0.0]), 0.5));
        let point = Capsule::new([1.0; 3], [1.0; 3], 0.5).unwrap();
        assert!(close(point.eval([1.0, 1.0, 2.0]), 0.5));
    }

    #[test]
    fn round_cones_follow_their_regions() {
        let cone = RoundCone::new([0.0; 3], [4.0, 0.0, 0.0], 1.0, 0.5).unwrap();
        // Beyond each end, the distance to that end's ball.
        assert!(close(cone.eval([-3.0, 0.0, 0.0]), 2.0));
        assert!(close(cone.eval([6.0, 0.0, 0.0]), 1.5));
        // Beside the cone, the distance to its slanted side.
        let slope: f64 = 0.5 / 4.0;
        let cos = (1.0 - slope * slope).sqrt();
        let along_side = cone.eval([2.0, 3.0, 0.0]);
        let expected = 3.0 * cos + 2.0 * slope - 1.0;
        assert!(
            close(along_side, expected),
            "{along_side} against {expected}"
        );
        // One ball holding the other is that ball.
        let held = RoundCone::new([0.0; 3], [0.5, 0.0, 0.0], 2.0, 0.5).unwrap();
        assert!(close(held.eval([0.0, 3.0, 0.0]), 1.0));
        // Outside, the distance to the nearest of the balls swept between
        // the ends, whose union the cone is.
        for p in [
            [1.0, 1.5, 0.3],
            [3.9, 0.7, -0.2],
            [4.3, 0.2, 0.1],
            [-0.4, 1.0, 0.0],
        ] {
            let swept = (0..=100_000)
                .map(|i| {
                    let t = f64::from(i) / 100_000.0;
                    let c = [4.0 * t, 0.0, 0.0];
                    let r = 1.0 - 0.5 * t;
                    ((p[0] - c[0]).powi(2) + p[1] * p[1] + p[2] * p[2]).sqrt() - r
                })
                .fold(f64::INFINITY, f64::min);
            assert!((cone.eval(p) - swept).abs() < 1e-8, "{p:?}");
        }
    }

    #[test]
    fn gradients_inside_boxes_are_finite() {
        let cuboid = Cuboid::new([0.0; 3], [1.0; 3]).unwrap();
        let mut row = [Dual::default()];
        cuboid.eval_gradients(&[[0.5, 0.1, 0.2]], &mut row);
        assert_eq!(row[0].gradient, [1.0, 0.0, 0.0]);
        cuboid.eval_gradients(&[[2.0, 0.1, 0.2]], &mut row);
        assert_eq!(row[0].gradient, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn faces_are_not_creases_but_edges_are() {
        let cuboid = Cuboid::new([0.0; 3], [1.0; 3]).unwrap();
        // The middle of a face: the nearest tie is a full face length away.
        let face = cuboid.eval_crease([1.0, 0.0, 0.0]);
        assert!(
            face.ties.iter().flatten().all(|tie| tie.distance > 0.5),
            "{face:?}"
        );
        // On an edge, a tie is zero.
        let edge = cuboid.eval_crease([1.0, 1.0, 0.3]);
        assert!(close(edge.ties[0].unwrap().distance, 0.0), "{edge:?}");
        // Outside, beyond the edge, its tie is still the nearest: zero
        // where the faces' planes are equally far.
        let beyond = cuboid.eval_crease([1.1, 1.1, 0.3]).ties[0].unwrap();
        assert!(close(beyond.function.value, 0.0), "{beyond:?}");
        assert_eq!(beyond.function.gradient, [1.0, -1.0, 0.0]);
        let rim = Cylinder::new([0.0; 3], [0.0, 0.0, 1.0], 1.0, 1.0)
            .unwrap()
            .eval_crease([1.1, 0.0, 1.1]);
        assert!(close(rim.ties[0].unwrap().distance, 0.0), "{rim:?}");
        // A corner has two.
        let corner = cuboid.eval_crease([1.0, 1.0, 1.0]);
        assert!(
            corner
                .ties
                .iter()
                .all(|tie| close(tie.unwrap().distance, 0.0)),
            "{corner:?}"
        );
        // A capsule is smooth everywhere.
        let capsule = Capsule::new([0.0; 3], [2.0, 0.0, 0.0], 0.5).unwrap();
        assert_eq!(capsule.eval_crease([2.0, 0.5, 0.0]).ties, [None, None]);
    }

    #[test]
    fn segments_keep_their_scale() {
        for scale in [1e-200, 1.0, 1e100] {
            let capsule = Capsule::new([0.0; 3], [scale, 0.0, 0.0], 0.1 * scale).unwrap();
            for end in [[0.0; 3], [scale, 0.0, 0.0]] {
                let value = capsule.eval(end);
                assert!((value / scale + 0.1).abs() < 1e-12, "{scale}: {value}");
            }
            assert!((capsule.eval([0.5 * scale, scale, 0.0]) / scale - 0.9).abs() < 1e-12);
            let cone =
                RoundCone::new([0.0; 3], [scale, 0.0, 0.0], 0.2 * scale, 0.1 * scale).unwrap();
            assert!(
                (cone.eval([-scale, 0.0, 0.0]) / scale - 0.8).abs() < 1e-12,
                "{scale}"
            );
            assert!((cone.eval([2.0 * scale, 0.0, 0.0]) / scale - 0.9).abs() < 1e-12);
        }
    }

    #[test]
    fn tiny_spheres_keep_their_sign() {
        let sphere = Sphere::new([0.0; 3], 1e-200).unwrap();
        assert_eq!(sphere.eval([2e-200, 0.0, 0.0]), 1e-200);
        assert_eq!(sphere.eval([0.0, 0.0, 0.5e-200]), -0.5e-200);
    }

    #[test]
    fn spindle_tori_claim_only_a_lipschitz_bound() {
        let spindle = Torus::new([0.0; 3], [0.0, 1.0, 0.0], 1.0, 2.0).unwrap();
        // Inside at the center, but sqrt(3) from the surface, not 1.
        assert_eq!(spindle.eval([0.0; 3]), -1.0);
        assert_eq!(Formula::claims(&spindle).metric, Metric::Lipschitz(1.0));
        let ring = Torus::new([0.0; 3], [0.0, 1.0, 0.0], 2.0, 1.0).unwrap();
        assert_eq!(Formula::claims(&ring).metric, Metric::ExactDistance);
    }

    #[test]
    fn claims_bound_nearly_aligned_cylinders() {
        let cylinder = Cylinder::new([0.0; 3], [1.0, 1e-10, 0.0], 1.0, 1.0).unwrap();
        let Support::Bounded(bounds) = Formula::claims(&cylinder).support else {
            panic!("bounded");
        };
        let p = [1.0 + 5e-11, -0.9, 0.0];
        assert!(cylinder.eval(p) < 0.0);
        assert!(bounds.contains(p), "{bounds:?}");
    }

    #[test]
    fn claims_bound_the_interior() {
        let cylinder = Cylinder::new([1.0, 0.0, 0.0], [1.0, 1.0, 0.0], 0.5, 1.0).unwrap();
        let claims = Formula::claims(&cylinder);
        assert_eq!(claims.metric, Metric::ExactDistance);
        let Support::Bounded(bounds) = claims.support else {
            panic!("bounded");
        };
        let rim = [
            1.0 + 0.5_f64.sqrt() + 0.5 * 0.5_f64.sqrt(),
            0.5_f64.sqrt() - 0.5 * 0.5_f64.sqrt(),
            0.0,
        ];
        assert!(bounds.contains(rim));
        assert!(close(cylinder.eval(rim), 0.0));
    }

    #[test]
    fn intervals_contain_samples() {
        let cone = RoundCone::new([0.0; 3], [1.0, 0.5, 0.0], 0.4, 0.1).unwrap();
        let cuboid = Cuboid::new([0.2, 0.0, 0.0], [0.3, 0.4, 0.5]).unwrap();
        let regions = [
            Aabb::new([-0.5, -0.5, -0.5], [0.5, 0.5, 0.5]).unwrap(),
            Aabb::new([0.9, 0.4, -0.1], [1.2, 0.6, 0.1]).unwrap(),
            Aabb::new([0.1, -0.1, 0.4], [0.3, 0.1, 0.6]).unwrap(),
        ];
        for region in regions {
            for field in [&cone as &dyn ScalarField, &cuboid] {
                let bounds: Interval = field.eval_interval(&region);
                let slopes = field.eval_gradient_interval(&region);
                for corner in region.corners().into_iter().chain([region.center()]) {
                    let mut row = [Dual::default()];
                    field.eval_gradients(&[corner], &mut row);
                    assert!(bounds.contains(row[0].value), "{region:?} {corner:?}");
                    for axis in 0..3 {
                        assert!(slopes.gradient[axis].contains(row[0].gradient[axis]));
                    }
                }
            }
        }
    }

    #[test]
    fn parameters_are_checked() {
        assert!(Sphere::new([f64::NAN, 0.0, 0.0], 1.0).is_err());
        assert!(Sphere::new([0.0; 3], 0.0).is_err());
        assert!(Cylinder::new([0.0; 3], [0.0; 3], 1.0, 1.0).is_err());
        assert!(RoundCone::new([0.0; 3], [1.0; 3], -1.0, 1.0).is_err());
    }
}
