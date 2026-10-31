// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Regular ruled patches between two bounded analytic curves.

use super::{Curve, CurveEvaluation, GeometryError, NurbsCurve, ParameterRange};
use exedra_math::{add, cross, dot, lerp, norm, normalize, scale, sub};

/// Immutable preparation for one bounded operation. Keeping this on the
/// construction stack avoids enlarging every retained curve/surface variant.
#[derive(Clone, Copy)]
struct CurveBounds {
    speed: f64,
    second: f64,
    magnitude: f64,
}

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
        if result.evaluate(0.0).is_none() || result.evaluate(1.0).is_none() {
            return Err(GeometryError::NonFinite);
        }
        let bounds = result.bounds();
        if !bounds.speed.is_finite() || !bounds.second.is_finite() || !bounds.magnitude.is_finite()
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
        let curve = self.curve.transformed(frame)?;
        Self::new(curve, self.range, self.reversed)
    }
    /// The span as a NURBS curve over `[0, 1]`, traced in its normalized
    /// direction: exactly the same points. Lines and NURBS spans keep their
    /// normalized parameter too; a conic becomes rational quadratic arcs,
    /// whose parameter is not proportional to angle. `None` where the
    /// conversion overflows.
    #[must_use]
    pub fn nurbs(&self) -> Option<NurbsCurve> {
        let [lo, hi] = self.range.bounds();
        let conic = |center: [f64; 3], x: [f64; 3], y: [f64; 3]| {
            NurbsCurve::arc(center, x, y, [lo, hi])?.with_domain([0.0, 1.0])
        };
        let curve = match &self.curve {
            Curve::Line(_) => NurbsCurve::line(
                self.curve.evaluate(lo)?.point,
                self.curve.evaluate(hi)?.point,
            )?,
            Curve::Nurbs(n) => n.piece(self.range)?.with_domain([0.0, 1.0])?,
            Curve::Circle(c) => {
                let f = c.frame();
                let axis = |i: usize| scale(f.rows.map(|r| r[i]), c.radius());
                conic(f.rows.map(|r| r[3]), axis(0), axis(1))?
            }
            Curve::Ellipse(e) => {
                let f = e.frame();
                let axis = |i: usize, r: f64| scale(f.rows.map(|row| row[i]), r);
                conic(
                    f.rows.map(|r| r[3]),
                    axis(0, e.semi_major()),
                    axis(1, e.semi_minor()),
                )?
            }
        };
        Some(if self.reversed {
            curve.reversed()
        } else {
            curve
        })
    }

    /// Evaluates the bounded curve, with derivatives in normalized parameter units.
    /// At an endpoint knot, derivatives come from inside this bounded span.
    #[must_use]
    pub fn evaluate(&self, fraction: f64) -> Option<CurveEvaluation> {
        let t = if self.reversed {
            1.0 - fraction
        } else {
            fraction
        };
        let parameter = self.range.at(t)?;
        let mut e = if let Curve::Nurbs(n) = &self.curve
            && parameter == self.range.bounds()[1]
            && parameter < n.curve().domain()[1]
            && n.curve()
                .knots()
                .get(n.curve().knots().partition_point(|&k| k < parameter))
                == Some(&parameter)
        {
            // The untrimmed evaluator is right-continuous at a knot. At this
            // bounded piece's upper endpoint, its left-hand span instead owns
            // the tangent/curvature used by our interval bounds and surface.
            // Only this exact interior-knot endpoint needs prepared evaluation.
            let prepared = n.curve().view().prepare();
            let left = prepared.span_at(parameter).checked_sub(1)?;
            prepared.evaluation_in(left, parameter)?
        } else {
            self.curve.evaluate(parameter)?
        };
        let step = self.range.span() * if self.reversed { -1.0 } else { 1.0 };
        e.first = scale(e.first, step);
        e.second = scale(scale(e.second, step), step);
        [e.point, e.first, e.second]
            .into_iter()
            .flatten()
            .all(f64::is_finite)
            .then_some(e)
    }
    /// Bounds of a NURBS span's piece over its range, in curve units.
    fn nurbs_bounds(&self) -> Option<exedra_curves::RangeBounds> {
        let Curve::Nurbs(n) = &self.curve else {
            return None;
        };
        n.curve().view().prepare().range_bounds(self.range.bounds())
    }
    fn bounds(&self) -> CurveBounds {
        if let Curve::Nurbs(_) = self.curve {
            return self.nurbs_bounds().map_or(
                CurveBounds {
                    speed: f64::INFINITY,
                    second: f64::INFINITY,
                    magnitude: f64::INFINITY,
                },
                |b| CurveBounds {
                    speed: b.first * self.range.span(),
                    second: b.second * self.range.span() * self.range.span(),
                    magnitude: b
                        .bounds
                        .iter()
                        .flatten()
                        .map(|v| v.abs())
                        .fold(0.0, f64::max),
                },
            );
        }
        CurveBounds {
            speed: self.speed(),
            second: self.second_bound(),
            magnitude: self.magnitude(),
        }
    }
    pub(crate) fn speed(&self) -> f64 {
        match &self.curve {
            Curve::Line(l) => norm(l.direction()) * self.range.span(),
            Curve::Nurbs(_) => self
                .nurbs_bounds()
                .map_or(f64::INFINITY, |b| b.first * self.range.span()),
            curve => {
                curve.harmonic().expect("harmonic curve").1[0]
                    * self.range.span()
                    * (1.0 + 4.0 * super::FRAME_TOLERANCE)
            }
        }
    }
    pub(crate) fn second_bound(&self) -> f64 {
        match &self.curve {
            Curve::Line(_) => 0.0,
            Curve::Nurbs(_) => self.nurbs_bounds().map_or(f64::INFINITY, |b| {
                b.second * self.range.span() * self.range.span()
            }),
            curve => {
                curve.harmonic().expect("harmonic curve").1[0]
                    * self.range.span()
                    * self.range.span()
                    * (1.0 + 4.0 * super::FRAME_TOLERANCE)
            }
        }
    }
    pub(crate) fn magnitude(&self) -> f64 {
        if let Curve::Nurbs(_) = self.curve {
            return self.nurbs_bounds().map_or(f64::INFINITY, |b| {
                b.bounds
                    .iter()
                    .flatten()
                    .map(|v| v.abs())
                    .fold(0.0, f64::max)
            });
        }
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
            (Curve::Nurbs(a), Curve::Nurbs(b)) => {
                // Over the same range in the same sense, two splines of one
                // order, knot vector and weights differ by the spline of
                // their controls' differences, which no point exceeds.
                let (a, b) = (a.curve(), b.curve());
                if self.range != other.range
                    || self.reversed != other.reversed
                    || a.order() != b.order()
                    || a.knots() != b.knots()
                    || a.weights() != b.weights()
                {
                    return f64::INFINITY;
                }
                let scale = a
                    .points()
                    .iter()
                    .chain(b.points())
                    .flatten()
                    .map(|v| v.abs())
                    .fold(1.0, f64::max);
                a.points()
                    .iter()
                    .zip(b.points())
                    .map(|(p, q)| norm(sub(*p, *q)))
                    .fold(0.0, f64::max)
                    + 8.0 * f64::EPSILON * scale
            }
            _ => f64::INFINITY,
        }
    }
}

