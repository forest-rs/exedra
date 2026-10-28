// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Spherical supporting geometry, including its two coordinate poles.

use super::{
    GeometryError, SurfaceEvaluation, check_radius, finite, frame_coordinates, rigid, vector,
};
use exedra_math::Placement3;

/// A sphere about a checked frame's origin, with longitude U and latitude V.
///
/// Longitude is periodic in radians; latitude is in `[-PI/2, PI/2]`. The
/// geometric normal is unique at either pole although the U derivative vanishes.
/// This supporting geometry does not establish a trimmed or closed solid.
#[derive(Clone, Debug, PartialEq)]
pub struct Sphere {
    frame: Placement3,
    radius: f64,
}

impl Sphere {
    /// Creates spherical supporting geometry with a positive finite radius.
    ///
    /// # Errors
    /// Returns a checked frame violation or [`GeometryError::Radius`].
    pub fn new(frame: Placement3, radius: f64) -> Result<Self, GeometryError> {
        rigid(frame)?;
        check_radius(radius)?;
        Ok(Self { frame, radius })
    }

    /// Supporting frame; its origin is the sphere center.
    #[must_use]
    pub const fn frame(&self) -> Placement3 {
        self.frame
    }

    /// Positive finite radius in model units.
    #[must_use]
    pub const fn radius(&self) -> f64 {
        self.radius
    }

    /// Supporting parameters of the radial projection of `point`.
    /// Longitude is principal in `[-PI, PI]`. Returns `None` on the polar axis,
    /// at the center, or when finite arithmetic cannot recover the parameters.
    /// Uses the actual accepted frame basis rather than assuming its transpose
    /// is an exact inverse.
    #[must_use]
    pub fn parameters(&self, point: [f64; 3]) -> Option<[f64; 2]> {
        let p = frame_coordinates(self.frame, point)?;
        let rho = libm::hypot(p[0], p[1]);
        if rho == 0.0 || !rho.is_finite() {
            return None;
        }
        Some([libm::atan2(p[1], p[0]), libm::atan2(p[2], rho)])
    }

    pub(super) fn evaluation(&self, uv: [f64; 2]) -> Option<SurfaceEvaluation> {
        let [u, v] = uv;
        let half_pi = core::f64::consts::FRAC_PI_2;
        if !u.is_finite() || !(-half_pi..=half_pi).contains(&v) {
            return None;
        }
        let (su, cu) = (libm::sin(u), libm::cos(u));
        // These canonical endpoints describe the limiting surface point and
        // derivatives, avoiding a fictitious finite ring from cos(PI/2).
        let (sv, cv) = if v == -half_pi {
            (-1.0, 0.0)
        } else if v == half_pi {
            (1.0, 0.0)
        } else {
            (libm::sin(v), libm::cos(v))
        };
        let r = self.radius;
        let normal = [cv * cu, cv * su, sv];
        let result = SurfaceEvaluation {
            point: self.frame.transform_point(normal.map(|x| r * x)),
            du: vector(self.frame, [-r * cv * su, r * cv * cu, 0.0]),
            dv: vector(self.frame, [-r * sv * cu, -r * sv * su, r * cv]),
            duu: vector(self.frame, [-r * cv * cu, -r * cv * su, 0.0]),
            duv: vector(self.frame, [r * sv * su, -r * sv * cu, 0.0]),
            dvv: vector(self.frame, normal.map(|x| -r * x)),
            normal: transformed_normal(self.frame, normal)?,
        };
        [
            result.point,
            result.du,
            result.dv,
            result.duu,
            result.duv,
            result.dvv,
            result.normal,
        ]
        .into_iter()
        .all(finite)
        .then_some(result)
    }
}

// A checked right-handed frame can still differ from exact orthogonality.
// Cofactors give the actual inverse-transpose direction; its positive
// determinant cancels when normalizing. This also works at a coordinate pole.
fn transformed_normal(frame: Placement3, normal: [f64; 3]) -> Option<[f64; 3]> {
    let axis = |i| frame.rows.map(|r| r[i]);
    let dual = [
        exedra_math::cross(axis(1), axis(2)),
        exedra_math::cross(axis(2), axis(0)),
        exedra_math::cross(axis(0), axis(1)),
    ];
    let result = core::array::from_fn::<_, 3, _>(|k| {
        dual[0][k] * normal[0] + dual[1][k] * normal[1] + dual[2][k] * normal[2]
    });
    let length = libm::hypot(libm::hypot(result[0], result[1]), result[2]);
    if length == 0.0 || !length.is_finite() {
        return None;
    }
    Some(result.map(|x| x / length))
}

