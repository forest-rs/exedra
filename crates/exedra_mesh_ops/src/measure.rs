// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Polygon and triangle measurements independent of construction recipes.
//!
//! Lengths use the input coordinate units and areas use squared units. Holes
//! subtract area and first moments while adding boundary length. No analytic
//! curves or tessellation error bound are inferred.

mod volume;
pub use volume::{SignedVolume, VolumeError, signed_volume};

/// One oriented polygon boundary in a shared local XY frame.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlanarBoundary<'a> {
    /// Ordered vertices, without repeating the first point at the end.
    pub points: &'a [[f64; 2]],
    /// Whether this is a clockwise hole; outer boundaries are counterclockwise.
    pub is_hole: bool,
}

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

/// Measures oriented loops in a shared orthonormal XY frame.
///
/// Empty input has zero area/perimeter and no centroid/bounds. Outer loops must
/// wind counterclockwise; holes must wind clockwise. The caller establishes
/// simplicity, disjointness and nesting: this function checks local numeric and
/// winding conditions but does not certify the filled domain's topology.
/// Coordinates are accumulated in f64 with translation and compensated sums;
/// this is not an exact predicate or a certified arithmetic error bound.
pub fn measure<'a>(
    loops: impl IntoIterator<Item = PlanarBoundary<'a>>,
) -> Result<PlanarMeasurements, MeasurementError> {
    use MeasurementError::{InvalidBoundary, NumericLimit};
    let mut area2 = Sum::default();
    let mut moment = [Sum::default(), Sum::default()];
    let mut perimeter = Sum::default();
    let mut bounds: Option<PlanarBounds> = None;
    let mut origin = None;
    let mut edges_examined = 0;
    for PlanarBoundary {
        points,
        is_hole: hole,
    } in loops
    {
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
mod tests {
    use super::*;

    #[test]
    fn direct_boundaries_measure_an_off_center_hole() {
        let outer = [[0.0, 0.0], [6.0, 0.0], [6.0, 4.0], [0.0, 4.0]];
        let hole = [[1.0, 1.0], [1.0, 2.0], [3.0, 2.0], [3.0, 1.0]];
        let result = measure([
            PlanarBoundary {
                points: &outer,
                is_hole: false,
            },
            PlanarBoundary {
                points: &hole,
                is_hole: true,
            },
        ])
        .unwrap();
        assert_eq!(result.area, 22.0);
        assert_eq!(result.perimeter, 26.0);
        assert_eq!(result.edges_examined, 8);
        let centroid = result.centroid.unwrap();
        assert!((centroid[0] - 68.0 / 22.0).abs() < 1e-12);
        assert!((centroid[1] - 45.0 / 22.0).abs() < 1e-12);
        assert_eq!(
            measure([PlanarBoundary {
                points: &hole,
                is_hole: false
            }]),
            Err(MeasurementError::InvalidBoundary)
        );
    }
}