/// Bounds the derivative of B-A when both rational curves share their basis.
/// Subtracting one control difference removes translation before taking a
/// derivative. The residual rational curve has position radius R, weighted
/// numerator derivative A', and denominator derivative w'; hence
/// |(B-A)'| <= (|A'| + R |w'|) / min(w).
fn nurbs_difference(a: &CurveSpan, b: &CurveSpan) -> Option<(f64, bool)> {
    let (Curve::Nurbs(a_curve), Curve::Nurbs(b_curve)) = (&a.curve, &b.curve) else {
        return None;
    };
    let (ac, bc) = (a_curve.curve(), b_curve.curve());
    if a.range != b.range
        || a.reversed != b.reversed
        || ac.order() != bc.order()
        || ac.knots() != bc.knots()
        || ac.weights() != bc.weights()
    {
        return None;
    }
    let translation = sub(bc.points()[0], ac.points()[0]);
    if !translation.iter().all(|v| v.is_finite()) {
        return None;
    }
    let residual = |i: usize| sub(sub(bc.points()[i], ac.points()[i]), translation);
    let magnitude = ac
        .points()
        .iter()
        .chain(bc.points())
        .flatten()
        .chain(translation.iter())
        .map(|v| v.abs())
        .fold(0.0, f64::max);
    // Includes subtraction of controls/translation, weight normalization,
    // and weighted control arithmetic below. Never treat rounded equality as
    // exact translation without this residual/error allowance.
    let roundoff = 128.0 * f64::EPSILON * magnitude + 32.0 * f64::from_bits(1);
    if !roundoff.is_finite() {
        return None;
    }
    let mut radius = 0.0_f64;
    let mut translated = true;
    for i in 0..ac.points().len() {
        let r = residual(i);
        translated &= r == [0.0; 3];
        let norm = libm::hypot(libm::hypot(r[0], r[1]), r[2]) + roundoff;
        if !norm.is_finite() {
            return None;
        }
        radius = radius.max(norm);
    }
    let max_weight = ac
        .weights()
        .map_or(1.0, |w| w.iter().copied().fold(0.0, f64::max));
    let weight = |i| ac.weights().map_or(1.0, |w| w[i] / max_weight);
    // Work in residual-radius units before multiplying by weights. Legal
    // weights can differ by 2^512; weighting tiny model-space residuals first
    // could underflow before a short knot interval amplifies their derivative.
    let normalized_residual = |i| residual(i).map(|v| v / radius);
    let normalized_roundoff = roundoff / radius;
    let mut min_weight = 1.0_f64;
    let mut numerator = 0.0_f64;
    let mut denominator = 0.0_f64;
    let degree = f64::from(ac.order() - 1);
    for i in 0..ac.points().len() {
        let w = weight(i);
        if w <= 0.0 {
            return None;
        }
        min_weight = min_weight.min(w);
        if i + 1 == ac.points().len() {
            break;
        }
        let width = ac.knots()[i + ac.order() as usize] - ac.knots()[i + 1];
        if width == 0.0 {
            continue;
        }
        let next = weight(i + 1);
        if !width.is_finite() {
            return None;
        }
        let scale_factor = degree * (a.range.span() / width);
        let difference = sub(
            scale(normalized_residual(i + 1), next),
            scale(normalized_residual(i), w),
        );
        let dn = (libm::hypot(libm::hypot(difference[0], difference[1]), difference[2])
            + (w + next) * normalized_roundoff)
            * scale_factor;
        let dw = ((next - w).abs() + 8.0 * f64::EPSILON * (w + next)) * scale_factor;
        if !dn.is_finite() || !dw.is_finite() || dn < f64::MIN_POSITIVE || dw < f64::MIN_POSITIVE {
            return None;
        }
        numerator = numerator.max(dn);
        denominator = denominator.max(dw);
    }
    min_weight *= 1.0 - 8.0 * f64::EPSILON;
    if min_weight <= 0.0 {
        return None;
    }
    let bound = ((numerator + denominator) / min_weight) * radius * (1.0 + 64.0 * f64::EPSILON)
        + 32.0 * f64::from_bits(1);
    bound.is_finite().then_some((bound, translated))
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
        // The bounds below take each span's tangent as Lipschitz, with its
        // second-derivative bound; across a corner it is not.
        for span in [&a, &b] {
            if let Curve::Nurbs(n) = &span.curve
                && let Some(knot) = n.curve().view().corner(span.range.bounds())
            {
                return Err(GeometryError::Kinked { knot });
            }
        }
        let mut result = Self {
            curves: [a, b],
            mixed_bound: 0.0,
        };
        let bounds = result.curves.each_ref().map(CurveSpan::bounds);
        result.mixed_bound = result.bound_mixed_with(&bounds)?;
        let mut work = 0;
        result.certify([0.0, 1.0], 0, &mut work, &bounds)?;
        Ok(result)
    }
    #[cfg(test)]
    fn bound_mixed(&self) -> Result<f64, GeometryError> {
        self.bound_mixed_with(&self.curves.each_ref().map(CurveSpan::bounds))
    }
    fn bound_mixed_with(&self, bounds: &[CurveBounds; 2]) -> Result<f64, GeometryError> {
        let speed = bounds[0].speed + bounds[1].speed;
        let second = bounds[0].second + bounds[1].second;
        let correlated = nurbs_difference(&self.curves[0], &self.curves[1]);
        if let Some((bound, true)) = correlated
            && bound <= second / 16.0
        {
            let bound = speed.min(bound) + 512.0 * f64::EPSILON * speed;
            return if bound.is_finite() {
                Ok(bound)
            } else {
                Err(GeometryError::NonFinite)
            };
        }
        let mut sampled = 0.0_f64;
        // Each derivative difference is Lipschitz with this second-derivative
        // bound. Midpoint samples plus the interval radius bound the entire U
        // range; these samples alone are never treated as a certificate.
        for i in 0..8 {
            let u = (f64::from(i) + 0.5) / 8.0;
            let a = self.curves[0].evaluate(u).ok_or(GeometryError::NonFinite)?;
            let b = self.curves[1].evaluate(u).ok_or(GeometryError::NonFinite)?;
            let difference = sub(b.first, a.first);
            let length = libm::hypot(libm::hypot(difference[0], difference[1]), difference[2]);
            sampled = sampled.max(length + second / 16.0);
        }
        let mut bound = speed.min(sampled);
        if let Some((correlated, _)) = correlated {
            bound = bound.min(correlated);
        }
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
            bound = bound.min(coefficient + 2.0 * bounds[1].speed * (wa - wb).abs());
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

    /// The patch as a B-spline surface, where one traces this patch
    /// exactly: its two source curves as NURBS of one order over one knot
    /// vector ([`NurbsCurve::compatible`]), the rows of a surface linear in V.
    ///
    /// The rows interpolated at equal parameters trace the patch, each
    /// ruling joining the same two points, only when both rows keep their
    /// spans' parameters, as lines and NURBS spans do, or both are conics of
    /// equal sweep in the same sense, whose arcs share one parameter; and
    /// when both rows have the same weights, or the rational surface would
    /// bend its rulings. `None` otherwise. The surface keeps the patch's
    /// points, not its parameters: for conics, its U at a ruling is the
    /// arcs' rational parameter, not the patch's angular one.
    #[must_use]
    pub fn nurbs_rows(&self) -> Option<[NurbsCurve; 2]> {
        let [a, b] = &self.curves;
        let keeps = |s: &CurveSpan| matches!(s.curve, Curve::Line(_) | Curve::Nurbs(_));
        let conic = |s: &CurveSpan| s.curve.harmonic().is_some();
        let shared = (keeps(a) && keeps(b))
            || (conic(a)
                && conic(b)
                && a.range.span() == b.range.span()
                && a.reversed == b.reversed);
        if !shared {
            return None;
        }
        let (a, b) = a.nurbs()?.compatible(&b.nurbs()?)?;
        (a.weights() == b.weights()).then_some([a, b])
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
    fn certify(
        &self,
        interval: [f64; 2],
        depth: u32,
        work: &mut u32,
        bounds: &[CurveBounds; 2],
    ) -> Result<(), GeometryError> {
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
        let difference = sub(b.first, a.first);
        let derivative_difference =
            libm::hypot(libm::hypot(difference[0], difference[1]), difference[2]);
        let mixed = self
            .mixed_bound
            .min(derivative_difference + half * (bounds[0].second + bounds[1].second));
        let distance = libm::hypot(libm::hypot(delta[0], delta[1]), delta[2]) + half * mixed;
        let regular = bounds.iter().zip(ns).all(|(c, n)| {
            let variation = half * (c.second * distance + c.speed * mixed);
            let headroom = 512.0
                * f64::EPSILON
                * c.speed
                * (distance + bounds[0].magnitude + bounds[1].magnitude);
            let lower = dot(n, reference) - variation - headroom;
            lower.is_finite() && lower > 0.0
        });
        if regular {
            return Ok(());
        }
        if depth == 20 {
            return Err(failure());
        }
        self.certify([interval[0], u], depth + 1, work, bounds)?;
        self.certify([u, interval[1]], depth + 1, work, bounds)
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
        let bounds = self.curves.each_ref().map(CurveSpan::bounds);
        let u_speed = bounds[0].speed.max(bounds[1].speed);
        let u_chord =
            bounds[0].second.max(bounds[1].second) / (8.0 * f64::from(n[0]) * f64::from(n[0]));
        let ruling = norm(sub(
            self.curves[1].evaluate(0.5).expect("checked").point,
            self.curves[0].evaluate(0.5).expect("checked").point,
        )) + 0.5 * (bounds[0].speed + bounds[1].speed);
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
        let bounds = self.curves.each_ref().map(CurveSpan::bounds);
        let magnitude = bounds[0].magnitude.max(bounds[1].magnitude);
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
        let second = bounds[0].second.max(bounds[1].second);
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
    fn spline(
        points: alloc::vec::Vec<[f64; 3]>,
        weights: alloc::vec::Vec<f64>,
        range: ParameterRange,
        reversed: bool,
    ) -> CurveSpan {
        let order = crate::usize_to_u32(points.len());
        let mut knots = alloc::vec![0.0; points.len()];
        knots.extend(alloc::vec![1.0; points.len()]);
        CurveSpan::new(
            Curve::nurbs(NurbsCurve::new(order, knots, points, Some(weights)).unwrap()).unwrap(),
            range,
            reversed,
        )
        .unwrap()
    }
    #[test]
    fn correlated_spline_bound_covers_translation_taper_and_small_control_changes() {
        let points = alloc::vec![
            [0.0, 0.0, 0.0],
            [1.0, 2.0, 0.0],
            [3.0, 2.0, 0.0],
            [4.0, 0.0, 0.0]
        ];
        for reversed in [false, true] {
            let range = ParameterRange::new(0.1, 0.9).unwrap();
            let a = spline(
                points.clone(),
                alloc::vec![1.0, 0.7, 1.4, 1.0],
                range,
                reversed,
            );
            for (factor, offset, perturb) in [
                (1.0, [0.0, 0.0, 1.0], 0.0),
                (1.0, [1e6, -1e6, 1.0], 0.0),
                (0.75, [0.5, 0.0, 1.0], 0.0),
                (1.0, [0.0, 0.0, 1.0], 1e-5),
            ] {
                let mut controls: alloc::vec::Vec<_> = points
                    .iter()
                    .map(|p| add(scale(*p, factor), offset))
                    .collect();
                controls[1][1] += perturb;
                let b = spline(controls, alloc::vec![1.0, 0.7, 1.4, 1.0], range, reversed);
                let (bound, translated) = nurbs_difference(&a, &b).unwrap();
                assert_eq!(translated, factor == 1.0 && perturb == 0.0);
                for i in 0..=2000 {
                    let u = f64::from(i) / 2000.0;
                    let actual = norm(sub(
                        a.evaluate(u).unwrap().first,
                        b.evaluate(u).unwrap().first,
                    ));
                    assert!(actual <= bound, "{actual} exceeds {bound} at {u}");
                }
                if factor == 1.0 && perturb == 0.0 && offset[0] == 0.0 {
                    assert!(bound < 1e-9);
                }
            }
            let wrong_weights = spline(
                points.clone(),
                alloc::vec![1.0, 0.9, 1.2, 1.0],
                range,
                reversed,
            );
            assert!(nurbs_difference(&a, &wrong_weights).is_none());
            let other_direction = spline(
                points.clone(),
                alloc::vec![1.0, 0.7, 1.4, 1.0],
                range,
                !reversed,
            );
            assert!(nurbs_difference(&a, &other_direction).is_none());
        }
    }
    #[test]
    fn bounded_knot_endpoint_uses_the_inside_derivative() {
        for reversed in [false, true] {
            for first_piece in [false, true] {
                let make = |z| {
                    let x = if first_piece {
                        [0.0, 1.0, 1.0]
                    } else {
                        [0.0, 0.0, 1.0]
                    };
                    let c = NurbsCurve::new(
                        2,
                        alloc::vec![0.0, 0.0, 0.5, 1.0, 1.0],
                        x.map(|x| [x, 0.0, z]).into(),
                        None,
                    )
                    .unwrap();
                    let range = if first_piece {
                        ParameterRange::new(0.0, 0.5)
                    } else {
                        ParameterRange::new(0.5, 1.0)
                    }
                    .unwrap();
                    CurveSpan::new(Curve::nurbs(c).unwrap(), range, reversed).unwrap()
                };
                let surface = Surface::ruled(make(0.0), make(1.0)).unwrap();
                for u in [0.0, 0.5, 1.0] {
                    let e = surface
                        .evaluate([u, 0.5])
                        .expect("bounded regular patch includes both endpoints");
                    assert_eq!(e.du, [if reversed { -1.0 } else { 1.0 }, 0.0, 0.0]);
                }
            }
        }
    }

    #[test]
    fn small_residuals_are_scaled_before_weighted_derivatives() {
        let span = |points| {
            let curve = NurbsCurve::new(
                2,
                alloc::vec![0.0, 0.0, 1e-200, 1.0, 1.0],
                points,
                Some(alloc::vec![
                    libm::scalbn(1.0, -256),
                    libm::scalbn(1.0, -256),
                    libm::scalbn(1.0, 256)
                ]),
            )
            .unwrap();
            CurveSpan::new(
                Curve::nurbs(curve).unwrap(),
                ParameterRange::new(0.0, 1e-200).unwrap(),
                false,
            )
            .unwrap()
        };
        let a = span(alloc::vec![[0.0;3];3]);
        let b = span(alloc::vec![[0.0; 3], [1e-170, 0.0, 0.0], [0.0; 3]]);
        let (bound, _) = nurbs_difference(&a, &b).unwrap();
        for u in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let d = sub(b.evaluate(u).unwrap().first, a.evaluate(u).unwrap().first);
            let actual = libm::hypot(libm::hypot(d[0], d[1]), d[2]);
            assert!(actual <= bound, "{actual} > {bound}");
        }
        assert!(bound >= 1e-170);
        let ruled = Ruled {
            curves: [a, b],
            mixed_bound: 0.0,
        };
        assert!(
            ruled.bound_mixed().unwrap() >= 1e-170,
            "the generic sampled fallback also needs a non-underflowing norm"
        );
    }

    #[test]
    fn translated_spline_patch_needs_no_fictitious_axial_refinement() {
        let points = alloc::vec![
            [0.0, 0.0, 0.0],
            [1.0, 2.0, 0.0],
            [3.0, 2.0, 0.0],
            [4.0, 0.0, 0.0]
        ];
        let a = spline(
            points.clone(),
            alloc::vec![1.0, 0.7, 1.4, 1.0],
            ParameterRange::UNIT,
            false,
        );
        let b = spline(
            points.iter().map(|p| add(*p, [0.0, 0.0, 1.0])).collect(),
            alloc::vec![1.0, 0.7, 1.4, 1.0],
            ParameterRange::UNIT,
            false,
        );
        let Surface::Ruled(ruled) = Surface::ruled(a, b).unwrap() else {
            unreachable!()
        };
        assert!(ruled.mixed_bound < 1e-9);
        assert!(
            (ruled.triangle_bound([0.01, 1.0]) - ruled.triangle_bound([0.01, 0.01])).abs() < 1e-9
        );
    }
    #[test]
    fn correlated_translation_does_not_bypass_regularity_checks() {
        for controls in [
            alloc::vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            alloc::vec![
                [0.140625, 0.0, 0.0],
                [-0.234375, 0.0, 0.0],
                [0.390625, 0.0, 0.0]
            ],
        ] {
            let a = spline(
                controls.clone(),
                alloc::vec![1.0;controls.len()],
                ParameterRange::UNIT,
                false,
            );
            let direction = if controls.len() == 2 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 0.0, 1.0]
            };
            let b = spline(
                controls.iter().map(|p| add(*p, direction)).collect(),
                alloc::vec![1.0;controls.len()],
                ParameterRange::UNIT,
                false,
            );
            assert!(matches!(
                Surface::ruled(a, b),
                Err(GeometryError::NonRegularRuled { .. })
            ));
        }
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