/// Local outward arithmetic for this support comparison only. All defining
/// frame entries are authoritative f64 values; interval width covers operations,
/// not an arbitrary allowance proportional to the world datum.
#[derive(Copy, Clone)]
struct Enclosure {
    lo: f64,
    hi: f64,
}
impl Enclosure {
    fn finite(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite()
    }
    fn checked(lo: f64, hi: f64) -> Self {
        if lo.is_finite() && hi.is_finite() {
            Self { lo, hi }
        } else {
            Self {
                lo: f64::NEG_INFINITY,
                hi: f64::INFINITY,
            }
        }
    }
    fn exact(x: f64) -> Self {
        Self { lo: x, hi: x }
    }
    fn rounded_product(x: f64) -> Self {
        Self {
            lo: x.next_down(),
            hi: x.next_up(),
        }
    }
    fn add(self, b: Self) -> Self {
        if !self.finite() || !b.finite() {
            return Self::checked(f64::NEG_INFINITY, f64::INFINITY);
        }
        Self::checked((self.lo + b.lo).next_down(), (self.hi + b.hi).next_up())
    }
    fn sub(self, b: Self) -> Self {
        if !self.finite() || !b.finite() {
            return Self::checked(f64::NEG_INFINITY, f64::INFINITY);
        }
        Self::checked((self.lo - b.hi).next_down(), (self.hi - b.lo).next_up())
    }
    fn mul(self, b: Self) -> Self {
        if !self.finite() || !b.finite() {
            return Self::checked(f64::NEG_INFINITY, f64::INFINITY);
        }
        let v = [
            self.lo * b.lo,
            self.lo * b.hi,
            self.hi * b.lo,
            self.hi * b.hi,
        ];
        if !v.into_iter().all(f64::is_finite) {
            return Self::checked(f64::NEG_INFINITY, f64::INFINITY);
        }
        Self::checked(
            v.into_iter().fold(f64::INFINITY, f64::min).next_down(),
            v.into_iter().fold(f64::NEG_INFINITY, f64::max).next_up(),
        )
    }
    fn div(self, b: Self) -> Option<Self> {
        if !self.finite() || !b.finite() || (b.lo <= 0.0 && b.hi >= 0.0) {
            return None;
        }
        let v = [
            self.lo / b.lo,
            self.lo / b.hi,
            self.hi / b.lo,
            self.hi / b.hi,
        ];
        let result = Self {
            lo: v.into_iter().fold(f64::INFINITY, f64::min).next_down(),
            hi: v.into_iter().fold(f64::NEG_INFINITY, f64::max).next_up(),
        };
        (result.lo.is_finite() && result.hi.is_finite()).then_some(result)
    }
    fn magnitude(self) -> f64 {
        if self.finite() {
            self.lo.abs().max(self.hi.abs())
        } else {
            f64::INFINITY
        }
    }
}
fn enclosure_dot(a: [Enclosure; 3], b: [Enclosure; 3]) -> Enclosure {
    (0..3).fold(Enclosure::exact(0.0), |s, i| s.add(a[i].mul(b[i])))
}
fn enclosure_cross(a: [Enclosure; 3], b: [Enclosure; 3]) -> [Enclosure; 3] {
    core::array::from_fn(|i| {
        a[(i + 1) % 3]
            .mul(b[(i + 2) % 3])
            .sub(a[(i + 2) % 3].mul(b[(i + 1) % 3]))
    })
}

/// Encloses the complete affine pole position against an authoritative vertex.
pub(crate) fn pole_gap(sphere: &Sphere, latitude: f64, point: [f64; 3]) -> Option<f64> {
    if latitude.abs() != core::f64::consts::FRAC_PI_2 {
        return None;
    }
    let z = Enclosure::exact(sphere.radius() * latitude.signum());
    let frame = sphere.frame();
    let mut bound = Enclosure::exact(0.0);
    for (k, p) in point.into_iter().enumerate() {
        let ideal =
            Enclosure::exact(frame.rows[k][3]).add(Enclosure::exact(frame.rows[k][2]).mul(z));
        let residual = Enclosure::exact(p).sub(ideal);
        bound = bound.add(Enclosure::exact(residual.magnitude()));
    }
    Some(bound.hi)
}

