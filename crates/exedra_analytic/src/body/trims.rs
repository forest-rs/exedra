// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit parameter correspondence, independent of construction provenance.

use super::{Body, BodyError, FaceId, LoopId, ValidationPolicy, ValidationReport};
use crate::geometry::{Curve, ParameterRange, Surface};
use alloc::vec::Vec;
use exedra_math::{Placement3, add, norm, scale, sub};

/// A parameter curve in one owning face's chart.
///
/// Fractions always follow the shared edge's canonical start-to-end direction;
/// [`super::Coedge::reversed`] changes traversal, never this correspondence.
/// Coordinates are surface parameters (radians and/or model units), not texture UVs.
#[derive(Clone, Debug, PartialEq)]
pub enum TrimCurve {
    /// An affine chart segment, including distinct copies of a periodic seam.
    Line {
        /// Parameters at the shared edge's start.
        start: [f64; 2],
        /// Parameters at the shared edge's end.
        end: [f64; 2],
    },
    /// `origin + linear*t + cosine*cos(t) + sine*sin(t)`.
    ///
    /// The bounded increasing `range` maps normalized edge fractions to `t`.
    /// This includes planar ellipses and oblique plane/cylinder intersections,
    /// whose cylindrical chart has affine U and harmonic V.
    Harmonic {
        /// Constant chart coordinates.
        origin: [f64; 2],
        /// Coefficient of the angular parameter.
        linear: [f64; 2],
        /// Cosine coefficients.
        cosine: [f64; 2],
        /// Sine coefficients.
        sine: [f64; 2],
        /// Increasing angular interval, in radians.
        range: ParameterRange,
    },
}
impl TrimCurve {
    /// Evaluates at a canonical edge fraction in `[0,1]` without chart wrapping.
    #[must_use]
    pub fn evaluate(&self, fraction: f64) -> Option<[f64; 2]> {
        if !(0.0..=1.0).contains(&fraction) {
            return None;
        }
        let p = match *self {
            Self::Line { start, end } => core::array::from_fn(|i| {
                if fraction == 0.0 {
                    start[i]
                } else if fraction == 1.0 {
                    end[i]
                } else {
                    (1.0 - fraction) * start[i] + fraction * end[i]
                }
            }),
            Self::Harmonic {
                origin,
                linear,
                cosine,
                sine,
                range,
            } => {
                let t = range.at(fraction)?;
                let (s, c) = (libm::sin(t), libm::cos(t));
                core::array::from_fn(|i| origin[i] + linear[i] * t + cosine[i] * c + sine[i] * s)
            }
        };
        p.iter().all(|v| v.is_finite()).then_some(p)
    }

    /// Conservative axis-aligned chart bounds over the complete parameter curve.
    /// Returns `None` for non-finite coefficients or unrepresentable arithmetic.
    #[must_use]
    pub fn bounds(&self) -> Option<[[f64; 2]; 2]> {
        let result = match *self {
            Self::Line { start, end } => {
                core::array::from_fn(|i| [start[i].min(end[i]), start[i].max(end[i])])
            }
            Self::Harmonic {
                origin,
                linear,
                cosine,
                sine,
                range,
            } => {
                if !origin
                    .into_iter()
                    .chain(linear)
                    .chain(cosine)
                    .chain(sine)
                    .all(f64::is_finite)
                {
                    return None;
                }
                let [lo, hi] = range.bounds();
                core::array::from_fn(|i| {
                    let at = |t: f64| {
                        origin[i]
                            + linear[i] * t
                            + cosine[i] * libm::cos(t)
                            + sine[i] * libm::sin(t)
                    };
                    let (a, b) = (at(lo), at(hi));
                    let mut bounds = [a.min(b), a.max(b)];
                    let radius = libm::hypot(cosine[i], sine[i]);
                    if radius > 0.0 && linear[i].abs() <= radius {
                        // derivative = linear + radius*cos(t + phase).
                        let phase = libm::atan2(cosine[i], sine[i]);
                        let root = libm::acos(-linear[i] / radius);
                        for base in [root - phase, -root - phase] {
                            let first = base
                                + core::f64::consts::TAU
                                    * libm::ceil((lo - base) / core::f64::consts::TAU);
                            let last = base
                                + core::f64::consts::TAU
                                    * libm::floor((hi - base) / core::f64::consts::TAU);
                            for t in [first, last] {
                                if t >= lo && t <= hi {
                                    let value = at(t);
                                    bounds[0] = bounds[0].min(value);
                                    bounds[1] = bounds[1].max(value);
                                }
                            }
                        }
                    }
                    bounds
                })
            }
        };
        // Explicit checks also reject NaN line endpoints (f64::min ignores NaN).
        self.evaluate(0.0)?;
        self.evaluate(1.0)?;
        result
            .into_iter()
            .flatten()
            .all(f64::is_finite)
            .then_some(result)
    }

