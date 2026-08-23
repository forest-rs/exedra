// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Declared structural extents.
//!
//! This module is deliberately tiny. `joiner` owns no geometry math: cuts,
//! offsets, booleans, and tessellation belong to `exedra_constructive` and
//! `exedra`, and vector arithmetic to `exedra_math`. What lives here is the
//! *analytic proxy* every element declares for itself — an [`OrientedBox`]
//! extent — and the tolerance its frame is held to. Nothing here evaluates a
//! recipe or touches a mesh.

use exedra_constructive::ir::Placement3;
use exedra_math::{add, dot, finite, is_orthogonal_frame, is_unit, scale, sub};

/// A point or direction in world space, in metres.
pub type Vec3 = [f64; 3];

/// How far a declared frame may stray from unit length and orthogonality
/// before [`crate::validate()`] reports it. A published number, like
/// [`crate::CONTACT_TOLERANCE`]: authored axes are expected to be exact to
/// well within it.
pub const FRAME_TOLERANCE: f64 = 1.0e-9;

/// The declared structural extent of an element: an oriented box.
///
/// The box is a *claim*, not a tessellation. Contact validation measures
/// anchors, gaps, and overlaps against it, and lowering derives the default
/// part placement from its frame. An element's compiled geometry may be far
/// richer than its extent — cut, filleted, profiled — but the extent is what
/// the structural claims are made about, so it must stay an honest bound.
///
/// `axes` are expected to be a finite, unit-length orthogonal frame and `size`
/// strictly positive; [`crate::validate()`] reports both. Either handedness is
/// valid: reflected local frames are represented explicitly by the placement.
#[derive(Clone, Debug, PartialEq)]
pub struct OrientedBox {
    /// The local origin corner in world space.
    pub origin: Vec3,
    /// The three local axis directions, expected unit and mutually
    /// orthogonal.
    pub axes: [Vec3; 3],
    /// Extents along each local axis, expected strictly positive.
    pub size: Vec3,
}

impl OrientedBox {
    /// An axis-aligned box with its minimum corner at `origin`.
    #[must_use]
    pub fn axis_aligned(origin: Vec3, size: Vec3) -> Self {
        Self {
            origin,
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            size,
        }
    }

    /// The world-space point at local coordinates `local`.
    ///
    /// Local coordinates run from `[0, 0, 0]` at [`OrientedBox::origin`] to
    /// [`OrientedBox::size`] at the opposite corner, so anchors read as
    /// distances along the element's own axes.
    #[must_use]
    pub fn anchor(&self, local: Vec3) -> Vec3 {
        add(
            self.origin,
            add(
                scale(self.axes[0], local[0]),
                add(scale(self.axes[1], local[1]), scale(self.axes[2], local[2])),
            ),
        )
    }

    /// The world-space centre of the box.
    #[must_use]
    pub fn center(&self) -> Vec3 {
        self.anchor([self.size[0] * 0.5, self.size[1] * 0.5, self.size[2] * 0.5])
    }

    /// The placement that maps this box's local frame into world space.
    ///
    /// Lowering uses it for the element's instance placement, so part-local
    /// coordinates and extent-local coordinates are the same coordinates.
    #[must_use]
    pub fn placement(&self) -> Placement3 {
        Placement3::from_axes(self.axes[0], self.axes[1], self.axes[2], self.origin)
    }

    /// Whether a world point lies inside the box within `tolerance`.
    #[must_use]
    pub fn contains_point(&self, point: Vec3, tolerance: f64) -> bool {
        let delta = sub(point, self.origin);
        (0..3).all(|axis| {
            let coordinate = dot(delta, self.axes[axis]);
            coordinate >= -tolerance && coordinate <= self.size[axis] + tolerance
        })
    }

    /// Whether local coordinates lie inside the box within `tolerance`.
    #[must_use]
    pub fn contains_local(&self, local: Vec3, tolerance: f64) -> bool {
        finite(local)
            && (0..3)
                .all(|axis| local[axis] >= -tolerance && local[axis] <= self.size[axis] + tolerance)
    }

    /// The box translated by `delta`, leaving its frame and size alone.
    #[must_use]
    pub fn translated(&self, delta: Vec3) -> Self {
        Self {
            origin: add(self.origin, delta),
            axes: self.axes,
            size: self.size,
        }
    }

    /// Whether the extent is finite, positively sized, and orthonormal.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        finite(self.origin)
            && finite(self.size)
            && self.size.iter().all(|value| *value > 0.0)
            && self.axes.iter().all(|axis| is_unit(*axis, FRAME_TOLERANCE))
            && is_orthogonal_frame(self.axes, FRAME_TOLERANCE)
    }

    /// The projection of the box onto `axis`, as a `(minimum, maximum)`
    /// interval. Used to measure how far two elements overlap across a
    /// contact.
    pub(crate) fn projection_interval(&self, axis: Vec3) -> (f64, f64) {
        let center = dot(self.center(), axis);
        let radius = (0..3)
            .map(|i| self.size[i] * 0.5 * dot(self.axes[i], axis).abs())
            .sum::<f64>();
        (center - radius, center + radius)
    }
}

/// The length of the overlap between two closed intervals, never negative.
pub(crate) fn interval_overlap(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.1.min(b.1) - a.0.max(b.0)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_and_placement_agree_on_the_local_frame() {
        let extent = OrientedBox {
            origin: [1.0, 2.0, 3.0],
            axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            size: [4.0, 0.5, 0.25],
        };
        let local = [2.0, 0.25, 0.125];
        let placement = extent.placement();
        let mapped = [
            placement.rows[0][0] * local[0]
                + placement.rows[0][1] * local[1]
                + placement.rows[0][2] * local[2]
                + placement.rows[0][3],
            placement.rows[1][0] * local[0]
                + placement.rows[1][1] * local[1]
                + placement.rows[1][2] * local[2]
                + placement.rows[1][3],
            placement.rows[2][0] * local[0]
                + placement.rows[2][1] * local[1]
                + placement.rows[2][2] * local[2]
                + placement.rows[2][3],
        ];
        assert_eq!(mapped, extent.anchor(local), "placement must match anchor");
        assert!(extent.is_well_formed(), "fixture extent is orthonormal");
        assert!(
            extent.contains_point(extent.center(), 0.0),
            "the centre is inside"
        );
    }

    #[test]
    fn degenerate_extents_are_reported_not_panicked_on() {
        let zero = OrientedBox::axis_aligned([0.0; 3], [0.0, 1.0, 1.0]);
        assert!(!zero.is_well_formed(), "zero extent is malformed");
        let skew = OrientedBox {
            origin: [0.0; 3],
            axes: [[1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            size: [1.0, 1.0, 1.0],
        };
        assert!(!skew.is_well_formed(), "parallel axes are malformed");
        let reflected = OrientedBox {
            origin: [0.0; 3],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]],
            size: [1.0; 3],
        };
        assert!(
            reflected.is_well_formed(),
            "reflected local frames are explicit and supported"
        );
        let nan = OrientedBox {
            origin: [0.0; 3],
            axes: [[f64::NAN, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            size: [1.0; 3],
        };
        assert!(!nan.is_well_formed(), "non-finite axes are malformed");
    }

    #[test]
    fn overlap_is_never_negative() {
        assert_eq!(interval_overlap((0.0, 1.0), (2.0, 3.0)), 0.0);
        assert_eq!(interval_overlap((0.0, 2.0), (1.0, 3.0)), 1.0);
    }
}
