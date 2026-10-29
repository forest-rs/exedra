// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Checked face-local trim bands derived from authoritative shared edge curves.

use super::{Body, Coedge, EdgeId, FaceId, ValidationPolicy};
use crate::geometry::{
    CurveSpan, ParameterDomain, ParameterRange, Surface, frame_coordinates,
    intersection::AxialHarmonic,
};
use exedra_math::{Placement3, add, scale, sub};

/// A checked full-turn cylinder face between two circular/elliptic graph rims.
///
/// This is a derived view: the body's curves, ranges and coedges remain authoritative.
/// Coordinates are native cylinder parameters, never texture coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct CylindricalBand {
    domain: ParameterDomain,
    lower: AxialHarmonic,
    upper: AxialHarmonic,
    rim_edges: [EdgeId; 2],
    seam: EdgeId,
    pub(crate) pair: [Coedge; 2],
    pub(crate) translation: [f64; 3],
    pub(crate) variable_axial_rulings: bool,
    pub(crate) center: [f64; 3],
    max_gap: f64,
}
impl CylindricalBand {
    /// Full-period azimuth interval and conservative axial enclosure.
    #[must_use]
    pub const fn domain(&self) -> ParameterDomain {
        self.domain
    }
    /// Lower and upper axial trim functions, in increasing native height.
    #[must_use]
    pub const fn boundaries(&self) -> [AxialHarmonic; 2] {
        [self.lower, self.upper]
    }
    /// Shared lower/upper rim edge identities, in increasing native height.
    #[must_use]
    pub const fn rim_edges(&self) -> [EdgeId; 2] {
        self.rim_edges
    }
    /// Shared axial seam, occurring in both directions on this face.
    #[must_use]
    pub const fn seam(&self) -> EdgeId {
        self.seam
    }
    /// Whole-curve support/parameter/pairing disagreement bound in model lengths.
    #[must_use]
    pub const fn max_gap(&self) -> f64 {
        self.max_gap
    }
    /// Closed axial interval at an azimuth in the view's declared interval.
    #[must_use]
    pub fn axial_interval(&self, u: f64) -> Option<ParameterRange> {
        if !self.domain.u.contains(u) {
            return None;
        }
        ParameterRange::new(self.lower.at(u)?, self.upper.at(u)?).ok()
    }
    /// Whether a finite native parameter lies in this computed closed band.
    /// The numeric boundary uncertainty remains available in [`Self::max_gap`].
    #[must_use]
    pub fn contains_parameter(&self, uv: [f64; 2]) -> bool {
        self.axial_interval(uv[0])
            .is_some_and(|range| range.contains(uv[1]))
    }
}

/// A face does not establish the bounded cylinder graph-band contract.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum CylindricalBandError {
    /// Agreement policy must be finite and nonnegative.
    InvalidPolicy,
    /// Unknown face, unsupported topology/curve family or inconsistent references.
    Unsupported {
        /// Owning face.
        face: FaceId,
    },
    /// Complete coefficient agreement exceeds the caller's length tolerance.
    Agreement {
        /// Owning face.
        face: FaceId,
        /// Conservative disagreement bound.
        bound: f64,
        /// Allowed model-length disagreement.
        allowed: f64,
    },
    /// The two complete rim functions cannot establish strictly positive width.
    Separation {
        /// Owning face.
        face: FaceId,
        /// Lower bound on axial width before numeric allowance.
        minimum: f64,
    },
}
impl core::fmt::Display for CylindricalBandError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "cylindrical trim band: {self:?}")
    }
}
impl core::error::Error for CylindricalBandError {}

struct Rim {
    axial: AxialHarmonic,
    center: [f64; 3],
    gap: f64,
}
fn norm(v: [f64; 3]) -> f64 {
    libm::hypot(libm::hypot(v[0], v[1]), v[2])
}

