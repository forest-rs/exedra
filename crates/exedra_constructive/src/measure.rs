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
//!     builders::rect_from_corner,
//!     ir::{CapMode, Placement3, Plane3},
//!     section::{SectionPolicy, section_body},
//!     tessellate::{EvalPolicy, tessellate_extrude},
//! };
//! let body = tessellate_extrude(&rect_from_corner(4.0, 3.0)?, &Placement3::IDENTITY,
//!     2.0, CapMode::Both, &EvalPolicy::default())?;
//! let section = section_body(&body,
//!     Plane3 { normal: [0.0, 0.0, 1.0], distance: 1.0 },
//!     &SectionPolicy::default())?;
//! let measured = section.measure()?;
//! assert_eq!(measured.area, 12.0);
//! assert_eq!(measured.perimeter, 14.0);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub use exedra_mesh_ops::measure::{MeasurementError, PlanarBounds, PlanarMeasurements};

pub(crate) fn measure<'a>(
    loops: impl IntoIterator<Item = (&'a [[f64; 2]], bool)>,
) -> Result<PlanarMeasurements, MeasurementError> {
    exedra_mesh_ops::measure::measure(
        loops
            .into_iter()
            .map(|(points, is_hole)| exedra_mesh_ops::measure::PlanarBoundary { points, is_hole }),
    )
}

#[cfg(test)]
mod tests;
