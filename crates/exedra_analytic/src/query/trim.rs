// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Continuous chart predicates; no polygonal approximation of curved trims.
use super::{Meter, QueryError};
use crate::geometry::{ParameterRange, frame_coordinates};
use exedra_math::Placement3;

#[derive(Clone, Debug)]
pub(super) enum Curve2 {
    Line([[f64; 2]; 2]),
    Conic {
        center: [f64; 2],
        cosine: [f64; 2],
        sine: [f64; 2],
        range: ParameterRange,
        reversed: bool,
    },
}
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum Region {
    Inside,
    Outside,
    Edge,
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn length(a: [f64; 2]) -> f64 {
    libm::hypot(a[0], a[1])
}
fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn rotate(p: [f64; 2], axis: [f64; 2]) -> [f64; 2] {
    [dot(p, axis), cross(axis, p)]
}
fn unwrapped(t: f64, range: ParameterRange) -> f64 {
    let lo = range.bounds()[0];
    t + core::f64::consts::TAU * libm::ceil((lo - t) / core::f64::consts::TAU)
}
impl Curve2 {
    pub(super) fn from_edge<F, E>(
        body: &crate::body::Body<F, E>,
        face: crate::body::FaceId,
        co: crate::body::Coedge,
    ) -> Option<Self> {
        let surface = &body.face(face)?.surface;
        let frame = surface.frame();
        let edge = body.edge(co.edge)?;
        let (curve, range) = edge.regular()?;
        if let Some((source, axes)) = curve.harmonic() {
            if range.span() > core::f64::consts::TAU * (1.0 + 64.0 * f64::EPSILON) {
                return None;
            }
            let center = frame_coordinates(frame, source.rows.map(|r| r[3]))?;
            let zero = Placement3::from_axes(
                frame.rows.map(|r| r[0]),
                frame.rows.map(|r| r[1]),
                frame.rows.map(|r| r[2]),
                [0.0; 3],
            );
            let cosine = frame_coordinates(zero, source.rows.map(|r| r[0] * axes[0]))?;
            let sine = frame_coordinates(zero, source.rows.map(|r| r[1] * axes[1]))?;
            Some(Self::Conic {
                center: [center[0], center[1]],
                cosine: [cosine[0], cosine[1]],
                sine: [sine[0], sine[1]],
                range,
                reversed: co.reversed,
            })
        } else {
            let at = |f| {
                let p = frame_coordinates(frame, body.edge_point(co.edge, f)?)?;
                Some([p[0], p[1]])
            };
            Some(Self::Line(if co.reversed {
                [at(1.0)?, at(0.0)?]
            } else {
                [at(0.0)?, at(1.0)?]
            }))
        }
    }
    fn near(&self, point: [f64; 2], allowance: f64) -> Option<bool> {
        match *self {
            Self::Line([a, b]) => {
                let d = sub(b, a);
                let len = length(d);
                if !len.is_finite() || len == 0.0 {
                    return None;
                }
                let axis = [d[0] / len, d[1] / len];
                let t = dot(sub(point, a), axis).clamp(0.0, len);
                Some(length(sub(sub(point, a), [axis[0] * t, axis[1] * t])) <= allowance)
            }
            Self::Conic {
                center,
                cosine,
                sine,
                range,
                ..
            } => {
                let determinant = cross(cosine, sine);
                let scale = length(cosine) * length(sine);
                if !determinant.is_finite() || determinant.abs() <= 256.0 * f64::EPSILON * scale {
                    return None;
                }
                let p = sub(point, center);
                let q = [cross(p, sine) / determinant, cross(cosine, p) / determinant];
                let radius = length(q);
                let minimum = determinant.abs() / libm::hypot(length(cosine), length(sine));
                if !radius.is_finite() || !minimum.is_finite() {
                    return None;
                }
                if minimum * (radius - 1.0).abs() > allowance {
                    return Some(false);
                }
                let t = unwrapped(libm::atan2(q[1], q[0]), range);
                let angular = allowance / minimum
                    + 128.0
                        * f64::EPSILON
                        * range.bounds()[0]
                            .abs()
                            .max(range.bounds()[1].abs())
                            .max(1.0);
                Some(
                    t <= range.bounds()[1] + angular
                        || t - core::f64::consts::TAU >= range.bounds()[0] - angular,
                )
            }
        }
    }
    pub(super) fn crossing(&self, point: [f64; 2], axis: [f64; 2], allowance: f64) -> Option<i32> {
        let point = rotate(point, axis);
        match *self {
            Self::Line([a, b]) => {
                let (a, b) = (rotate(a, axis), rotate(b, axis));
                if ((point[1] - a[1]).abs() <= allowance || (point[1] - b[1]).abs() <= allowance)
                    && a[0].max(b[0]) >= point[0] - allowance
                {
                    return None;
                }
                if (a[1] > point[1]) == (b[1] > point[1]) {
                    return Some(0);
                }
                let x = a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if !x.is_finite() || (x - point[0]).abs() <= allowance {
                    return None;
                }
                Some(if x > point[0] {
                    if b[1] > a[1] { 1 } else { -1 }
                } else {
                    0
                })
            }
            Self::Conic {
                center,
                cosine,
                sine,
                range,
                reversed,
            } => {
                let (center, a, b) = (
                    rotate(center, axis),
                    rotate(cosine, axis),
                    rotate(sine, axis),
                );
                let radius = libm::hypot(a[1], b[1]);
                if radius == 0.0 || !radius.is_finite() {
                    return None;
                }
                let height = point[1] - center[1];
                if height.abs() > radius + allowance {
                    return Some(0);
                }
                if (height.abs() - radius).abs() <= allowance {
                    return None;
                }
                let phase = libm::atan2(b[1], a[1]);
                let angle = libm::acos(height / radius);
                // Inverse-trig roots become ill-conditioned near a tangent.
                // Bound their angular sensitivity over the full Y uncertainty
                // interval; allowance/radius would silently miss near-end roots.
                let derivative_min = libm::sqrt(
                    (radius - height.abs() - allowance) * (radius + height.abs() + allowance),
                );
                if !derivative_min.is_finite() || derivative_min <= 0.0 {
                    return None;
                }
                let [lo, hi] = range.bounds();
                let angular = 512.0 * f64::EPSILON * lo.abs().max(hi.abs()).max(1.0)
                    + 2.0 * allowance / derivative_min;
                let mut winding = 0;
                for root in [phase + angle, phase - angle] {
                    let base = unwrapped(root, range);
                    // Keep the preceding periodic copy when a root lies just
                    // below the lower endpoint; unwrapping must not discard it.
                    for t in [base, base - core::f64::consts::TAU] {
                        if t < lo - angular || t > hi + angular {
                            continue;
                        }
                        let (s, c) = (libm::sin(t), libm::cos(t));
                        let x = center[0] + a[0] * c + b[0] * s;
                        if x < point[0] - allowance {
                            continue;
                        }
                        if (x - point[0]).abs() <= allowance
                            || (t - lo).abs() <= angular
                            || (t - hi).abs() <= angular
                        {
                            return None;
                        }
                        let derivative = -a[1] * s + b[1] * c;
                        if derivative.abs() <= allowance {
                            return None;
                        }
                        winding += if (derivative > 0.0) != reversed {
                            1
                        } else {
                            -1
                        };
                    }
                }
                Some(winding)
            }
        }
    }
}

pub(super) fn classify(
    curves: &[Curve2],
    point: [f64; 2],
    allowance: f64,
    meter: &mut Meter,
) -> Result<Region, QueryError> {
    for curve in curves {
        meter.trim()?;
        if curve
            .near(point, allowance)
            .ok_or_else(|| meter.numeric(None, allowance))?
        {
            return Ok(Region::Edge);
        }
    }
    for axis in [
        [1.0, 0.0],
        [0.8944271909999159, 0.4472135954999579],
        [0.4472135954999579, 0.8944271909999159],
    ] {
        let mut winding = Some(0_i32);
        for curve in curves {
            meter.trim()?;
            let Some(crossing) = curve.crossing(point, axis, allowance) else {
                winding = None;
                break;
            };
            winding = winding.and_then(|w| w.checked_add(crossing));
        }
        if let Some(winding) = winding {
            return Ok(if winding == 0 {
                Region::Outside
            } else {
                Region::Inside
            });
        }
    }
    Ok(Region::Edge)
}
