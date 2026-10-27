// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bounded analytic intersections with explicit parameter correspondence.
//! These operations construct geometry; they do not edit a body or certify a solid.

use super::{Curve, Cylinder, Ellipse, GeometryError, Plane, Surface};
use exedra_math::{Placement3, add, cross, dot, scale, sub};

/// Numeric bounds for a complete nonparallel plane/cylinder section.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SectionPolicy {
    /// Minimum absolute dot product of unit cylinder axis and plane normal.
    /// Values closer to parallel are refused before constructing large semiaxes.
    pub min_axis_dot: f64,
    /// Maximum whole-curve agreement bound with either supporting surface.
    pub max_gap: f64,
}

/// A finite axial function in cylinder coordinates, independent of texture UVs.
/// `v(u) = offset + cosine*cos(u) + sine*sin(u)`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AxialHarmonic {
    offset: f64,
    cosine: f64,
    sine: f64,
}
impl AxialHarmonic {
    /// Checks finite coefficients and finite full-period bounds.
    /// # Errors
    /// Returns [`GeometryError::NonFinite`] when coefficients/bounds overflow.
    pub fn new(offset: f64, cosine: f64, sine: f64) -> Result<Self, GeometryError> {
        let result = Self {
            offset,
            cosine,
            sine,
        };
        if ![offset, cosine, sine]
            .into_iter()
            .chain(result.bounds())
            .all(f64::is_finite)
        {
            return Err(GeometryError::NonFinite);
        }
        Ok(result)
    }
    /// Constant, cosine and sine coefficients in model units.
    #[must_use]
    pub const fn coefficients(self) -> [f64; 3] {
        [self.offset, self.cosine, self.sine]
    }
    /// Evaluates at a cylinder azimuth in radians, without periodic wrapping.
    #[must_use]
    pub fn at(self, u: f64) -> Option<f64> {
        if !u.is_finite() {
            return None;
        }
        let v = self.offset + self.cosine * libm::cos(u) + self.sine * libm::sin(u);
        v.is_finite().then_some(v)
    }
    /// Conservative full-period axial bounds; independent of a parameter trim.
    #[must_use]
    pub fn bounds(self) -> [f64; 2] {
        // L1 is a full-period enclosure without assuming a rounded hypot
        // overestimates the exact amplitude. Include multiply/add evaluation
        // roundoff and round both endpoints outward.
        let amplitude = (self.cosine.abs() + self.sine.abs()).next_up();
        let padding = 8.0 * f64::EPSILON * (self.offset.abs() + amplitude);
        [
            (self.offset - amplitude - padding).next_down(),
            (self.offset + amplitude + padding).next_up(),
        ]
    }
}

/// Exact-form ellipse section and its cylinder-parameter correspondence.
/// Floating-point support disagreement is bounded explicitly by [`Self::max_gap`].
#[derive(Clone, Debug, PartialEq)]
pub struct PlaneCylinderSection {
    curve: Curve,
    phase: f64,
    reversed: bool,
    axial: AxialHarmonic,
    max_gap: f64,
}
impl PlaneCylinderSection {
    /// Closed section geometry in the source coordinates, parameterized by ellipse angle.
    #[must_use]
    pub fn curve(&self) -> &Curve {
        &self.curve
    }
    /// Ellipse's checked frame and ordered semiaxes, including the circular limit.
    #[must_use]
    pub fn ellipse(&self) -> &Ellipse {
        let Curve::Ellipse(ellipse) = &self.curve else {
            unreachable!("section is an ellipse")
        };
        ellipse
    }
    /// Cylinder azimuth at ellipse parameter zero, in `[-PI, PI]`.
    #[must_use]
    pub const fn phase(&self) -> f64 {
        self.phase
    }
    /// Whether increasing ellipse angle decreases cylinder azimuth.
    #[must_use]
    pub const fn reversed(&self) -> bool {
        self.reversed
    }
    /// Cylinder height as an explicit harmonic in cylinder azimuth.
    #[must_use]
    pub const fn axial(&self) -> AxialHarmonic {
        self.axial
    }
    /// Conservative full-section agreement bound with both source supports.
    #[must_use]
    pub const fn max_gap(&self) -> f64 {
        self.max_gap
    }
    /// Maps an ellipse angle within one turn of zero to an unwrapped cylinder angle.
    /// Larger absolute angles are refused: adding a phase to a huge f64 would
    /// lose the phase and invalidate the reported parameter-map agreement.
    #[must_use]
    pub fn cylinder_angle(&self, ellipse_angle: f64) -> Option<f64> {
        if !ellipse_angle.is_finite() || ellipse_angle.abs() > core::f64::consts::TAU {
            return None;
        }
        let u = self.phase
            + if self.reversed {
                -ellipse_angle
            } else {
                ellipse_angle
            };
        u.is_finite().then_some(u)
    }
    /// Inverse of [`Self::cylinder_angle`] within the same one-turn window.
    /// No angular wrapping is performed; an unknown periodic branch is refused.
    #[must_use]
    pub fn ellipse_angle(&self, cylinder_angle: f64) -> Option<f64> {
        let t = cylinder_angle - self.phase;
        let t = if self.reversed { -t } else { t };
        let limit = core::f64::consts::TAU;
        (t.is_finite() && t.abs() <= limit * (1.0 + 8.0 * f64::EPSILON))
            .then(|| t.clamp(-limit, limit))
    }
}

