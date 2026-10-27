// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Regular ruled patches between two bounded line/circle curves.

use super::{Curve, CurveEvaluation, GeometryError, ParameterRange};
use exedra_math::{add, cross, dot, lerp, norm, normalize, scale, sub};

/// A bounded source curve traversed over a normalized parameter in `[0,1]`.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveSpan {
    curve: Curve,
    range: ParameterRange,
    reversed: bool,
}
impl CurveSpan {
    /// Checks that both bounded endpoints and parameter-scaled derivatives are finite.
    ///
    /// # Errors
    /// Returns [`GeometryError::NonFinite`] for unrepresentable bounded geometry.
    pub fn new(curve: Curve, range: ParameterRange, reversed: bool) -> Result<Self, GeometryError> {
        let result = Self {
            curve,
            range,
            reversed,
        };
        if result.evaluate(0.0).is_none()
            || result.evaluate(1.0).is_none()
            || !result.speed().is_finite()
            || !result.second_bound().is_finite()
            || !result.magnitude().is_finite()
        {
            return Err(GeometryError::NonFinite);
        }
        Ok(result)
    }
    /// Original untrimmed source geometry.
    #[must_use]
    pub const fn curve(&self) -> &Curve {
        &self.curve
    }
    /// Increasing source-curve parameter bounds.
    #[must_use]
    pub const fn range(&self) -> ParameterRange {
        self.range
    }
    /// Whether normalized traversal opposes increasing source parameters.
    #[must_use]
    pub const fn reversed(&self) -> bool {
        self.reversed
    }
    pub(crate) fn transformed(
        &self,
        frame: exedra_math::Placement3,
    ) -> Result<Self, GeometryError> {
        let curve = match &self.curve {
            Curve::Line(l) => Curve::line(
                frame.transform_point(l.origin()),
                super::vector(frame, l.direction()),
            )?,
            Curve::Circle(c) => Curve::circle(frame * c.frame(), c.radius())?,
            Curve::Ellipse(e) => Curve::ellipse(frame * e.frame(), e.semi_major(), e.semi_minor())?,
        };
        Self::new(curve, self.range, self.reversed)
    }
    /// Evaluates the bounded curve, with derivatives in normalized parameter units.
    #[must_use]
    pub fn evaluate(&self, fraction: f64) -> Option<CurveEvaluation> {
        let t = if self.reversed {
            1.0 - fraction
        } else {
            fraction
        };
        let mut e = self.curve.evaluate(self.range.at(t)?)?;
        let step = self.range.span() * if self.reversed { -1.0 } else { 1.0 };
        e.first = scale(e.first, step);
        e.second = scale(scale(e.second, step), step);
        [e.point, e.first, e.second]
            .into_iter()
            .flatten()
            .all(f64::is_finite)
            .then_some(e)
    }
    fn speed(&self) -> f64 {
        match &self.curve {
            Curve::Line(l) => norm(l.direction()) * self.range.span(),
            curve => {
                curve.harmonic().expect("harmonic curve").1[0]
                    * self.range.span()
                    * (1.0 + 4.0 * super::FRAME_TOLERANCE)
            }
        }
    }
    fn second_bound(&self) -> f64 {
        match &self.curve {
            Curve::Line(_) => 0.0,
            curve => {
                curve.harmonic().expect("harmonic curve").1[0]
                    * self.range.span()
                    * self.range.span()
                    * (1.0 + 4.0 * super::FRAME_TOLERANCE)
            }
        }
    }
    pub(crate) fn magnitude(&self) -> f64 {
        match self.curve.harmonic() {
            Some((frame, radii)) => {
                frame
                    .rows
                    .iter()
                    .map(|r| r[3].abs())
                    .fold(radii[0], f64::max)
                    + radii[0] * (1.0 + 4.0 * super::FRAME_TOLERANCE)
            }
            None => [0.0, 1.0]
                .into_iter()
                .flat_map(|t| self.evaluate(t).expect("checked").point)
                .map(f64::abs)
                .fold(0.0, f64::max),
        }
    }
    /// Conservative distance between curves at matching normalized parameters.
    pub(crate) fn alignment(&self, other: &Self) -> f64 {
        match (&self.curve, &other.curve) {
            (Curve::Line(_), Curve::Line(_)) => [0.0, 1.0]
                .into_iter()
                .map(|t| {
                    norm(sub(
                        self.evaluate(t).expect("checked span").point,
                        other.evaluate(t).expect("checked span").point,
                    ))
                })
                .fold(0.0, f64::max),
            (a, b) if a.harmonic().is_some() && b.harmonic().is_some() => {
                let coefficients = |span: &Self, curve: &Curve| {
                    let (frame, [major, minor]) = curve.harmonic().expect("harmonic curve");
                    let phase = span.range.bounds()[usize::from(span.reversed)];
                    let (s, c) = (libm::sin(phase), libm::cos(phase));
                    let sign = if span.reversed { -1.0 } else { 1.0 };
                    [
                        frame.rows.map(|r| r[3]),
                        super::vector(frame, [major * c, minor * s, 0.0]),
                        super::vector(frame, [-sign * major * s, sign * minor * c, 0.0]),
                    ]
                };
                let ar = a.harmonic().expect("harmonic").1[0];
                let br = b.harmonic().expect("harmonic").1[0];
                coefficients(self, a)
                    .into_iter()
                    .zip(coefficients(other, b))
                    .map(|(a, b)| norm(sub(a, b)))
                    .sum::<f64>()
                    + br * (self.range.span() - other.range.span()).abs()
                    + 8.0
                        * f64::EPSILON
                        * (ar
                            * self
                                .range
                                .bounds()
                                .into_iter()
                                .map(f64::abs)
                                .fold(self.range.span(), f64::max)
                            + br * other
                                .range
                                .bounds()
                                .into_iter()
                                .map(f64::abs)
                                .fold(other.range.span(), f64::max))
            }
            _ => f64::INFINITY,
        }
    }
}