fn axial_point(frame: Placement3, z: Enclosure) -> [Enclosure; 3] {
    core::array::from_fn(|k| {
        Enclosure::exact(frame.rows[k][3]).add(Enclosure::exact(frame.rows[k][2]).mul(z))
    })
}
fn point_gap(a: [Enclosure; 3], b: [Enclosure; 3]) -> f64 {
    (0..3)
        .fold(Enclosure::exact(0.0), |sum, k| {
            sum.add(Enclosure::exact(a[k].sub(b[k]).magnitude()))
        })
        .hi
}
pub(crate) fn cone_pole_bounds(cone: &super::Cone) -> Option<[[f64; 3]; 2]> {
    cone.apex_parameter()?;
    let z = Enclosure::exact(-cone.radius()).div(Enclosure::exact(cone.slope()))?;
    let points = axial_point(cone.frame(), z);
    points
        .into_iter()
        .all(Enclosure::finite)
        .then_some([points.map(|v| v.lo), points.map(|v| v.hi)])
}

/// The exact cone apex quotient is enclosed independently of its canonical
/// rounded parameter; no normal is assigned to this singular point.
pub(crate) fn cone_pole_gap(cone: &super::Cone, v: f64, point: [f64; 3]) -> Option<f64> {
    if Some(v) != cone.apex_parameter() {
        return None;
    }
    let z = Enclosure::exact(-cone.radius()).div(Enclosure::exact(cone.slope()))?;
    Some(point_gap(
        point.map(Enclosure::exact),
        axial_point(cone.frame(), z),
    ))
}

/// Bounds a complete line against one closed conical generator. Only the
/// explicitly identified pole endpoint may use the apex limit. Both projected
/// endpoints lie on one closed ray, so convexity bounds the whole line.
pub(crate) fn cone_generator_gap(
    cone: &super::Cone,
    line: &super::Line,
    range: super::ParameterRange,
    poles: [bool; 2],
) -> f64 {
    let compute = || -> Option<f64> {
        let endpoint = |t: f64| {
            core::array::from_fn::<_, 3, _>(|k| {
                Enclosure::exact(line.origin()[k])
                    .add(Enclosure::exact(line.direction()[k]).mul(Enclosure::exact(t)))
            })
        };
        let bounds = range.bounds();
        let source = bounds.map(endpoint);
        let reference = usize::from(poles[0]);
        let nominal = line_origin_at(line, bounds[reference])?;
        let local = frame_coordinates(cone.frame(), nominal)?;
        let scale = local[0].abs().max(local[1].abs());
        if scale == 0.0 || !scale.is_finite() {
            return None;
        }
        let sign = if local[0] < 0.0 { -1.0 } else { 1.0 };
        let x = sign * local[0] / scale;
        let y = sign * local[1] / scale;
        // Any finite Cayley parameter defines an exact unit direction by the
        // rational circle identity. Hypot only chooses a nearby generator;
        // its rounding cannot invalidate that identity or the residual bound.
        let t = y / (libm::hypot(x, y) + x);
        if !t.is_finite() {
            return None;
        }
        let t = Enclosure::exact(t);
        let t2 = t.mul(t);
        let denominator = Enclosure::exact(1.0).add(t2);
        let c = Enclosure::exact(1.0)
            .sub(t2)
            .div(denominator)?
            .mul(Enclosure::exact(sign));
        let s = t
            .mul(Enclosure::exact(2.0))
            .div(denominator)?
            .mul(Enclosure::exact(sign));
        let mut gap = 0.0_f64;
        for i in 0..2 {
            let ideal = if poles[i] {
                let z = Enclosure::exact(-cone.radius()).div(Enclosure::exact(cone.slope()))?;
                axial_point(cone.frame(), z)
            } else {
                let p = line_origin_at(line, bounds[i])?;
                let z = Enclosure::exact(frame_coordinates(cone.frame(), p)?[2]);
                let radial =
                    Enclosure::exact(cone.radius()).add(Enclosure::exact(cone.slope()).mul(z));
                if radial.lo <= 0.0 {
                    return None;
                }
                let local = [radial.mul(c), radial.mul(s), z];
                let frame = cone.frame();
                core::array::from_fn(|k| {
                    (0..3).fold(Enclosure::exact(frame.rows[k][3]), |sum, j| {
                        sum.add(Enclosure::exact(frame.rows[k][j]).mul(local[j]))
                    })
                })
            };
            gap = gap.max(point_gap(source[i], ideal));
        }
        gap.is_finite().then_some(gap)
    };
    compute().unwrap_or(f64::INFINITY)
}
fn line_origin_at(line: &super::Line, t: f64) -> Option<[f64; 3]> {
    let p = core::array::from_fn(|k| line.origin()[k] + line.direction()[k] * t);
    finite(p).then_some(p)
}