    /// Conservative absolute U/V derivatives in normalized edge-fraction units.
    /// Invalid or overflowed coefficients may produce non-finite bounds; callers
    /// must validate the owning body before planning sampling from them.
    #[must_use]
    pub fn derivative_bounds(&self) -> [f64; 2] {
        match *self {
            Self::Line { start, end } => core::array::from_fn(|i| (end[i] - start[i]).abs()),
            Self::Harmonic {
                linear,
                cosine,
                sine,
                range,
                ..
            } => core::array::from_fn(|i| {
                (linear[i].abs() + libm::hypot(cosine[i], sine[i])) * range.span()
            }),
        }
    }
    fn coefficients(&self) -> Option<Harmonic<2>> {
        match *self {
            Self::Line { start, end } => Some(Harmonic {
                origin: start,
                linear: core::array::from_fn(|i| end[i] - start[i]),
                cosine: [0.0; 2],
                sine: [0.0; 2],
                frequency: 0.0,
            }),
            Self::Harmonic {
                origin,
                linear,
                cosine,
                sine,
                range,
            } => {
                let t = range.bounds()[0];
                let (s, c) = (libm::sin(t), libm::cos(t));
                Some(Harmonic {
                    origin: core::array::from_fn(|i| origin[i] + linear[i] * t),
                    linear: linear.map(|x| x * range.span()),
                    cosine: core::array::from_fn(|i| cosine[i] * c + sine[i] * s),
                    sine: core::array::from_fn(|i| -cosine[i] * s + sine[i] * c),
                    frequency: range.span(),
                })
            }
        }
    }
}

/// Explicit chart curves for every occurrence in a uniquely owned boundary loop.
///
/// A record must have exactly one face owner and exactly one curve per coedge.
/// The same shared edge may have different curves at its two seam occurrences.
/// Missing records mean only support agreement was checked for that loop.
#[derive(Clone, Debug, PartialEq)]
pub struct LoopTrims {
    /// Boundary loop whose occurrence order defines `curves`.
    pub loop_id: LoopId,
    /// Parameter curves in coedge order, each in canonical edge direction.
    pub curves: Vec<TrimCurve>,
}

