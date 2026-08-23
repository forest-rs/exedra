// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Declared structural extents and the small vector algebra the graph needs.
//!
//! This module is deliberately tiny. `joiner` owns no geometry math: cuts,
//! offsets, booleans, and tessellation belong to `exedra_constructive` and
//! `exedra`. What lives here is the *analytic proxy* every element declares
//! for itself — an [`OrientedBox`] extent — plus the dot/cross arithmetic
//! that contact validation measures with. Nothing here evaluates a recipe or
//! touches a mesh.

use exedra_constructive::ir::Placement3;

/// A point or direction in world space, in metres.
pub type Vec3 = [f64; 3];

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
            && self.axes.iter().copied().all(is_unit)
            && is_orthogonal_frame(self.axes)
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

/// Componentwise sum.
#[must_use]
pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Componentwise difference.
#[must_use]
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Uniform scaling.
#[must_use]
pub fn scale(a: Vec3, factor: f64) -> Vec3 {
    [a[0] * factor, a[1] * factor, a[2] * factor]
}

/// Dot product.
#[must_use]
pub fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Cross product.
#[must_use]
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Euclidean length, through `libm` so `no_std` builds stay identical.
#[must_use]
pub fn norm(a: Vec3) -> f64 {
    libm::sqrt(dot(a, a))
}

/// The unit vector along `a`, or `None` when `a` is degenerate or
/// non-finite.
#[must_use]
pub fn normalize(a: Vec3) -> Option<Vec3> {
    let length = norm(a);
    if !length.is_finite() || length <= 0.0 {
        return None;
    }
    let unit = scale(a, 1.0 / length);
    finite(unit).then_some(unit)
}

/// Whether every component is finite.
#[must_use]
pub fn finite(a: Vec3) -> bool {
    a.iter().all(|value| value.is_finite())
}

/// Whether `a` is finite and of unit length within `1e-9`.
#[must_use]
pub fn is_unit(a: Vec3) -> bool {
    finite(a) && (norm(a) - 1.0).abs() < 1.0e-9
}

/// Whether three axes are mutually orthogonal within `1e-9`.
#[must_use]
pub fn is_orthogonal_frame(axes: [Vec3; 3]) -> bool {
    dot(axes[0], axes[1]).abs() <= 1.0e-9
        && dot(axes[0], axes[2]).abs() <= 1.0e-9
        && dot(axes[1], axes[2]).abs() <= 1.0e-9
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
        assert_eq!(normalize([0.0; 3]), None, "degenerate direction");
        assert_eq!(
            normalize([f64::NAN, 0.0, 0.0]),
            None,
            "non-finite direction"
        );
        assert_eq!(normalize([0.0, 3.0, 0.0]), Some([0.0, 1.0, 0.0]));
        assert_eq!(cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn overlap_is_never_negative() {
        assert_eq!(interval_overlap((0.0, 1.0), (2.0, 3.0)), 0.0);
        assert_eq!(interval_overlap((0.0, 2.0), (1.0, 3.0)), 1.0);
    }
}
