// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Owned and borrowed curve paths and moving-section descriptors shared by geometry heads.
//! Cursor authoring, sampling and topology remain owned by the consuming head.

use alloc::vec::Vec;

/// Authored endpoint connectivity of a controlled sweep path.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PathClosure {
    /// A spatial rail with distinct endpoints and optional caps.
    #[default]
    Open,
    /// A cyclic path in the plane through its first station.
    ///
    /// Polyline stations omit the repeated endpoint; curved segments explicitly
    /// terminate at the exact starting point. The closing run and corner
    /// are explicit; caps must be the uncapped sweep mode. Only f64 rounding-scale
    /// deviation from the plane is accepted, without projecting the points.
    ClosedPlanar {
        /// Finite nonzero plane normal in path-local coordinates.
        /// Its magnitude and sign do not change section orientation, which
        /// remains controlled by `section_x` and traversal direction.
        normal: [f64; 3],
    },
}

/// Join behavior at authored curve segment boundaries, including a closed seam.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PathJoin {
    /// Require analytic tangent continuity; no corner is inferred or repaired.
    #[default]
    Smooth,
    /// Join sections on the tangent-bisector plane at an authored corner.
    Miter {
        /// Maximum section-plane stretch, `1 / cos(turn / 2)`, finite and >= 1.
        /// Evaluation refuses larger corners instead of beveling them.
        limit: f64,
    },
}

/// One authored segment, starting at the preceding endpoint (or path start).
///
/// These three primitives form an exhaustive shared contract: adding another
/// requires coordinated validation, sampling and encoding support in consumers.
#[derive(Clone, Debug, PartialEq)]
pub enum PathSegment3 {
    /// Straight segment.
    Line {
        /// Endpoint in path-local coordinates.
        to: [f64; 3],
    },
    /// Circular arc obtained by rotating the start about an authored axis.
    /// The axis need not pass through the circle's center plane: its origin
    /// is any point on the rotation axis. No projection onto a world plane.
    Arc {
        /// Any point on the rotation axis, in path-local coordinates.
        axis_origin: [f64; 3],
        /// Nonzero finite axis direction; magnitude is irrelevant.
        axis: [f64; 3],
        /// Signed right-handed rotation in radians, `0 < abs(sweep) <= tau`.
        sweep: f64,
    },
    /// Spatial cubic Bezier segment.
    Cubic {
        /// First control point, in path-local coordinates.
        control1: [f64; 3],
        /// Second control point, in path-local coordinates.
        control2: [f64; 3],
        /// Endpoint in path-local coordinates.
        to: [f64; 3],
    },
}

/// Borrowed mathematical input for a constant-section curve sweep.
///
/// This view owns no allocation or recipe identity. Consumers validate their
/// supported path, frame and section contracts before constructing geometry.
#[derive(Copy, Clone, Debug)]
pub struct CurvePathView<'a> {
    /// First endpoint in path-local coordinates.
    pub start: [f64; 3],
    /// Ordered spans with implicit shared endpoints.
    pub segments: &'a [PathSegment3],
    /// Authored initial section-X direction, projected normal to the first tangent.
    pub section_x: [f64; 3],
    /// Profile-space datum carried along the path.
    pub section_origin: [f64; 2],
    /// Explicit endpoint connectivity.
    pub closure: PathClosure,
    /// Behavior at authored tangent discontinuities.
    pub joins: PathJoin,
}

/// An authored path that can expose line/arc/cubic spans without copying them.
pub trait CurvePathSource {
    /// Returns a view, or `None` for a representation without explicit curve spans.
    fn curve_path(&self) -> Option<CurvePathView<'_>>;
}

impl CurvePathSource for CurvePathView<'_> {
    fn curve_path(&self) -> Option<CurvePathView<'_>> {
        Some(*self)
    }
}

/// Frame policy for polyline sweeps.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum FramePolicy {
    /// Rotation-minimizing frame seeded from the first segment.
    #[default]
    RotationMinimizing,
}