impl<F, E> Body<F, E> {
    /// Derives a full-turn trim band from a cylinder's exact two rims and shared seam.
    ///
    /// Whole harmonic coefficients establish the angle/height relationship and
    /// complete radial pairing. Agreeing endpoints or a few sampled positions are
    /// insufficient. No input geometry or face domain is repaired or normalized.
    /// # Errors
    /// Refuses unsupported topology, mismatched complete rims, nonpositive width,
    /// or numeric agreement that cannot meet `policy.max_gap`.
    pub fn cylindrical_band(
        &self,
        id: FaceId,
        policy: &ValidationPolicy,
    ) -> Result<CylindricalBand, CylindricalBandError> {
        use CylindricalBandError as Error;
        if !policy.max_gap.is_finite() || policy.max_gap < 0.0 {
            return Err(Error::InvalidPolicy);
        }
        let refuse = || Error::Unsupported { face: id };
        let face = self.face(id).ok_or_else(refuse)?;
        let Surface::Cylinder(cylinder) = &face.surface else {
            return Err(refuse());
        };
        if face.loops.len() != 1 {
            return Err(refuse());
        }
        let loop_ = self.boundary_loop(face.loops[0]).ok_or_else(refuse)?;
        let coedges = &loop_.coedges;
        if coedges.len() != 4 {
            return Err(refuse());
        }
        for (i, &coedge) in coedges.iter().enumerate() {
            let end = self.coedge_vertices(coedge).ok_or_else(refuse)?[1];
            let start = self
                .coedge_vertices(coedges[(i + 1) % 4])
                .ok_or_else(refuse)?[0];
            if end != start {
                return Err(refuse());
            }
        }
        let first = coedges
            .iter()
            .position(|c| {
                self.edge(c.edge)
                    .is_some_and(|e| e.regular().is_some_and(|(c, _)| c.harmonic().is_some()))
            })
            .ok_or_else(refuse)?;
        let pair = [coedges[first], coedges[(first + 2) % 4]];
        let seams = [coedges[(first + 1) % 4], coedges[(first + 3) % 4]];
        if seams[0].edge != seams[1].edge
            || seams[0].reversed == seams[1].reversed
            || !matches!(
                self.edge(seams[0].edge).and_then(|e| e.regular()),
                Some((crate::geometry::Curve::Line(_), _))
            )
        {
            return Err(refuse());
        }
        let mut endpoint_gap = 0.0_f64;
        for id in [pair[0].edge, pair[1].edge, seams[0].edge] {
            let edge = self.edge(id).ok_or_else(refuse)?;
            let (curve, range) = edge.regular().ok_or_else(refuse)?;
            for (vertex, parameter) in [edge.start, edge.end].into_iter().zip(range.bounds()) {
                let stored = *self.vertices().get(vertex.index()).ok_or_else(refuse)?;
                let evaluated = curve.evaluate(parameter).ok_or_else(refuse)?.point;
                endpoint_gap = endpoint_gap.max(norm(sub(stored, evaluated)));
            }
        }
        let cf = cylinder.frame();
        let zero = Placement3::from_axes(
            cf.rows.map(|r| r[0]),
            cf.rows.map(|r| r[1]),
            cf.rows.map(|r| r[2]),
            [0.0; 3],
        );
        let rim = |coedge: Coedge| -> Option<Rim> {
            let edge = self.edge(coedge.edge)?;
            let (curve, range) = edge.regular()?;
            if edge.start != edge.end
                || (range.span() - core::f64::consts::TAU).abs()
                    > 64.0 * f64::EPSILON * core::f64::consts::TAU
            {
                return None;
            }
            let (frame, [major, minor]) = curve.harmonic()?;
            let center = frame.rows.map(|r| r[3]);
            let world_a = frame.rows.map(|r| r[0] * major);
            let world_b = frame.rows.map(|r| r[1] * minor);
            let local = frame_coordinates(cf, center)?;
            let a = frame_coordinates(zero, world_a)?;
            let b = frame_coordinates(zero, world_b)?;
            let phase = libm::atan2(a[1], a[0]);
            let (sin, cos) = (libm::sin(phase), libm::cos(phase));
            let sign = if cos * b[1] - sin * b[0] < 0.0 {
                -1.0
            } else {
                1.0
            };
            let axial = AxialHarmonic::new(
                local[2],
                a[2] * cos - sign * b[2] * sin,
                a[2] * sin + sign * b[2] * cos,
            )
            .ok()?;
            // Compare complete coefficients against the actual forward basis.
            let ideal_center = add(
                cf.rows.map(|r| r[3]),
                scale(cf.rows.map(|r| r[2]), local[2]),
            );
            let ideal_a = add(
                scale(cf.rows.map(|r| r[0]), cylinder.radius() * cos),
                add(
                    scale(cf.rows.map(|r| r[1]), cylinder.radius() * sin),
                    scale(cf.rows.map(|r| r[2]), a[2]),
                ),
            );
            let ideal_b = add(
                scale(cf.rows.map(|r| r[0]), -sign * cylinder.radius() * sin),
                add(
                    scale(cf.rows.map(|r| r[1]), sign * cylinder.radius() * cos),
                    scale(cf.rows.map(|r| r[2]), b[2]),
                ),
            );
            let magnitude = frame
                .rows
                .iter()
                .chain(&cf.rows)
                .map(|r| r[3].abs())
                .fold(major, f64::max);
            let gap = norm(sub(center, ideal_center))
                + norm(sub(world_a, ideal_a))
                + norm(sub(world_b, ideal_b))
                + 256.0 * f64::EPSILON * magnitude;
            gap.is_finite().then_some(Rim { axial, center, gap })
        };
        let a = rim(pair[0]).ok_or_else(refuse)?;
        let b = rim(pair[1]).ok_or_else(refuse)?;
        let native_delta = frame_coordinates(zero, sub(b.center, a.center)).ok_or_else(refuse)?;
        let translation = scale(cf.rows.map(|r| r[2]), native_delta[2]);
        let lower = self.edge(pair[0].edge).ok_or_else(refuse)?;
        let upper = self.edge(pair[1].edge).ok_or_else(refuse)?;
        let (lower_curve, lower_range) = lower.regular().ok_or_else(refuse)?;
        let (upper_curve, upper_range) = upper.regular().ok_or_else(refuse)?;
        let lower = CurveSpan::new(lower_curve.clone(), lower_range, pair[0].reversed)
            .map_err(|_| refuse())?
            .transformed(Placement3::translate(
                translation[0],
                translation[1],
                translation[2],
            ))
            .map_err(|_| refuse())?;
        let upper = CurveSpan::new(upper_curve.clone(), upper_range, !pair[1].reversed)
            .map_err(|_| refuse())?;
        // Complete radial coefficients must agree at matching traversal fractions.
        // Axial coefficients may differ: each rim is a separate planar section.
        let coefficients = |span: &CurveSpan| -> Option<[[f64; 2]; 3]> {
            let (frame, [major, minor]) = span.curve().harmonic()?;
            let phase = span.range().bounds()[usize::from(span.reversed())];
            let (sin, cos) = (libm::sin(phase), libm::cos(phase));
            let sign = if span.reversed() { -1.0 } else { 1.0 };
            let center = frame_coordinates(cf, frame.rows.map(|r| r[3]))?;
            let a = frame_coordinates(zero, frame.rows.map(|r| r[0] * major))?;
            let b = frame_coordinates(zero, frame.rows.map(|r| r[1] * minor))?;
            Some([
                [center[0], center[1]],
                core::array::from_fn(|i| a[i] * cos + b[i] * sin),
                core::array::from_fn(|i| sign * (-a[i] * sin + b[i] * cos)),
            ])
        };
        let radial_gap = coefficients(&lower)
            .ok_or_else(refuse)?
            .into_iter()
            .zip(coefficients(&upper).ok_or_else(refuse)?)
            .map(|(a, b)| libm::hypot(a[0] - b[0], a[1] - b[1]))
            .sum::<f64>()
            + 64.0 * f64::EPSILON * cylinder.radius();
        let gap = a.gap.max(b.gap) + radial_gap + endpoint_gap;
        if !gap.is_finite() || gap > policy.max_gap {
            return Err(Error::Agreement {
                face: id,
                bound: gap,
                allowed: policy.max_gap,
            });
        }
        let variable_axial_rulings = lower.alignment(&upper) > policy.max_gap;
        let (lower, upper, edges) = if a.axial.coefficients()[0] < b.axial.coefficients()[0] {
            (a.axial, b.axial, [pair[0].edge, pair[1].edge])
        } else {
            (b.axial, a.axial, [pair[1].edge, pair[0].edge])
        };
        let lo = lower.coefficients();
        let hi = upper.coefficients();
        let minimum = (hi[0] - lo[0] - (hi[1] - lo[1]).abs() - (hi[2] - lo[2]).abs()).next_down();
        if minimum * norm(cf.rows.map(|r| r[2])) <= 2.0 * gap {
            return Err(Error::Separation { face: id, minimum });
        }
        let u = face.domain.u;
        if (u.span() - core::f64::consts::TAU).abs() > 64.0 * f64::EPSILON * core::f64::consts::TAU
        {
            return Err(refuse());
        }
        let [lo, hi] = [lower.bounds()[0], upper.bounds()[1]];
        let v = ParameterRange::new(lo, hi).map_err(|_| refuse())?;
        if face.domain.v.bounds()[0] > lo || face.domain.v.bounds()[1] < hi {
            return Err(refuse());
        }
        Ok(CylindricalBand {
            domain: ParameterDomain { u, v },
            lower,
            upper,
            rim_edges: edges,
            seam: seams[0].edge,
            pair,
            translation,
            variable_axial_rulings,
            center: a.center,
            max_gap: gap,
        })
    }
}
