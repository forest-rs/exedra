// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Measurements of evaluated polygonal planar domains.
//!
//! Use [`PlanarPatch::measure`](crate::clearance::PlanarPatch::measure) or
//! [`PlaneSection::measure`](crate::section::PlaneSection::measure). Results use
//! the owner's orthonormal local XY frame: lengths are in body units and area
//! is in squared body units. Holes subtract area and first moments, but add
//! boundary length. No analytic curves are reconstructed and no tessellation
//! error bound is inferred. Keep the owner for its frame and source evidence.
//! Computation uses f64 arithmetic with translated coordinates and compensated
//! sums, not exact predicates or a certified arithmetic error bound. Nearly
//! cancelling determinants or outer/hole areas can still lose relative accuracy.
//!
//! ```
//! use exedra_constructive::{
//!     builders::rect,
//!     ir::{CapMode, Placement3, Plane3},
//!     section::{SectionPolicy, section_body},
//!     tessellate::{EvalPolicy, tessellate_extrude},
//! };
//! let body = tessellate_extrude(&rect(4.0, 3.0)?, &Placement3::IDENTITY,
//!     2.0, CapMode::Both, &EvalPolicy::default())?;
//! let section = section_body(&body,
//!     Plane3 { normal: [0.0, 0.0, 1.0], distance: 1.0 },
//!     &SectionPolicy::default())?;
//! let measured = section.measure()?;
//! assert_eq!(measured.area, 12.0);
//! assert_eq!(measured.perimeter, 14.0);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

/// Axis-aligned bounds in the measured domain's local XY frame.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlanarBounds {
    /// Minimum coordinates on each axis.
    pub min: [f64; 2],
    /// Maximum coordinates on each axis.
    pub max: [f64; 2],
}

/// Measurements of a filled planar domain, including its holes.
///
/// The centroid is an area-weighted center; it need not lie in material (for
/// example, the centroid of an annulus lies in its hole). These measurements
/// do not certify simplicity, nesting, or absence of overlap.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlanarMeasurements {
    /// Material area: outer areas minus hole areas, in squared body units.
    pub area: f64,
    /// Total outer and hole boundary length, in body units.
    pub perimeter: f64,
    /// Area centroid in local XY, or `None` for an empty domain.
    pub centroid: Option<[f64; 2]>,
    /// Local XY bounds, or `None` for an empty domain.
    pub bounds: Option<PlanarBounds>,
    /// Number of boundary segments visited; no mesh traversal or tessellation.
    pub edges_examined: usize,
}

/// Refusal to report a planar measurement.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MeasurementError {
    /// A loop has fewer than three points, zero-length edges, zero area, or
    /// winding inconsistent with its outer/hole role; or net area is nonpositive.
    InvalidBoundary,
    /// Nonfinite coordinates/arithmetic, or a nonzero product that underflows
    /// or becomes subnormal. This is not a general precision-loss detector;
    /// finite division results can be subnormal.
    NumericLimit,
}
impl core::fmt::Display for MeasurementError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidBoundary => "invalid oriented boundary for planar measurement",
            Self::NumericLimit => "planar measurement exceeds numeric limits",
        })
    }
}
impl core::error::Error for MeasurementError {}

// Compensated summation keeps small holes and short edges from being lost when
// accumulated alongside much larger contributions. Translation below avoids
// taking cross products of large absolute coordinates for small distant shapes.
#[derive(Default)]
struct Sum {
    value: f64,
    correction: f64,
}
impl Sum {
    fn add(&mut self, value: f64) {
        let next = self.value + value;
        self.correction += if self.value.abs() >= value.abs() {
            (self.value - next) + value
        } else {
            (value - next) + self.value
        };
        self.value = next;
    }
    fn total(&self) -> f64 {
        self.value + self.correction
    }
}

// Refuse products that lose their magnitude entirely or enter subnormal
// arithmetic. In particular, a finite area must not hide underflowed moments.
fn product(a: f64, b: f64) -> Result<f64, MeasurementError> {
    let value = a * b;
    if !value.is_finite() || (a != 0.0 && b != 0.0 && (value == 0.0 || value.is_subnormal())) {
        Err(MeasurementError::NumericLimit)
    } else {
        Ok(value)
    }
}

pub(crate) fn measure<'a>(
    loops: impl IntoIterator<Item = (&'a [[f64; 2]], bool)>,
) -> Result<PlanarMeasurements, MeasurementError> {
    use MeasurementError::{InvalidBoundary, NumericLimit};
    let mut area2 = Sum::default();
    let mut moment = [Sum::default(), Sum::default()];
    let mut perimeter = Sum::default();
    let mut bounds: Option<PlanarBounds> = None;
    let mut origin = None;
    let mut edges_examined = 0;
    for (points, hole) in loops {
        if points.len() < 3 {
            return Err(InvalidBoundary);
        }
        let reference = *origin.get_or_insert(points[0]);
        let local_origin = points[0];
        let mut loop_area2 = Sum::default();
        let mut loop_moment = [Sum::default(), Sum::default()];
        for (i, &p) in points.iter().enumerate() {
            let q = points[(i + 1) % points.len()];
            if !p.iter().chain(q.iter()).all(|v| v.is_finite()) {
                return Err(NumericLimit);
            }
            let minmax = bounds.get_or_insert(PlanarBounds { min: p, max: p });
            for (axis, &v) in p.iter().enumerate() {
                minmax.min[axis] = minmax.min[axis].min(v);
                minmax.max[axis] = minmax.max[axis].max(v);
            }
            let length = libm::hypot(q[0] - p[0], q[1] - p[1]);
            if length == 0.0 {
                return Err(InvalidBoundary);
            }
            perimeter.add(length);
            let a = [p[0] - local_origin[0], p[1] - local_origin[1]];
            let b = [q[0] - local_origin[0], q[1] - local_origin[1]];
            let cross = product(a[0], b[1])? - product(b[0], a[1])?;
            loop_area2.add(cross);
            for axis in 0..2 {
                loop_moment[axis].add(product(a[axis] + b[axis], cross)?);
            }
            edges_examined += 1;
        }
        let signed = loop_area2.total();
        if !signed.is_finite() {
            return Err(NumericLimit);
        }
        if signed == 0.0 || (signed < 0.0) != hole {
            return Err(InvalidBoundary);
        }
        area2.add(signed);
        for axis in 0..2 {
            moment[axis].add(loop_moment[axis].total() / 3.0);
            moment[axis].add(product(local_origin[axis] - reference[axis], signed)?);
        }
    }
    let twice_area = area2.total();
    let area = product(twice_area, 0.5)?;
    let perimeter = perimeter.total();
    if !area.is_finite() || !perimeter.is_finite() {
        return Err(NumericLimit);
    }
    let centroid = if let Some(reference) = origin {
        if twice_area <= 0.0 {
            return Err(InvalidBoundary);
        }
        if area == 0.0 {
            return Err(NumericLimit);
        }
        let centroid =
            core::array::from_fn(|axis| reference[axis] + moment[axis].total() / twice_area);
        if !centroid.iter().all(|v| v.is_finite()) {
            return Err(NumericLimit);
        }
        Some(centroid)
    } else {
        None
    };
    Ok(PlanarMeasurements {
        area,
        perimeter,
        centroid,
        bounds,
        edges_examined,
    })
}

#[cfg(test)]
mod tests;