/// Exact linear interpolation between two bounded curves: `(1-v) A(u) + v B(u)`.
///
/// Both parameters are normalized. Construction certifies nonzero `du × dv`
/// throughout the unit square with conservative interval bounds. This is a
/// local regularity certificate, not an intersection or solid-validity test.
#[derive(Clone, Debug, PartialEq)]
pub struct Ruled {
    curves: [CurveSpan; 2],
    mixed_bound: f64,
}
impl Ruled {
    pub(super) fn new(a: CurveSpan, b: CurveSpan) -> Result<Self, GeometryError> {
        let mut result = Self {
            curves: [a, b],
            mixed_bound: 0.0,
        };
        result.mixed_bound = result.bound_mixed()?;
        let mut work = 0;
        result.certify([0.0, 1.0], 0, &mut work)?;
        Ok(result)
    }
    fn bound_mixed(&self) -> Result<f64, GeometryError> {
        let speed = self.curves[0].speed() + self.curves[1].speed();
        let second = self.curves[0].second_bound() + self.curves[1].second_bound();
        let mut sampled = 0.0_f64;
        // Each derivative difference is Lipschitz with this second-derivative
        // bound. Midpoint samples plus the interval radius bound the entire U
        // range; these samples alone are never treated as a certificate.
        for i in 0..8 {
            let u = (f64::from(i) + 0.5) / 8.0;
            let a = self.curves[0].evaluate(u).ok_or(GeometryError::NonFinite)?;
            let b = self.curves[1].evaluate(u).ok_or(GeometryError::NonFinite)?;
            sampled = sampled.max(norm(sub(b.first, a.first)) + second / 16.0);
        }
        let mut bound = speed.min(sampled);
        if self
            .curves
            .iter()
            .all(|c| matches!(c.curve, Curve::Circle(_)))
        {
            // Equal angular schedules share cosine/sine coefficients. This
            // removes fictitious mixed curvature on translated cylinders and
            // tightly bounds differing radii without per-cell curve work.
            let a = self.curves[0]
                .evaluate(0.0)
                .ok_or(GeometryError::NonFinite)?;
            let b = self.curves[1]
                .evaluate(0.0)
                .ok_or(GeometryError::NonFinite)?;
            let wa = self.curves[0].range.span();
            let wb = self.curves[1].range.span();
            let d = sub(b.first, a.first);
            let e = sub(scale(b.second, 1.0 / wb), scale(a.second, 1.0 / wa));
            let magnitude = norm(d).max(norm(e));
            let coefficient = if magnitude == 0.0 {
                0.0
            } else {
                let (d, e) = (scale(d, 1.0 / magnitude), scale(e, 1.0 / magnitude));
                let (dd, ee, de) = (dot(d, d), dot(e, e), dot(d, e));
                magnitude * libm::sqrt(0.5 * (dd + ee + libm::hypot(dd - ee, 2.0 * de)))
            };
            bound = bound.min(coefficient + 2.0 * self.curves[1].speed() * (wa - wb).abs());
        }
        let bound = bound + 512.0 * f64::EPSILON * speed;
        if !bound.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        Ok(bound)
    }
    /// The two source curves, at V zero and one respectively.
    #[must_use]
    pub const fn curves(&self) -> &[CurveSpan; 2] {
        &self.curves
    }
    pub(super) fn evaluation(&self, uv: [f64; 2]) -> Option<super::SurfaceEvaluation> {
        if !ParameterRange::UNIT.contains(uv[1]) {
            return None;
        }
        let [a, b] = [
            self.curves[0].evaluate(uv[0])?,
            self.curves[1].evaluate(uv[0])?,
        ];
        let du = lerp(a.first, b.first, uv[1]);
        let dv = sub(b.point, a.point);
        let result = super::SurfaceEvaluation {
            point: lerp(a.point, b.point, uv[1]),
            du,
            dv,
            duu: lerp(a.second, b.second, uv[1]),
            duv: sub(b.first, a.first),
            dvv: [0.0; 3],
            normal: normalize(cross(du, dv))?,
        };
        [
            result.point,
            result.du,
            result.dv,
            result.duu,
            result.duv,
            result.normal,
        ]
        .into_iter()
        .flatten()
        .all(f64::is_finite)
        .then_some(result)
    }
    fn certify(&self, interval: [f64; 2], depth: u32, work: &mut u32) -> Result<(), GeometryError> {
        *work += 1;
        let failure = || GeometryError::NonRegularRuled { interval };
        if *work > 8192 {
            return Err(failure());
        }
        let u = 0.5 * (interval[0] + interval[1]);
        let [a, b] = [
            self.curves[0].evaluate(u).ok_or_else(failure)?,
            self.curves[1].evaluate(u).ok_or_else(failure)?,
        ];
        let delta = sub(b.point, a.point);
        let ns = [cross(a.first, delta), cross(b.first, delta)];
        let reference = normalize(add(ns[0], ns[1])).ok_or_else(failure)?;
        let half = (interval[1] - interval[0]) * 0.5;
        let mixed = self.mixed_bound.min(
            norm(sub(b.first, a.first))
                + half * (self.curves[0].second_bound() + self.curves[1].second_bound()),
        );
        let distance = norm(delta) + half * mixed;
        let regular = self.curves.iter().zip(ns).all(|(c, n)| {
            let variation = half * (c.second_bound() * distance + c.speed() * mixed);
            let headroom = 512.0
                * f64::EPSILON
                * c.speed()
                * (distance + self.curves[0].magnitude() + self.curves[1].magnitude());
            let lower = dot(n, reference) - variation - headroom;
            lower.is_finite() && lower > 0.0
        });
        if regular {
            return Ok(());
        }
        if depth == 20 {
            return Err(failure());
        }
        self.certify([interval[0], u], depth + 1, work)?;
        self.certify([u, interval[1]], depth + 1, work)
    }
    pub(crate) fn boundary(&self, side: usize) -> Option<CurveSpan> {
        match side {
            0 => Some(self.curves[0].clone()),
            2 => Some(self.curves[1].clone()),
            1 | 3 => {
                let u = if side == 1 { 1.0 } else { 0.0 };
                let a = self.curves[0].evaluate(u)?.point;
                let b = self.curves[1].evaluate(u)?.point;
                CurveSpan::new(Curve::line(a, sub(b, a)).ok()?, ParameterRange::UNIT, false).ok()
            }
            _ => None,
        }
    }
    // Once source-curve chords spend at most half the target, prefer splitting
    // a physically long ruling rather than forcing all mixed-error reduction
    // into ever smaller profile edges. Correctness still uses triangle_bound.
    pub(crate) fn refinement_axis(&self, n: [u32; 2], target: f64) -> usize {
        let u_speed = self.curves[0].speed().max(self.curves[1].speed());
        let u_chord = self.curves[0]
            .second_bound()
            .max(self.curves[1].second_bound())
            / (8.0 * f64::from(n[0]) * f64::from(n[0]));
        let ruling = norm(sub(
            self.curves[1].evaluate(0.5).expect("checked").point,
            self.curves[0].evaluate(0.5).expect("checked").point,
        )) + 0.5 * (self.curves[0].speed() + self.curves[1].speed());
        usize::from(
            u_chord <= target * 0.5 && ruling / f64::from(n[1]) > 2.0 * u_speed / f64::from(n[0]),
        )
    }
    /// Surface-to-triangle error for a normalized rectangular cell.
    /// Includes f64 headroom; callers separately account for boundary alignment
    /// and emitted vertex precision.
    /// Invalid normalized step widths outside `(0,1]` return infinity.
    #[must_use]
    pub fn triangle_bound(&self, steps: [f64; 2]) -> f64 {
        if steps
            .into_iter()
            .any(|s| !s.is_finite() || s <= 0.0 || s > 1.0)
        {
            return f64::INFINITY;
        }
        let magnitude = self
            .curves
            .iter()
            .map(CurveSpan::magnitude)
            .fold(0.0, f64::max);
        let rounding = magnitude * 512.0 * f64::EPSILON;
        if self
            .curves
            .iter()
            .all(|c| matches!(c.curve, Curve::Line(_)))
        {
            let corners = [
                self.curves[0].evaluate(0.0).expect("checked").point,
                self.curves[0].evaluate(1.0).expect("checked").point,
                self.curves[1].evaluate(1.0).expect("checked").point,
                self.curves[1].evaluate(0.0).expect("checked").point,
            ];
            return exedra_math::surface::bilinear_triangle_bounds(corners).map_or(
                f64::INFINITY,
                |b| {
                    b.planar_deviation
                        .min(b.mixed_deviation * steps[0] * steps[1])
                        + rounding
                },
            );
        }
        let second = self.curves[0]
            .second_bound()
            .max(self.curves[1].second_bound());
        let mixed = self.mixed_bound;
        second * steps[0] * steps[0] / 8.0 + mixed * steps[0] * steps[1] / 4.0 + rounding
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Surface;
    use exedra_math::Placement3;

    fn transition() -> Surface {
        Surface::ruled(
            CurveSpan::new(
                Curve::circle(Placement3::IDENTITY, 2.0).unwrap(),
                ParameterRange::new(0.0, core::f64::consts::FRAC_PI_2).unwrap(),
                false,
            )
            .unwrap(),
            CurveSpan::new(
                Curve::line([3.0, 0.0, 5.0], [-3.0, 3.0, 0.0]).unwrap(),
                ParameterRange::UNIT,
                false,
            )
            .unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn curved_ruled_partials_match_independent_differences() {
        let surface = transition();
        let uv = [0.37, 0.62];
        let e = surface.evaluate(uv).unwrap();
        for axis in 0..2 {
            let mut lo = uv;
            lo[axis] -= 1e-5;
            let mut hi = uv;
            hi[axis] += 1e-5;
            let (a, b) = (surface.evaluate(lo).unwrap(), surface.evaluate(hi).unwrap());
            for (actual, expected) in [
                (
                    scale(sub(b.point, a.point), 0.5e5),
                    if axis == 0 { e.du } else { e.dv },
                ),
                (
                    scale(sub(b.du, a.du), 0.5e5),
                    if axis == 0 { e.duu } else { e.duv },
                ),
                (
                    scale(sub(b.dv, a.dv), 0.5e5),
                    if axis == 0 { e.duv } else { e.dvv },
                ),
            ] {
                assert!(
                    norm(sub(actual, expected)) < 1e-8,
                    "{actual:?} {expected:?}"
                );
            }
        }
        assert_eq!(surface.periods(), [None, None]);
        assert_eq!(surface.parameters(e.point), None);
        assert!(surface.evaluate([1.01, 0.0]).is_none());
        assert!(surface.evaluate([0.0, -0.01]).is_none());
    }
    #[test]
    fn reversed_spans_keep_endpoints_and_normalized_derivatives() {
        let span = CurveSpan::new(
            Curve::line([1.0, 2.0, 3.0], [2.0, 0.0, 0.0]).unwrap(),
            ParameterRange::new(2.0, 4.0).unwrap(),
            true,
        )
        .unwrap();
        assert_eq!(span.evaluate(0.0).unwrap().point, [9.0, 2.0, 3.0]);
        assert_eq!(span.evaluate(1.0).unwrap().point, [5.0, 2.0, 3.0]);
        assert_eq!(span.evaluate(0.5).unwrap().first, [-4.0, 0.0, 0.0]);
        assert!(span.evaluate(f64::NAN).is_none());
    }
    #[test]
    fn mixed_derivative_bounds_cover_unequal_arc_schedules_and_remove_translated_circle_error() {
        let a = CurveSpan::new(
            Curve::circle(Placement3::IDENTITY, 2.0).unwrap(),
            ParameterRange::new(0.3, 1.7).unwrap(),
            false,
        )
        .unwrap();
        let translated = a.transformed(Placement3::translate(0.0, 0.0, 5.0)).unwrap();
        let same = Ruled {
            curves: [a.clone(), translated],
            mixed_bound: 0.0,
        };
        assert!(same.bound_mixed().unwrap() < 1e-10);
        for reversed in [false, true] {
            let b = CurveSpan::new(
                Curve::circle(Placement3::rotate_y_then_translate(0.3, 1.0, 0.0, 5.0), 3.0)
                    .unwrap(),
                ParameterRange::new(0.7, 2.5).unwrap(),
                reversed,
            )
            .unwrap();
            let ruled = Ruled {
                curves: [a.clone(), b],
                mixed_bound: 0.0,
            };
            let bound = ruled.bound_mixed().unwrap();
            for i in 0..=1000 {
                let u = f64::from(i) / 1000.0;
                let d = sub(
                    ruled.curves[1].evaluate(u).unwrap().first,
                    ruled.curves[0].evaluate(u).unwrap().first,
                );
                assert!(norm(d) <= bound, "{u} {} {bound}", norm(d));
            }
        }
    }
    #[test]
    fn regularity_refuses_collapsed_and_interior_singular_walls() {
        let a = CurveSpan::new(
            Curve::line([0.0; 3], [1.0, 0.0, 0.0]).unwrap(),
            ParameterRange::UNIT,
            false,
        )
        .unwrap();
        for b in [
            a.clone(),
            CurveSpan::new(
                Curve::line([1.0, 0.0, 1.0], [-1.0, 0.0, 0.0]).unwrap(),
                ParameterRange::UNIT,
                false,
            )
            .unwrap(),
            CurveSpan::new(
                Curve::line([0.0; 3], [1.0, 0.0, 1.0]).unwrap(),
                ParameterRange::UNIT,
                false,
            )
            .unwrap(),
        ] {
            assert!(matches!(
                Surface::ruled(a.clone(), b),
                Err(GeometryError::NonRegularRuled { .. })
            ));
        }
    }
    #[test]
    fn curved_patch_triangle_bound_covers_independent_parameter_interpolation() {
        let surface = transition();
        let Surface::Ruled(ruled) = &surface else {
            unreachable!()
        };
        for (nu, nv) in [(1, 1), (8, 4), (16, 8)] {
            let bound = ruled.triangle_bound([1.0 / f64::from(nu), 1.0 / f64::from(nv)]);
            for i in 0..nu {
                for j in 0..nv {
                    let corner = |x, y| {
                        surface
                            .evaluate([f64::from(x) / f64::from(nu), f64::from(y) / f64::from(nv)])
                            .unwrap()
                            .point
                    };
                    let p = [
                        corner(i, j),
                        corner(i + 1, j),
                        corner(i + 1, j + 1),
                        corner(i, j + 1),
                    ];
                    for (u, v) in [(0.25, 0.75), (0.75, 0.25), (0.5, 0.5)] {
                        let exact = surface
                            .evaluate([
                                (f64::from(i) + u) / f64::from(nu),
                                (f64::from(j) + v) / f64::from(nv),
                            ])
                            .unwrap()
                            .point;
                        let triangle = if v <= u {
                            add(
                                scale(p[0], 1.0 - u),
                                add(scale(p[1], u - v), scale(p[2], v)),
                            )
                        } else {
                            add(
                                scale(p[0], 1.0 - v),
                                add(scale(p[2], u), scale(p[3], v - u)),
                            )
                        };
                        assert!(norm(sub(exact, triangle)) <= bound, "{nu} {nv} {bound}");
                    }
                }
            }
        }
    }
}