/// A complete plane/cylinder section could not be represented within policy.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum SectionError {
    /// Dot threshold must be finite in `(0,1]`, gap finite and nonnegative.
    InvalidPolicy,
    /// Plane is parallel or too nearly parallel to the cylinder axis.
    NearParallel {
        /// Absolute dot of normalized axis and normal.
        axis_dot: f64,
        /// Requested lower bound.
        minimum: f64,
    },
    /// Derived frame or semiaxes violate the primitive geometry contract.
    Geometry(GeometryError),
    /// Derived coordinates/coefficients overflow finite arithmetic.
    NumericLimit,
    /// The complete section cannot meet the requested support agreement.
    SupportGap {
        /// Conservative whole-curve disagreement.
        bound: f64,
        /// Caller policy in model units.
        allowed: f64,
    },
}
impl core::fmt::Display for SectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "plane/cylinder section: {self:?}")
    }
}
impl core::error::Error for SectionError {}

/// Constructs the complete ellipse made by a plane crossing an infinite cylinder.
/// Neither input surface is modified. Parallel/grazing generatrices are outside
/// this bounded operation; body trimming and material containment belong to callers.
/// # Errors
/// Returns policy, near-parallel, finite-arithmetic or whole-support-gap refusals.
pub fn plane_cylinder(
    plane: &Plane,
    cylinder: &Cylinder,
    policy: &SectionPolicy,
) -> Result<PlaneCylinderSection, SectionError> {
    if !policy.min_axis_dot.is_finite()
        || policy.min_axis_dot <= 0.0
        || policy.min_axis_dot > 1.0
        || !policy.max_gap.is_finite()
        || policy.max_gap < 0.0
    {
        return Err(SectionError::InvalidPolicy);
    }
    let pf = plane.frame();
    let cf = cylinder.frame();
    let norm = |v: [f64; 3]| libm::hypot(libm::hypot(v[0], v[1]), v[2]);
    let unit = |v: [f64; 3]| scale(v, 1.0 / norm(v));
    let normal = unit(cross(pf.rows.map(|r| r[0]), pf.rows.map(|r| r[1])));
    let axis = cf.rows.map(|r| r[2]);
    let denominator = dot(normal, axis);
    let cosine = (denominator / norm(axis)).abs().min(1.0);
    if cosine < policy.min_axis_dot {
        return Err(SectionError::NearParallel {
            axis_dot: cosine,
            minimum: policy.min_axis_dot,
        });
    }
    // The native V coordinate multiplies the actual accepted Z basis. Do not
    // normalize that defining basis or reinterpret physical height as native V.
    let origin = cf.rows.map(|r| r[3]);
    let offset = dot(normal, sub(pf.rows.map(|r| r[3]), origin)) / denominator;
    let vc = -cylinder.radius() * dot(normal, cf.rows.map(|r| r[0])) / denominator;
    let vs = -cylinder.radius() * dot(normal, cf.rows.map(|r| r[1])) / denominator;
    let axial = AxialHarmonic::new(offset, vc, vs).map_err(|_| SectionError::NumericLimit)?;
    let center = add(origin, scale(axis, offset));
    let a = add(
        scale(cf.rows.map(|r| r[0]), cylinder.radius()),
        scale(axis, vc),
    );
    let b = add(
        scale(cf.rows.map(|r| r[1]), cylinder.radius()),
        scale(axis, vs),
    );
    let magnitude = a.into_iter().chain(b).map(f64::abs).fold(0.0, f64::max);
    if magnitude == 0.0 || !magnitude.is_finite() || !center.into_iter().all(f64::is_finite) {
        return Err(SectionError::NumericLimit);
    }
    let normalized_a = scale(a, 1.0 / magnitude);
    let normalized_b = scale(b, 1.0 / magnitude);
    let aa = dot(normalized_a, normalized_a);
    let bb = dot(normalized_b, normalized_b);
    let ab = dot(normalized_a, normalized_b);
    let mut phase = 0.5 * libm::atan2(2.0 * ab, aa - bb);
    let (sin, cos) = (libm::sin(phase), libm::cos(phase));
    let mut major = add(scale(a, cos), scale(b, sin));
    let mut minor = add(scale(a, -sin), scale(b, cos));
    let mut major_radius = norm(major);
    let mut minor_radius = norm(minor);
    if minor_radius > major_radius {
        (major, minor) = (minor, scale(major, -1.0));
        core::mem::swap(&mut major_radius, &mut minor_radius);
        phase += core::f64::consts::FRAC_PI_2;
    }
    if ![major_radius, minor_radius].into_iter().all(f64::is_finite) || minor_radius <= 0.0 {
        return Err(SectionError::NumericLimit);
    }
    let reversed = dot(cross(unit(major), unit(minor)), normal) < 0.0;
    if reversed {
        minor = scale(minor, -1.0);
    }
    let curve = Curve::ellipse(
        Placement3::from_axes(unit(major), unit(minor), normal, center),
        major_radius,
        minor_radius,
    )
    .map_err(SectionError::Geometry)?;
    let Curve::Ellipse(ellipse) = &curve else {
        unreachable!()
    };
    // The complete parameter map is another harmonic coefficient comparison.
    let (sin, cos) = (libm::sin(phase), libm::cos(phase));
    let sign = if reversed { -1.0 } else { 1.0 };
    let expected_a = add(
        scale(cf.rows.map(|r| r[0]), cylinder.radius() * cos),
        add(
            scale(cf.rows.map(|r| r[1]), cylinder.radius() * sin),
            scale(axis, vc * cos + vs * sin),
        ),
    );
    let expected_b = scale(
        add(
            scale(cf.rows.map(|r| r[0]), -cylinder.radius() * sin),
            add(
                scale(cf.rows.map(|r| r[1]), cylinder.radius() * cos),
                scale(axis, -vc * sin + vs * cos),
            ),
        ),
        sign,
    );
    let actual_a = ellipse.frame().rows.map(|r| r[0] * major_radius);
    let actual_b = ellipse.frame().rows.map(|r| r[1] * minor_radius);
    let coordinate_scale = center
        .into_iter()
        .map(f64::abs)
        .fold(major_radius, f64::max);
    let parameter_gap = norm(sub(actual_a, expected_a))
        + norm(sub(actual_b, expected_b))
        + 256.0 * f64::EPSILON * coordinate_scale;
    let gap = super::ellipse_support_gap(ellipse, &Surface::Plane(plane.clone()))
        .max(super::ellipse_support_gap(
            ellipse,
            &Surface::Cylinder(cylinder.clone()),
        ))
        .max(parameter_gap);
    if !gap.is_finite() || gap > policy.max_gap {
        return Err(SectionError::SupportGap {
            bound: gap,
            allowed: policy.max_gap,
        });
    }
    Ok(PlaneCylinderSection {
        curve,
        phase,
        reversed,
        axial,
        max_gap: gap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::{PI, TAU};
    fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
        let d = sub(a, b);
        libm::hypot(libm::hypot(d[0], d[1]), d[2])
    }
    #[test]
    fn section_positions_inverse_phase_and_axial_trim_agree_across_scales_and_senses() {
        for size in [1e-80, 1.0, 1e80] {
            for tilt in [0.0, 1e-8, 0.5, 1.3] {
                for roll in [-PI, -0.7, 0.0, PI - 1e-9] {
                    for reverse in [false, true] {
                        let pf = if reverse {
                            Placement3::rotate_x_then_translate(PI, 0.0, 0.0, 7.0 * size)
                        } else {
                            Placement3::translate(0.0, 0.0, 7.0 * size)
                        };
                        let cf = Placement3::rotate_y_then_translate(
                            tilt,
                            -3.0 * size,
                            2.0 * size,
                            -5.0 * size,
                        ) * Placement3::rotate_z_then_translate(roll, 0.0, 0.0, 0.0);
                        let Surface::Plane(p) = Surface::plane(pf).unwrap() else {
                            unreachable!()
                        };
                        let Surface::Cylinder(c) = Surface::cylinder(cf, 2.0 * size).unwrap()
                        else {
                            unreachable!()
                        };
                        let section = plane_cylinder(
                            &p,
                            &c,
                            &SectionPolicy {
                                min_axis_dot: 0.1,
                                max_gap: 1e-10 * size,
                            },
                        )
                        .unwrap();
                        assert_eq!(section.reversed(), reverse);
                        assert!(section.cylinder_angle(1e100).is_none());
                        assert!(section.ellipse_angle(1e100).is_none());
                        for i in 0..33 {
                            let t = (f64::from(i) / 32.0) * TAU - PI;
                            let point = section.curve().evaluate(t).unwrap().point;
                            let u = section.cylinder_angle(t).unwrap();
                            let v = section.axial().at(u).unwrap();
                            let expected =
                                Surface::Cylinder(c.clone()).evaluate([u, v]).unwrap().point;
                            assert!(
                                distance(point, expected) <= 1e-11 * size,
                                "tilt={tilt} roll={roll} reverse={reverse} t={t}"
                            );
                            assert!((section.ellipse_angle(u).unwrap() - t).abs() < 2e-15);
                            let native = section.ellipse().parameter(point).unwrap();
                            assert!(
                                libm::sin(native - t).abs() < 1e-13,
                                "ellipse inverse at t={t}"
                            );
                            let uv = Surface::Cylinder(c.clone()).parameters(point).unwrap();
                            assert!(
                                libm::sin(uv[0] - u).abs() < 1e-13,
                                "cylinder phase at t={t}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn accepted_near_rigid_frames_preserve_native_parameter_correspondence() {
        let Surface::Plane(p) = Surface::plane(Placement3::translate(0.0, 0.0, 1e6)).unwrap()
        else {
            unreachable!()
        };
        let mut frame = Placement3::IDENTITY;
        frame.rows[2][2] = 1.0 + 2e-13;
        let surface = Surface::cylinder(frame, 1.0).unwrap();
        let Surface::Cylinder(c) = &surface else {
            unreachable!()
        };
        let policy = SectionPolicy {
            min_axis_dot: 0.1,
            max_gap: 1e-7,
        };
        let section = plane_cylinder(&p, c, &policy).unwrap();
        for t in [0.0, 0.7, 3.0] {
            let native = section.curve().evaluate(t).unwrap().point;
            let u = section.cylinder_angle(t).unwrap();
            let actual = surface
                .evaluate([u, section.axial().at(u).unwrap()])
                .unwrap()
                .point;
            assert!(
                distance(native, actual) <= section.max_gap(),
                "correspondence must fit reported gap, actual={} bound={}",
                distance(native, actual),
                section.max_gap()
            );
        }
    }

    #[test]
    fn accepted_sheared_near_rigid_inputs_keep_full_parameter_coefficients() {
        let mut pf = Placement3::translate(0.0, 0.0, 1e6);
        pf.rows[1][0] = 2e-13;
        let Surface::Plane(p) = Surface::plane(pf).unwrap() else {
            unreachable!()
        };
        let mut cf = Placement3::rotate_y_then_translate(0.5, 3.0, 2.0, 0.0);
        cf.rows[0][0] += 1e-13;
        cf.rows[1][0] = 1e-13;
        cf.rows[2][2] *= 1.0 + 1e-13;
        let surface = Surface::cylinder(cf, 1.0).unwrap();
        let Surface::Cylinder(c) = &surface else {
            unreachable!()
        };
        let section = plane_cylinder(
            &p,
            c,
            &SectionPolicy {
                min_axis_dot: 0.1,
                max_gap: 1e-6,
            },
        )
        .unwrap();
        for t in [-3.0, -0.4, 0.0, 1.0, 3.0, 6.0] {
            let a = section.curve().evaluate(t).unwrap().point;
            let u = section.cylinder_angle(t).unwrap();
            let b = surface
                .evaluate([u, section.axial().at(u).unwrap()])
                .unwrap()
                .point;
            assert!(distance(a, b) <= section.max_gap());
        }
    }

    #[test]
    fn near_rigid_inverse_support_bound_covers_scaled_radial_basis() {
        let factor = 1.0 + 2e-13;
        let mut frame = Placement3::IDENTITY;
        frame.rows[0][0] = factor;
        let surface = Surface::cylinder(frame, 1.0).unwrap();
        let Curve::Ellipse(ellipse) =
            Curve::ellipse(Placement3::IDENTITY, 1.0, 1.0 / factor).unwrap()
        else {
            unreachable!()
        };
        let bound = super::super::ellipse_support_gap(&ellipse, &surface);
        let actual = surface.evaluate([0.0, 0.0]).unwrap().point;
        let edge = Curve::Ellipse(ellipse.clone()).evaluate(0.0).unwrap().point;
        assert!(
            distance(edge, actual) <= bound,
            "actual={} bound={bound}",
            distance(edge, actual)
        );
        let Curve::Ellipse(ellipse) = Curve::ellipse(
            Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_2, 0.0, 0.0, 0.0),
            1.0,
            1.0 / factor,
        )
        .unwrap() else {
            unreachable!()
        };
        let curve = Curve::Ellipse(ellipse.clone());
        let edge = curve.evaluate(core::f64::consts::FRAC_PI_2).unwrap().point;
        let actual = surface.evaluate([PI, 0.0]).unwrap().point;
        let bound = super::super::ellipse_support_gap(&ellipse, &surface);
        assert!(
            distance(edge, actual) <= bound,
            "transpose-based pseudo-inverse must not hide real support gap"
        );
    }
    #[test]
    fn axial_bounds_round_outward_and_cover_off_origin_cancellation() {
        let harmonic = AxialHarmonic::new(1e6, 0.1, -0.2).unwrap();
        let [min, max] = harmonic.bounds();
        assert!(
            min < 1e6 - 0.3 && max > 1e6 + 0.3,
            "L1 bounds and arithmetic headroom round outward"
        );
        for offset in [0.0, 1e6, -1e6] {
            let harmonic = AxialHarmonic::new(offset, 1e6, -1e6).unwrap();
            let bounds = harmonic.bounds();
            for u in [-1e100, -PI, -1e-20, 0.0, 1e-20, PI, 1e100] {
                let value = harmonic.at(u).unwrap();
                assert!(bounds[0] <= value && value <= bounds[1]);
            }
        }
    }

    #[test]
    fn parallel_invalid_and_unrepresentable_sections_refuse_without_mutating_inputs() {
        let Surface::Plane(p) = Surface::plane(Placement3::IDENTITY).unwrap() else {
            unreachable!()
        };
        let Surface::Cylinder(c) = Surface::cylinder(
            Placement3::rotate_y_then_translate(PI / 2.0, 0.0, 0.0, 0.0),
            1.0,
        )
        .unwrap() else {
            unreachable!()
        };
        let original = c.clone();
        let policy = SectionPolicy {
            min_axis_dot: 1e-6,
            max_gap: 1e-10,
        };
        assert!(matches!(
            plane_cylinder(&p, &c, &policy),
            Err(SectionError::NearParallel { .. })
        ));
        assert_eq!(c, original);
        assert_eq!(
            plane_cylinder(
                &p,
                &c,
                &SectionPolicy {
                    min_axis_dot: 0.0,
                    ..policy
                }
            ),
            Err(SectionError::InvalidPolicy)
        );
        assert!(AxialHarmonic::new(f64::MAX, f64::MAX, 0.0).is_err());
        assert!(AxialHarmonic::new(0.0, f64::NAN, 0.0).is_err());
        let Surface::Cylinder(c) = Surface::cylinder(Placement3::IDENTITY, 2.0).unwrap() else {
            unreachable!()
        };
        assert!(matches!(
            plane_cylinder(
                &p,
                &c,
                &SectionPolicy {
                    max_gap: 0.0,
                    ..policy
                }
            ),
            Err(SectionError::SupportGap { .. })
        ));
    }
    #[test]
    fn ellipse_primitive_derivatives_and_support_refusals_are_independent_of_tessellation() {
        let curve = Curve::ellipse(Placement3::translate(1.0, 2.0, 3.0), 5.0, 2.0).unwrap();
        assert_eq!(curve.period(), Some(TAU));
        let e = curve.evaluate(0.0).unwrap();
        assert_eq!(e.point, [6.0, 2.0, 3.0]);
        assert_eq!(e.first, [0.0, 2.0, 0.0]);
        assert_eq!(e.second, [-5.0, 0.0, 0.0]);
        for axes in [
            [1.0, 2.0],
            [1.0, 0.0],
            [f64::INFINITY, 1.0],
            [f64::NAN, 1.0],
        ] {
            assert!(Curve::ellipse(Placement3::IDENTITY, axes[0], axes[1]).is_err());
        }
        let Curve::Ellipse(vertical) = Curve::ellipse(
            Placement3::rotate_x_then_translate(PI / 2.0, 0.0, 0.0, 0.0),
            1.0,
            1.0,
        )
        .unwrap() else {
            unreachable!()
        };
        let cylinder = Surface::cylinder(Placement3::IDENTITY, 1.0).unwrap();
        assert!(
            super::super::ellipse_support_gap(&vertical, &cylinder) >= 1.0,
            "an edge projecting to a diameter is not a cylinder-supported circle"
        );
    }
}
