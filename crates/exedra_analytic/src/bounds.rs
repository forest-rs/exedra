// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Finite conservative bounds of native supporting patches, independent of meshing.

use crate::{
    body::{Body, FaceId},
    geometry::{Curve, ParameterDomain, ParameterRange, Surface},
};
use exedra_math::Placement3;

/// A finite native support enclosure could not be represented.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BoundsError {
    /// There are no native faces to enclose.
    EmptyBody,
    /// An evaluated position or conservative extent exceeds finite coordinates.
    NumericLimit {
        /// Offending native face, when queried through a body.
        face: Option<FaceId>,
    },
}
impl core::fmt::Display for BoundsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyBody => f.write_str("native bounds require at least one face"),
            Self::NumericLimit { face } => {
                write!(f, "native support bounds exceed numeric limits on {face:?}")
            }
        }
    }
}
impl core::error::Error for BoundsError {}
fn numeric() -> BoundsError {
    BoundsError::NumericLimit { face: None }
}

struct Bounds([[f64; 3]; 2]);
impl Bounds {
    fn new() -> Self {
        Self([[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]])
    }
    fn point(&mut self, point: [f64; 3]) -> Result<(), BoundsError> {
        if point.iter().any(|v| !v.is_finite()) {
            return Err(numeric());
        }
        for (k, value) in point.into_iter().enumerate() {
            self.0[0][k] = self.0[0][k].min(value);
            self.0[1][k] = self.0[1][k].max(value);
        }
        Ok(())
    }
    fn include(&mut self, bounds: [[f64; 3]; 2]) -> Result<(), BoundsError> {
        self.point(bounds[0])?;
        self.point(bounds[1])
    }
    fn finish(mut self) -> Result<[[f64; 3]; 2], BoundsError> {
        for k in 0..3 {
            let scale = self.0[0][k].abs().max(self.0[1][k].abs());
            let allowance = scale * (32.0 * f64::EPSILON);
            self.0[0][k] = (self.0[0][k] - allowance).next_down();
            self.0[1][k] = (self.0[1][k] + allowance).next_up();
        }
        if self.0.iter().flatten().any(|v| !v.is_finite()) {
            return Err(numeric());
        }
        Ok(self.0)
    }
}
fn product(a: f64, b: f64) -> Result<[f64; 2], BoundsError> {
    let value = a * b;
    if !value.is_finite() {
        return Err(numeric());
    }
    Ok([value.next_down(), value.next_up()])
}
fn sum(a: [f64; 2], b: [f64; 2]) -> Result<[f64; 2], BoundsError> {
    let result = [(a[0] + b[0]).next_down(), (a[1] + b[1]).next_up()];
    if result.iter().any(|v| !v.is_finite()) {
        return Err(numeric());
    }
    Ok(result)
}
fn linear(origin: f64, direction: f64, t: f64) -> Result<[f64; 2], BoundsError> {
    sum([origin; 2], product(direction, t)?)
}
fn transformed(frame: Placement3, point: [f64; 3]) -> Result<[[f64; 3]; 2], BoundsError> {
    let mut result = [[0.0; 3]; 2];
    for (k, row) in frame.rows.into_iter().enumerate() {
        let mut value = [row[3]; 2];
        for (a, b) in row[..3].iter().zip(point) {
            value = sum(value, product(*a, b)?)?;
        }
        result[0][k] = value[0];
        result[1][k] = value[1];
    }
    Ok(result)
}
fn circle(frame: Placement3, radius: f64, height: f64) -> Result<[[f64; 3]; 2], BoundsError> {
    let center = transformed(frame, [0.0, 0.0, height])?;
    let mut bounds = Bounds::new();
    let mut extrema = [[0.0; 3]; 2];
    for (k, row) in frame.rows.into_iter().enumerate() {
        // |a cos(t)+b sin(t)| <= |a|+|b|. Outward basic arithmetic
        // makes this an enclosure even after large-coordinate cancellation.
        let extent = sum(
            product(radius, row[0].abs())?,
            product(radius, row[1].abs())?,
        )?[1];
        extrema[0][k] = (center[0][k] - extent).next_down();
        extrema[1][k] = (center[1][k] + extent).next_up();
    }
    bounds.include(extrema)?;
    bounds.finish()
}
fn placed(bounds: [[f64; 3]; 2], frame: Placement3) -> Result<[[f64; 3]; 2], BoundsError> {
    let mut result = Bounds::new();
    for corner in 0..8 {
        result.include(transformed(
            frame,
            core::array::from_fn(|k| bounds[usize::from(corner & (1 << k) != 0)][k]),
        )?)?;
    }
    result.finish()
}