/// Whole harmonic curve comparison with the actual affine image of the sphere.
/// The bound uses all coefficients, not samples. Supplied A/B coefficients each
/// came from one rounded frame-axis/radius product; their neighboring floats
/// enclose that defining product before the actual-basis inverse.
pub(crate) fn harmonic_gap(sphere: &Sphere, center: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let calculate = || -> Option<f64> {
        let frame = sphere.frame();
        let axis = |i| frame.rows.map(|r| Enclosure::exact(r[i]));
        let dual = [
            enclosure_cross(axis(1), axis(2)),
            enclosure_cross(axis(2), axis(0)),
            enclosure_cross(axis(0), axis(1)),
        ];
        let determinant = enclosure_dot(axis(0), dual[0]);
        let radius = Enclosure::exact(sphere.radius());
        let coordinates = |p: [Enclosure; 3]| -> Option<[Enclosure; 3]> {
            Some([
                enclosure_dot(p, dual[0]).div(determinant)?.div(radius)?,
                enclosure_dot(p, dual[1]).div(determinant)?.div(radius)?,
                enclosure_dot(p, dual[2]).div(determinant)?.div(radius)?,
            ])
        };
        let delta = core::array::from_fn(|k| {
            Enclosure::exact(center[k]).sub(Enclosure::exact(frame.rows[k][3]))
        });
        let c = coordinates(delta)?;
        let a = coordinates(a.map(Enclosure::rounded_product))?;
        let b = coordinates(b.map(Enclosure::rounded_product))?;
        let aa = enclosure_dot(a, a);
        let bb = enclosure_dot(b, b);
        let half = Enclosure::exact(0.5);
        // |norm(P/R)^2 - 1| bounds |norm(P/R)-1| since norm(P/R)+1 >= 1.
        let coefficients = [
            enclosure_dot(c, c)
                .add(aa.mul(half))
                .add(bb.mul(half))
                .sub(Enclosure::exact(1.0)),
            enclosure_dot(c, a).mul(Enclosure::exact(2.0)),
            enclosure_dot(c, b).mul(Enclosure::exact(2.0)),
            aa.sub(bb).mul(half),
            enclosure_dot(a, b),
        ];
        let residual = coefficients
            .into_iter()
            .fold(Enclosure::exact(0.0), |sum, c| {
                sum.add(Enclosure::exact(c.magnitude()))
            });
        // Sum of absolute matrix entries bounds the forward operator norm.
        let operator = frame
            .rows
            .into_iter()
            .flat_map(|r| [r[0], r[1], r[2]])
            .fold(Enclosure::exact(0.0), |sum, x| {
                sum.add(Enclosure::exact(x.abs()))
            });
        let bound = radius.mul(residual).mul(operator).hi;
        bound.is_finite().then_some(bound)
    };
    calculate().unwrap_or(f64::INFINITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3], gap: f64) {
        assert!(a.into_iter().zip(b).all(|(x, y)| (x - y).abs() <= gap));
    }

    #[test]
    fn poles_are_exact_points_with_unique_normals_and_singular_u_derivatives() {
        let sphere = Sphere::new(Placement3::IDENTITY, 3.0).unwrap();
        for sign in [-1.0, 1.0] {
            for u in [-2.0, 0.0, 2.0, 1000.0] {
                let e = sphere
                    .evaluation([u, sign * core::f64::consts::FRAC_PI_2])
                    .unwrap();
                assert_eq!(e.point, [0.0, 0.0, sign * 3.0]);
                assert_eq!(e.du, [0.0; 3]);
                assert_eq!(e.normal, [0.0, 0.0, sign]);
                assert_eq!(sphere.parameters(e.point), None);
            }
        }
    }

    #[test]
    fn derivatives_and_actual_basis_parameter_roundtrips_agree() {
        let mut frame = Placement3::rotate_y_then_translate(0.7, 2.0, -3.0, 4.0);
        frame.rows[0][0] *= 1.0 + 2e-13;
        let sphere = Sphere::new(frame, 2.5).unwrap();
        for uv in [[-2.0, -0.8], [0.3, 0.4], [2.8, 1.2]] {
            let e = sphere.evaluation(uv).unwrap();
            let inverse = sphere.parameters(e.point).unwrap();
            close(sphere.evaluation(inverse).unwrap().point, e.point, 1e-14);
            let h = 1e-5;
            for axis in 0..2 {
                let mut a = uv;
                let mut b = uv;
                a[axis] -= h;
                b[axis] += h;
                let a = sphere.evaluation(a).unwrap();
                let b = sphere.evaluation(b).unwrap();
                close(
                    core::array::from_fn(|k| (b.point[k] - a.point[k]) / (2.0 * h)),
                    if axis == 0 { e.du } else { e.dv },
                    1e-9,
                );
                close(
                    core::array::from_fn(|k| (b.du[k] - a.du[k]) / (2.0 * h)),
                    if axis == 0 { e.duu } else { e.duv },
                    1e-9,
                );
            }
        }
    }

    #[test]
    fn harmonic_support_bound_is_whole_curve_and_uses_local_deltas() {
        let sphere = Sphere::new(Placement3::translate(1e16, 0.0, 0.0), 2.0).unwrap();
        let good = harmonic_gap(&sphere, [1e16, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]);
        assert!(
            good > 0.0 && good < 1e-11,
            "large datum must not replace local arithmetic: {good}"
        );
        let bad = harmonic_gap(&sphere, [1e16, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!(
            bad >= 1.0,
            "off-sphere ellipse must be refused by a tight policy: {bad}"
        );
        assert_eq!(
            harmonic_gap(&sphere, [1e16, 0.0, 0.0], [f64::MAX; 3], [0.0; 3]),
            f64::INFINITY
        );
    }

    #[test]
    fn accepted_near_rigid_frames_have_actual_perpendicular_outward_normals() {
        let mut frame = Placement3::IDENTITY;
        frame.rows[0][1] = 5e-13;
        frame.rows[1][1] = 1.0 + 2e-13;
        let sphere = Sphere::new(frame, 3.0).unwrap();
        for v in [
            -core::f64::consts::FRAC_PI_2,
            -0.4,
            0.8,
            core::f64::consts::FRAC_PI_2,
        ] {
            let e = sphere.evaluation([0.7, v]).unwrap();
            let dot = exedra_math::dot;
            assert!(dot(e.normal, e.du).abs() < 2e-15);
            assert!(dot(e.normal, e.dv).abs() < 2e-15);
            assert!(
                (libm::hypot(libm::hypot(e.normal[0], e.normal[1]), e.normal[2]) - 1.0).abs()
                    < 4e-16
            );
            assert!(dot(e.normal, e.point) > 0.0);
            if e.du != [0.0; 3] {
                assert!(dot(e.normal, exedra_math::cross(e.du, e.dv)) > 0.0);
            }
        }
    }

    #[test]
    fn invalid_latitude_and_nonfinite_or_overflowed_results_are_refused() {
        let sphere = Sphere::new(Placement3::IDENTITY, 1.0).unwrap();
        assert!(
            sphere
                .evaluation([0.0, core::f64::consts::FRAC_PI_2.next_up()])
                .is_none()
        );
        assert!(sphere.evaluation([f64::INFINITY, 0.0]).is_none());
        assert!(sphere.evaluation([0.0, f64::NAN]).is_none());
        assert!(Sphere::new(Placement3::IDENTITY, 0.0).is_err());
        let huge = Sphere::new(Placement3::translate(f64::MAX, 0.0, 0.0), f64::MAX).unwrap();
        assert!(huge.evaluation([0.0, 0.0]).is_none());
    }
}