impl<F, E> Body<F, E> {
    pub(super) fn validate_trims(
        &self,
        policy: &ValidationPolicy,
        report: &mut ValidationReport,
    ) -> Result<(), BodyError> {
        // Legacy constructors pay no trim-table or owner-table allocation cost.
        if self.trims().is_empty() {
            return Ok(());
        }
        let mut owners = alloc::vec![None; self.loops().len()];
        let mut multiple = alloc::vec![false; self.loops().len()];
        for (i, face) in self.faces().iter().enumerate() {
            for id in &face.loops {
                if owners[id.index()]
                    .replace(FaceId(crate::usize_to_u32(i)))
                    .is_some()
                {
                    multiple[id.index()] = true;
                }
            }
        }
        let mut seen = alloc::vec![false; self.loops().len()];
        for trims in self.trims() {
            let id = trims.loop_id;
            let fail = |reason| BodyError::TrimStructure {
                loop_id: id,
                reason,
            };
            let ring = self.boundary_loop(id).ok_or_else(|| fail("missing loop"))?;
            let face_id = owners[id.index()].ok_or_else(|| fail("no owning face"))?;
            if multiple[id.index()] {
                return Err(fail("multiple owning face uses"));
            }
            if core::mem::replace(&mut seen[id.index()], true) {
                return Err(fail("duplicate trim record"));
            }
            if ring.coedges.len() != trims.curves.len() {
                return Err(fail("curve count differs from coedge count"));
            }
            let face = &self.faces()[face_id.index()];
            for (i, (coedge, trim)) in ring.coedges.iter().zip(&trims.curves).enumerate() {
                let bad_domain = || BodyError::TrimDomain {
                    face: face_id,
                    loop_id: id,
                    coedge: i,
                };
                let bounds = trim.bounds().ok_or_else(bad_domain)?;
                for (actual, domain) in bounds.into_iter().zip([face.domain.u, face.domain.v]) {
                    let [lo, hi] = domain.bounds();
                    let roundoff = 128.0 * f64::EPSILON * lo.abs().max(hi.abs()).max(1.0);
                    if actual[0] < lo - roundoff || actual[1] > hi + roundoff {
                        return Err(bad_domain());
                    }
                }
                let edge = &self.edges()[coedge.edge.index()];
                let bound = match (&edge.geometry, composed(&face.surface, trim)) {
                    (super::EdgeGeometry::Curve { curve, range }, Some(actual)) => {
                        actual.gap(&curve_coefficients(curve, *range))
                    }
                    (super::EdgeGeometry::Collapsed, Some(actual)) => {
                        actual.gap(&Harmonic::point(self.vertices()[edge.start.index()]))
                    }
                    // Cone pole position is meaningful although its normal isn't.
                    (super::EdgeGeometry::Collapsed, None) => match (&face.surface, trim) {
                        (Surface::Cone(cone), TrimCurve::Line { start, end })
                            if start[1] == end[1] && Some(start[1]) == cone.apex_parameter() =>
                        {
                            cone.apex_point().map_or(f64::INFINITY, |p| {
                                norm(sub(p, self.vertices()[edge.start.index()]))
                            })
                        }
                        _ => f64::INFINITY,
                    },
                    _ => f64::INFINITY,
                };
                let bound = bound + correspondence_roundoff(&face.surface, trim, edge.regular());
                if !bound.is_finite() || bound > policy.max_gap {
                    return Err(BodyError::TrimCorrespondence {
                        face: face_id,
                        loop_id: id,
                        coedge: i,
                        bound,
                        allowed: policy.max_gap,
                    });
                }
                report.max_gap = report.max_gap.max(bound);
                report.max_trim_gap = report.max_trim_gap.max(bound);
                report.trimmed_coedges += 1;
                let next = (i + 1) % ring.coedges.len();
                let end = trim
                    .evaluate(if coedge.reversed { 0.0 } else { 1.0 })
                    .ok_or_else(bad_domain)?;
                let start = trims.curves[next]
                    .evaluate(if ring.coedges[next].reversed {
                        1.0
                    } else {
                        0.0
                    })
                    .ok_or_else(bad_domain)?;
                // Chart closure is strict up to floating arithmetic, independent
                // of model-space healing tolerances and periodic identification.
                if (0..2).any(|axis| {
                    (end[axis] - start[axis]).abs()
                        > 256.0 * f64::EPSILON * end[axis].abs().max(start[axis].abs()).max(1.0)
                }) {
                    return Err(BodyError::TrimClosure {
                        face: face_id,
                        loop_id: id,
                        coedge: i,
                    });
                }
            }
        }
        Ok(())
    }
}