/// A 3D sweep path.
#[derive(Clone, Debug, PartialEq)]
pub enum Path3 {
    /// A polyline path with a deterministic frame policy.
    Polyline {
        /// Path points, at least two, consecutive points distinct.
        points: Vec<[f64; 3]>,
        /// How profile frames orient along the path.
        frame: FramePolicy,
    },
    /// Constant-section straight runs joined on tangent-bisector cut planes.
    ///
    /// Section orientation is transported by the shortest rotation between
    /// segment directions. Local reversed spans fail; distant intersections
    /// are not checked. Consumers validate their own geometric support and budgets.
    MiteredPolyline {
        /// Finite stations: at least two for open paths, three for closed paths.
        /// Adjacent points differ; do not repeat the first station at the end.
        points: Vec<[f64; 3]>,
        /// Authored section-X direction in path-local coordinates. Projected
        /// perpendicular to the first segment; must not be nearly parallel.
        section_x: [f64; 3],
        /// Profile coordinates of the datum placed on the path. No recentering
        /// or inferred section offset is applied.
        section_origin: [f64; 2],
        /// Authored connectivity; closure is never inferred from coordinates.
        closure: PathClosure,
        /// Maximum section-plane stretch, `1 / cos(turn / 2)`, finite and >= 1.
        /// Exceeding it fails rather than changing the join style.
        miter_limit: f64,
    },
    /// Lines, circular arcs and spatial cubics with authored joins and closure.
    ///
    /// Initial section orientation is authored; analytic tangents guide
    /// rotation-minimizing transport. Sampling accuracy belongs to the consuming geometry head.
    /// Planar closure reconstructs the same roll around the loop; no final twist
    /// or endpoint snapping is applied. Sharp joins require `PathJoin::Miter`.
    Curves {
        /// First endpoint, in path-local coordinates.
        start: [f64; 3],
        /// Nonempty ordered segments, with implicit shared endpoints.
        segments: Vec<PathSegment3>,
        /// Initial section-X direction before placement, projected normal to
        /// the first analytic tangent. Must have a usable perpendicular part.
        section_x: [f64; 3],
        /// Profile-space datum placed on the path before framing and mitering.
        section_origin: [f64; 2],
        /// Authored endpoint connectivity; closed curves terminate exactly at `start`.
        closure: PathClosure,
        /// Behavior at authored segment boundaries and the closed seam.
        joins: PathJoin,
    },
}

impl CurvePathSource for Path3 {
    fn curve_path(&self) -> Option<CurvePathView<'_>> {
        match self {
            Self::Curves {
                start,
                segments,
                section_x,
                section_origin,
                closure,
                joins,
            } => Some(CurvePathView {
                start: *start,
                segments,
                section_x: *section_x,
                section_origin: *section_origin,
                closure: *closure,
                joins: *joins,
            }),
            _ => None,
        }
    }
}

impl Path3 {
    /// A counterclockwise circle of `radius` about the origin in the XY plane.
    ///
    /// The sweep starts at `(radius, 0, 0)`, travels with +Y tangent, and
    /// seeds section X along +Z. The path closes at one shared station and
    /// requires an uncapped sweep.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::Radius`] unless
    /// `radius` is finite and positive.
    pub fn xy_circle(radius: f64) -> Result<Self, PathError> {
        if !radius.is_finite() || radius <= 0.0 {
            return Err(PathError::Radius { radius });
        }
        Ok(Self::Curves {
            start: [radius, 0.0, 0.0],
            segments: alloc::vec![PathSegment3::Arc {
                axis_origin: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                sweep: core::f64::consts::TAU,
            }],
            section_x: [0.0, 0.0, 1.0],
            section_origin: [0.0; 2],
            closure: PathClosure::ClosedPlanar {
                normal: [0.0, 0.0, 1.0],
            },
            joins: PathJoin::Smooth,
        })
    }

    /// Authored polyline points before placement, or `None` for analytic curves.
    ///
    /// Curves retain authored spans instead of a preselected sampling of points.
    /// Sampling is an explicit operation in the consuming geometry head.
    #[must_use]
    pub fn polyline_points(&self) -> Option<&[[f64; 3]]> {
        match self {
            Self::Polyline { points, .. } | Self::MiteredPolyline { points, .. } => Some(points),
            Self::Curves { .. } => None,
        }
    }
}