impl Curve {
    /// Conservative enclosure of a bounded curve in its coordinate system.
    ///
    /// Lines use their range endpoints. Circular arcs use the entire supporting
    /// circle, so short arcs may have deliberately loose bounds. No sampling or
    /// display-tolerance dependence is introduced.
    /// # Errors
    /// Returns [`BoundsError::NumericLimit`] on unrepresentable positions/extents.
    pub fn conservative_bounds(&self, range: ParameterRange) -> Result<[[f64; 3]; 2], BoundsError> {
        match self {
            Self::Circle(c) => circle(c.frame(), c.radius(), 0.0),
            Self::Line(line) => {
                let mut bounds = Bounds::new();
                for t in range.bounds() {
                    let intervals = core::array::from_fn::<_, 3, _>(|k| {
                        linear(line.origin()[k], line.direction()[k], t)
                    });
                    let mut endpoints = [[0.0; 3]; 2];
                    for (k, value) in intervals.into_iter().enumerate() {
                        let value = value?;
                        endpoints[0][k] = value[0];
                        endpoints[1][k] = value[1];
                    }
                    bounds.include(endpoints)?;
                }
                bounds.finish()
            }
        }
    }
}
impl Surface {
    /// Finite enclosure of a supporting patch's rectangular parameter domain.
    ///
    /// Bounds include trims because they enclose the entire support domain;
    /// they need not be tight around a trimmed face. Cylinders/cones include
    /// full angular orbits, tori their whole support, and ruled patches the
    /// convex hull of their source-curve enclosures. A small outward numeric
    /// allowance is included. This query never creates a display mesh.
    /// # Errors
    /// Returns [`BoundsError::NumericLimit`] for unrepresentable support bounds.
    pub fn conservative_bounds(
        &self,
        domain: ParameterDomain,
    ) -> Result<[[f64; 3]; 2], BoundsError> {
        let mut bounds = Bounds::new();
        match self {
            Self::Plane(_) => {
                for u in domain.u.bounds() {
                    for v in domain.v.bounds() {
                        bounds.include(transformed(self.frame(), [u, v, 0.0])?)?;
                    }
                }
            }
            Self::Cylinder(c) => {
                for z in domain.v.bounds() {
                    bounds.include(circle(c.frame(), c.radius(), z)?)?;
                }
            }
            Self::Cone(c) => {
                for z in domain.v.bounds() {
                    c.radius_at(z).ok_or_else(numeric)?;
                    let radius = linear(c.radius(), c.slope(), z)?;
                    bounds.include(circle(c.frame(), radius[0].abs().max(radius[1].abs()), z)?)?;
                }
            }
            Self::Torus(t) => {
                let frame = t.frame();
                let center = frame.rows.map(|r| r[3]);
                let mut extent = [0.0; 3];
                for (k, row) in frame.rows.into_iter().enumerate() {
                    let major = sum(
                        product(t.major_radius(), row[0].abs())?,
                        product(t.major_radius(), row[1].abs())?,
                    )?;
                    let minor = sum(
                        sum(
                            product(t.minor_radius(), row[0].abs())?,
                            product(t.minor_radius(), row[1].abs())?,
                        )?,
                        product(t.minor_radius(), row[2].abs())?,
                    )?;
                    extent[k] = sum(major, minor)?[1];
                }
                bounds.point(core::array::from_fn(|k| center[k] - extent[k]))?;
                bounds.point(core::array::from_fn(|k| center[k] + extent[k]))?;
            }
            Self::Ruled(r) => {
                for span in r.curves() {
                    bounds.include(span.curve().conservative_bounds(span.range())?)?;
                }
            }
        }
        bounds.finish()
    }
}
impl<F, E> Body<F, E> {
    /// Conservative native support bounds after a finite affine placement.
    ///
    /// The source enclosure's eight corners are transformed with outward-rounded
    /// products and sums, including cancellation near large occurrence datums.
    /// # Errors
    /// Returns [`BoundsError`] when source or placed bounds cannot be represented.
    pub fn conservative_bounds_in(
        &self,
        placement: Placement3,
    ) -> Result<[[f64; 3]; 2], BoundsError> {
        placed(self.conservative_bounds()?, placement)
    }
    /// Conservative native bounds enclosing every supporting face domain.
    ///
    /// This is a source-space support enclosure, independent of display quality.
    /// It may include space outside trims and partial angular faces. It is not
    /// a minimum bounding box or a substitute for material classification.
    /// # Errors
    /// Returns [`BoundsError`] for an empty body or an unrepresentable face bound.
    pub fn conservative_bounds(&self) -> Result<[[f64; 3]; 2], BoundsError> {
        if self.faces().is_empty() {
            return Err(BoundsError::EmptyBody);
        }
        let mut bounds = Bounds::new();
        for (i, face) in self.faces().iter().enumerate() {
            bounds.include(face.surface.conservative_bounds(face.domain).map_err(|_| {
                BoundsError::NumericLimit {
                    face: Some(FaceId(u32::try_from(i).expect("body IDs fit u32"))),
                }
            })?)?;
        }
        bounds.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::CurveSpan;
    fn domain() -> ParameterDomain {
        ParameterDomain {
            u: ParameterRange::new(0.0, core::f64::consts::TAU).unwrap(),
            v: ParameterRange::new(-2.0, 3.0).unwrap(),
        }
    }
    #[test]
    fn curved_support_enclosures_cover_dense_evaluations_after_rotation() {
        let frame = Placement3::rotate_x_then_translate(0.7, 4.0, -8.0, 2.0);
        for surface in [
            Surface::cylinder(frame, 5.0).unwrap(),
            Surface::cone(frame, 5.0, 0.3).unwrap(),
            Surface::torus(frame, 8.0, 2.0).unwrap(),
        ] {
            let d = domain();
            let bounds = surface.conservative_bounds(d).unwrap();
            for i in 0..=80 {
                for j in 0..=30 {
                    let point = surface
                        .evaluate([
                            d.u.at(f64::from(i) / 80.0).unwrap(),
                            d.v.at(f64::from(j) / 30.0).unwrap(),
                        ])
                        .unwrap()
                        .point;
                    assert!(
                        point
                            .into_iter()
                            .enumerate()
                            .all(|(k, v)| v >= bounds[0][k] && v <= bounds[1][k]),
                        "{point:?} outside {bounds:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn cancellation_is_bounded_by_input_arithmetic_not_output_magnitude() {
        let range = ParameterRange::new(1.0, 1.0 + f64::EPSILON).unwrap();
        let curve = Curve::line([-1e16, 0.0, 0.0], [1e16, 0.0, 0.0]).unwrap();
        let bounds = curve.conservative_bounds(range).unwrap();
        assert!(bounds[0][0] <= 0.0);
        assert!(bounds[1][0] >= 1e16 * f64::EPSILON);
        let frame = Placement3::from_axes(
            [1e16, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [-1e16, 0.0, 0.0],
        );
        let bounds = placed([[1.0, 0.0, 0.0], [1.0 + f64::EPSILON, 1.0, 1.0]], frame).unwrap();
        assert!(bounds[1][0] >= 1e16 * f64::EPSILON);
    }
    #[test]
    fn ruled_bound_encloses_intermediate_curved_sections() {
        let range = ParameterRange::new(0.0, core::f64::consts::PI).unwrap();
        let a = CurveSpan::new(
            Curve::circle(Placement3::IDENTITY, 2.0).unwrap(),
            range,
            false,
        )
        .unwrap();
        let b = CurveSpan::new(
            Curve::circle(Placement3::translate(3.0, 0.0, 8.0), 4.0).unwrap(),
            range,
            false,
        )
        .unwrap();
        let surface = Surface::ruled(a, b).unwrap();
        let d = ParameterDomain {
            u: ParameterRange::new(0.0, 1.0).unwrap(),
            v: ParameterRange::new(0.0, 1.0).unwrap(),
        };
        let bounds = surface.conservative_bounds(d).unwrap();
        for i in 0..=30 {
            for j in 0..=30 {
                let point = surface
                    .evaluate([f64::from(i) / 30.0, f64::from(j) / 30.0])
                    .unwrap()
                    .point;
                assert!(
                    point
                        .into_iter()
                        .enumerate()
                        .all(|(k, v)| v >= bounds[0][k] && v <= bounds[1][k])
                );
            }
        }
    }
}