// Retain input-operand scale rather than the potentially cancelled output
// magnitude. Parameter interpolation and phase composition also spend error.
fn correspondence_roundoff(
    surface: &Surface,
    trim: &TrimCurve,
    edge: Option<(&Curve, ParameterRange)>,
) -> f64 {
    let uv_scale = match *trim {
        TrimCurve::Line { start, end } => start
            .into_iter()
            .chain(end)
            .map(f64::abs)
            .fold(1.0, f64::max),
        TrimCurve::Harmonic {
            origin,
            linear,
            cosine,
            sine,
            range,
        } => {
            let t = range.bounds().into_iter().map(f64::abs).fold(1.0, f64::max);
            origin
                .into_iter()
                .chain(linear.map(|v| v * t))
                .chain(cosine)
                .chain(sine)
                .map(f64::abs)
                .fold(t, f64::max)
        }
    };
    let surface_scale = match surface {
        Surface::Plane(_) => 1.0,
        Surface::Cylinder(c) => 1.0 + c.radius(),
        Surface::Cone(c) => 1.0 + c.radius() + c.slope().abs(),
        Surface::Sphere(s) => 1.0 + s.radius(),
        Surface::Torus(t) => 1.0 + t.major_radius() + t.minor_radius(),
        Surface::Ruled(r) => {
            1.0 + r
                .curves()
                .iter()
                .map(crate::geometry::CurveSpan::magnitude)
                .fold(0.0, f64::max)
        }
    };
    let placement = surface
        .frame()
        .rows
        .into_iter()
        .flatten()
        .map(f64::abs)
        .fold(1.0, f64::max);
    let edge_scale = edge.map_or(0.0, |(curve, range)| {
        let t = range.bounds().into_iter().map(f64::abs).fold(1.0, f64::max);
        match curve {
            Curve::Line(l) => l
                .origin()
                .into_iter()
                .chain(l.direction().map(|v| v * t))
                .map(f64::abs)
                .fold(1.0, f64::max),
            _ => {
                let (frame, radii) = curve.harmonic().expect("harmonic");
                frame
                    .rows
                    .into_iter()
                    .flatten()
                    .map(f64::abs)
                    .fold(1.0, f64::max)
                    + radii[0] * t
            }
        }
    });
    1024.0 * f64::EPSILON * (placement + uv_scale * surface_scale + edge_scale)
}