/// Transports a section axis one station along a sampled path without spin:
/// one step of the double-reflection rotation-minimizing frame
/// ([`FramePolicy::RotationMinimizing`]).
///
/// `x` is the section axis at `from`, perpendicular to the unit `tangent`
/// there; the result is the axis at `to`, perpendicular to the unit
/// `next_tangent`. The chord `to - from` reflects the frame, and a second
/// reflection carries the reflected tangent onto `next_tangent`; the second
/// is skipped when they already agree (`|next_tangent - reflected|^2 <=
/// 1e-24`). Only additions, multiplications and divisions are used, in a
/// fixed order, so equal inputs give equal bits on every platform. When
/// `from == to` there is no chord, nothing moves, and `x` is returned.
///
/// Reference: W. Wang, B. Juttler, D. Zheng, Y. Liu, "Computation of Rotation
/// Minimizing Frames", ACM Transactions on Graphics 27(1), 2008, section 4
/// (the double reflection method).
#[must_use]
pub fn double_reflection(
    from: [f64; 3],
    x: [f64; 3],
    tangent: [f64; 3],
    to: [f64; 3],
    next_tangent: [f64; 3],
) -> [f64; 3] {
    let v1 = crate::sub(to, from);
    let c1 = crate::dot(v1, v1);
    if c1 == 0.0 {
        return x;
    }
    let x_l = crate::sub(x, crate::scale(v1, 2.0 / c1 * crate::dot(v1, x)));
    let t_l = crate::sub(
        tangent,
        crate::scale(v1, 2.0 / c1 * crate::dot(v1, tangent)),
    );
    let v2 = crate::sub(next_tangent, t_l);
    let c2 = crate::dot(v2, v2);
    if c2 <= 1e-24 {
        x_l
    } else {
        crate::sub(x_l, crate::scale(v2, 2.0 / c2 * crate::dot(v2, x_l)))
    }
}

/// Invalid mathematical path construction, independent of sampling policy.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PathError {
    /// A circle radius must be finite and strictly positive.
    Radius {
        /// Supplied radius.
        radius: f64,
    },
}
impl core::fmt::Display for PathError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Radius { radius } => {
                write!(f, "path circle radius {radius} must be positive and finite")
            }
        }
    }
}
impl core::error::Error for PathError {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn circle_construction_and_borrowing_need_no_sampling_policy() {
        for radius in [0.0, -1.0, f64::INFINITY, f64::NAN] {
            assert!(matches!(
                Path3::xy_circle(radius),
                Err(PathError::Radius { .. })
            ));
        }
        let path = Path3::xy_circle(3.0).unwrap();
        let view = path.curve_path().unwrap();
        assert_eq!(view.start, [3.0, 0.0, 0.0]);
        assert_eq!(view.section_x, [0.0, 0.0, 1.0]);
        assert!(matches!(
            view.closure,
            PathClosure::ClosedPlanar {
                normal: [0.0, 0.0, 1.0]
            }
        ));
        let Path3::Curves { segments, .. } = &path else {
            unreachable!()
        };
        assert!(
            core::ptr::eq(view.segments, segments.as_slice()),
            "borrow the owned spans without a copy"
        );
        assert!(path.polyline_points().is_none());
    }

    #[test]
    fn double_reflection_carries_a_frame_around_a_planar_turn_without_spin() {
        // A quarter turn in the XY plane: the out-of-plane axis stays put.
        let x = double_reflection(
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        );
        assert_eq!(x, [0.0, 0.0, 1.0]);
        // The in-plane axis turns with the tangent.
        let y = double_reflection(
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        );
        assert!((y[0] + 1.0).abs() < 1e-15 && y[1].abs() < 1e-15, "{y:?}");
        // No chord: nothing moves.
        assert_eq!(
            double_reflection(
                [1.0; 3],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0; 3],
                [0.0, 0.0, 1.0]
            ),
            [0.0, 1.0, 0.0]
        );
    }
}