#[derive(Clone, Copy)]
struct Harmonic<const N: usize> {
    origin: [f64; N],
    linear: [f64; N],
    cosine: [f64; N],
    sine: [f64; N],
    frequency: f64,
}
impl Harmonic<3> {
    fn point(origin: [f64; 3]) -> Self {
        Self {
            origin,
            linear: [0.0; 3],
            cosine: [0.0; 3],
            sine: [0.0; 3],
            frequency: 0.0,
        }
    }
    fn placed(self, frame: Placement3) -> Self {
        Self {
            origin: frame.transform_point(self.origin),
            linear: frame.transform_vector(self.linear),
            cosine: frame.transform_vector(self.cosine),
            sine: frame.transform_vector(self.sine),
            frequency: self.frequency,
        }
    }
    fn gap(&self, other: &Self) -> f64 {
        let mut gap = norm(sub(self.origin, other.origin))
            + norm(sub(self.linear, other.linear))
            + norm(sub(self.cosine, other.cosine))
            + norm(sub(self.sine, other.sine));
        gap += (norm(self.cosine) + norm(self.sine)).min(norm(other.cosine) + norm(other.sine))
            * (self.frequency - other.frequency).abs();
        let magnitude = self
            .origin
            .into_iter()
            .chain(other.origin)
            .map(f64::abs)
            .fold(
                norm(self.cosine)
                    + norm(self.sine)
                    + norm(self.linear)
                    + norm(other.cosine)
                    + norm(other.sine)
                    + norm(other.linear),
                f64::max,
            );
        gap + 512.0 * f64::EPSILON * magnitude
    }
}
fn curve_coefficients(curve: &Curve, range: ParameterRange) -> Harmonic<3> {
    match curve {
        Curve::Line(line) => Harmonic {
            origin: add(line.origin(), scale(line.direction(), range.bounds()[0])),
            linear: scale(line.direction(), range.span()),
            ..Harmonic::point([0.0; 3])
        },
        _ => {
            let (frame, [a, b]) = curve.harmonic().expect("harmonic curve");
            let t = range.bounds()[0];
            let (s, c) = (libm::sin(t), libm::cos(t));
            Harmonic {
                origin: [0.0; 3],
                linear: [0.0; 3],
                cosine: [a * c, b * s, 0.0],
                sine: [-a * s, b * c, 0.0],
                frequency: range.span(),
            }
            .placed(frame)
        }
    }
}
fn composed(surface: &Surface, trim: &TrimCurve) -> Option<Harmonic<3>> {
    let uv = trim.coefficients()?;
    let affine = |axis: usize| uv.cosine[axis] == 0.0 && uv.sine[axis] == 0.0;
    let constant = |axis: usize| affine(axis) && uv.linear[axis] == 0.0;
    let circle = |radius: f64, height: f64, angle: f64, speed: f64| {
        let (s, c) = (libm::sin(angle), libm::cos(angle));
        let sign = if speed < 0.0 { -1.0 } else { 1.0 };
        Harmonic {
            origin: [0.0, 0.0, height],
            linear: [0.0; 3],
            cosine: [radius * c, radius * s, 0.0],
            sine: [-sign * radius * s, sign * radius * c, 0.0],
            frequency: speed.abs(),
        }
    };
    let result = match surface {
        Surface::Plane(_) => Harmonic {
            origin: [uv.origin[0], uv.origin[1], 0.0],
            linear: [uv.linear[0], uv.linear[1], 0.0],
            cosine: [uv.cosine[0], uv.cosine[1], 0.0],
            sine: [uv.sine[0], uv.sine[1], 0.0],
            frequency: uv.frequency,
        },
        Surface::Cylinder(c) if affine(0) && uv.linear[0] != 0.0 => {
            if (!affine(1)) && uv.frequency != uv.linear[0].abs() {
                return None;
            }
            let mut h = circle(c.radius(), uv.origin[1], uv.origin[0], uv.linear[0]);
            h.linear[2] = uv.linear[1];
            h.cosine[2] = uv.cosine[1];
            h.sine[2] = uv.sine[1];
            h
        }
        Surface::Cylinder(c) if constant(0) => {
            let (s, c0) = (libm::sin(uv.origin[0]), libm::cos(uv.origin[0]));
            Harmonic {
                origin: [c.radius() * c0, c.radius() * s, uv.origin[1]],
                linear: [0.0, 0.0, uv.linear[1]],
                cosine: [0.0, 0.0, uv.cosine[1]],
                sine: [0.0, 0.0, uv.sine[1]],
                frequency: uv.frequency,
            }
        }
        Surface::Cone(c) if constant(1) && affine(0) => circle(
            c.radius_at(uv.origin[1])?,
            uv.origin[1],
            uv.origin[0],
            uv.linear[0],
        ),
        Surface::Cone(c) if constant(0) => {
            let (s, co) = (libm::sin(uv.origin[0]), libm::cos(uv.origin[0]));
            let radius = c
                .radius_at(uv.origin[1])
                .or_else(|| (Some(uv.origin[1]) == c.apex_parameter()).then_some(0.0))?;
            let v = |a: f64| [c.slope() * a * co, c.slope() * a * s, a];
            Harmonic {
                origin: [radius * co, radius * s, uv.origin[1]],
                linear: v(uv.linear[1]),
                cosine: v(uv.cosine[1]),
                sine: v(uv.sine[1]),
                frequency: uv.frequency,
            }
        }
        Surface::Sphere(s) if constant(1) && affine(0) => circle(
            s.radius() * libm::cos(uv.origin[1]),
            s.radius() * libm::sin(uv.origin[1]),
            uv.origin[0],
            uv.linear[0],
        ),
        Surface::Sphere(s) if constant(0) && affine(1) => {
            let h = circle(s.radius(), 0.0, uv.origin[1], uv.linear[1]);
            let (su, cu) = (libm::sin(uv.origin[0]), libm::cos(uv.origin[0]));
            let map = |p: [f64; 3]| [p[0] * cu, p[0] * su, p[1]];
            Harmonic {
                origin: map(h.origin),
                linear: map(h.linear),
                cosine: map(h.cosine),
                sine: map(h.sine),
                frequency: h.frequency,
            }
        }
        Surface::Torus(t) if constant(1) && affine(0) => circle(
            t.major_radius() + t.minor_radius() * libm::cos(uv.origin[1]),
            t.minor_radius() * libm::sin(uv.origin[1]),
            uv.origin[0],
            uv.linear[0],
        ),
        Surface::Torus(t) if constant(0) && affine(1) => {
            let h = circle(t.minor_radius(), 0.0, uv.origin[1], uv.linear[1]);
            let (su, cu) = (libm::sin(uv.origin[0]), libm::cos(uv.origin[0]));
            let map = |p: [f64; 3]| [p[0] * cu, p[0] * su, p[1]];
            Harmonic {
                origin: [t.major_radius() * cu, t.major_radius() * su, 0.0],
                linear: [0.0; 3],
                cosine: map(h.cosine),
                sine: map(h.sine),
                frequency: h.frequency,
            }
        }
        _ => return None,
    };
    Some(result.placed(surface.frame()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::body::{BodyParts, EdgeId};
    use alloc::vec;
    use core::f64::consts::TAU;
    pub(crate) fn cylinder_parts() -> BodyParts {
        let mut parts = crate::body::tests::cylinder_parts();
        let line = |start, end| TrimCurve::Line { start, end };
        parts.trims = vec![
            LoopTrims {
                loop_id: LoopId(0),
                curves: vec![
                    line([0.0, 0.0], [TAU, 0.0]),
                    line([TAU, 0.0], [TAU, 3.0]),
                    line([0.0, 3.0], [TAU, 3.0]),
                    line([0.0, 0.0], [0.0, 3.0]),
                ],
            },
            LoopTrims {
                loop_id: LoopId(1),
                curves: vec![TrimCurve::Harmonic {
                    origin: [0.0; 2],
                    linear: [0.0; 2],
                    cosine: [2.0, 0.0],
                    sine: [0.0, 2.0],
                    range: ParameterRange::new(0.0, TAU).unwrap(),
                }],
            },
            LoopTrims {
                loop_id: LoopId(2),
                curves: vec![TrimCurve::Harmonic {
                    origin: [0.0; 2],
                    linear: [0.0; 2],
                    cosine: [2.0, 0.0],
                    sine: [0.0, 2.0],
                    range: ParameterRange::new(0.0, TAU).unwrap(),
                }],
            },
        ];
        parts
    }
    const POLICY: ValidationPolicy = ValidationPolicy { max_gap: 1e-10 };
    #[test]
    fn canonical_curves_retain_distinct_seam_parameters_and_move_roles() {
        let body = Body::try_from_parts(cylinder_parts(), &POLICY).unwrap();
        assert!(body.is_closed());
        assert!(body.has_explicit_trims(FaceId(0)));
        for face in body.faces() {
            for &loop_id in &face.loops {
                for (i, coedge) in body
                    .boundary_loop(loop_id)
                    .unwrap()
                    .coedges
                    .iter()
                    .enumerate()
                {
                    for fraction in [0.0, 0.17, 0.5, 0.89, 1.0] {
                        let uv = body
                            .trim_curve(loop_id, i)
                            .unwrap()
                            .evaluate(fraction)
                            .unwrap();
                        assert!(
                            norm(sub(
                                face.surface.evaluate(uv).unwrap().point,
                                body.edge_point(coedge.edge, fraction).unwrap()
                            )) < 1e-12
                        );
                    }
                }
            }
        }
        assert_eq!(
            body.trim_curve(LoopId(0), 1).unwrap().evaluate(0.5),
            Some([TAU, 1.5])
        );
        assert_eq!(
            body.trim_curve(LoopId(0), 3).unwrap().evaluate(0.5),
            Some([0.0, 1.5])
        );
        let copied = body.clone().map_roles(|()| 42_u32, |()| "edge");
        assert_eq!(copied.trims(), body.trims());
        assert_eq!(copied.faces()[0].role, 42);
        assert_eq!(copied.edges()[EdgeId(2).index()].role, "edge");
    }
    #[test]
    fn correspondence_checks_whole_curve_despite_equal_endpoints() {
        let mut p = cylinder_parts();
        // Opposite circle traversal has exactly the same shared endpoints.
        if let TrimCurve::Harmonic { sine, .. } = &mut p.trims[1].curves[0] {
            sine[1] = -2.0;
        }
        assert!(matches!(
            Body::try_from_parts(p, &POLICY),
            Err(BodyError::TrimCorrespondence {
                face: FaceId(1),
                coedge: 0,
                ..
            })
        ));
        let mut p = cylinder_parts();
        p.trims[0].curves[0] = TrimCurve::Harmonic {
            origin: [0.0; 2],
            linear: [1.0, 0.0],
            cosine: [0.0, 0.0],
            sine: [0.0, 0.2],
            range: ParameterRange::new(0.0, TAU).unwrap(),
        };
        p.faces[0].domain.v = ParameterRange::new(-0.3, 3.0).unwrap();
        assert!(matches!(
            Body::try_from_parts(p, &POLICY),
            Err(BodyError::TrimCorrespondence {
                face: FaceId(0),
                coedge: 0,
                ..
            })
        ));
    }
    #[test]
    fn chart_domain_closure_and_unique_loop_ownership_are_separate_failures() {
        let mut p = cylinder_parts();
        p.trims[0].curves[1] = TrimCurve::Line {
            start: [0.0, 0.0],
            end: [0.0, 3.0],
        };
        assert!(matches!(
            Body::try_from_parts(p, &POLICY),
            Err(BodyError::TrimClosure { .. })
        ));
        let mut p = cylinder_parts();
        p.faces[0].domain.u = ParameterRange::new(0.0, TAU * 0.9).unwrap();
        assert!(matches!(
            Body::try_from_parts(p, &POLICY),
            Err(BodyError::TrimDomain { .. })
        ));
        let mut p = cylinder_parts();
        p.faces.push(p.faces[0].clone());
        assert!(matches!(
            Body::try_from_parts(p, &POLICY),
            Err(BodyError::TrimStructure {
                reason: "multiple owning face uses",
                ..
            })
        ));
        let mut p = cylinder_parts();
        p.trims.push(p.trims[0].clone());
        assert!(matches!(
            Body::try_from_parts(p, &POLICY),
            Err(BodyError::TrimStructure {
                reason: "duplicate trim record",
                ..
            })
        ));
    }
    #[test]
    fn harmonic_bounds_find_interior_stationary_points() {
        let trim = TrimCurve::Harmonic {
            origin: [2.0, -3.0],
            linear: [0.2, -0.1],
            cosine: [1.5, 0.7],
            sine: [0.6, -2.0],
            range: ParameterRange::new(-0.7, 5.9).unwrap(),
        };
        let bounds = trim.bounds().unwrap();
        for i in 0..=1000 {
            let p = trim.evaluate(f64::from(i) / 1000.0).unwrap();
            for axis in 0..2 {
                assert!(p[axis] >= bounds[axis][0] - 1e-14 && p[axis] <= bounds[axis][1] + 1e-14);
            }
        }
    }
    #[test]
    fn affine_plus_harmonic_cylinder_section_matches_exact_oblique_ellipse() {
        let c = core::f64::consts::FRAC_1_SQRT_2;
        let curve = Curve::ellipse(
            Placement3::from_axes([c, 0.0, c], [0.0, 1.0, 0.0], [-c, 0.0, c], [0.0, 0.0, 2.0]),
            2.0_f64.sqrt(),
            1.0,
        )
        .unwrap();
        let trim = TrimCurve::Harmonic {
            origin: [0.0, 2.0],
            linear: [1.0, 0.0],
            cosine: [0.0, 1.0],
            sine: [0.0, 0.0],
            range: ParameterRange::new(0.0, TAU).unwrap(),
        };
        let surface = Surface::cylinder(Placement3::IDENTITY, 1.0).unwrap();
        assert!(
            composed(&surface, &trim).unwrap().gap(&curve_coefficients(
                &curve,
                ParameterRange::new(0.0, TAU).unwrap()
            )) < 1e-11
        );
    }
}
