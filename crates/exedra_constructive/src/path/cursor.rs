// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Radius/tangent path authoring over the ordinary retained curve representation.

use super::{
    ArcGeometry, PathClosure, PathDiscretizeError, PathJoin, PathSegment3, add, cross, dot,
    same_tangent, scale, sub, unit, validate_path_structure, validate_planar_curves,
};
use crate::ir::Path3;
use alloc::vec::Vec;

/// Draws a spatial path while retaining its current endpoint and tangent.
///
/// Lines, circular arcs and cubics lower to ordinary [`Path3::Curves`]. No path
/// is sampled to find its next endpoint: arc authoring and later sampling share
/// the same analytic evaluation. The first failure is retained until [`Self::finish`].
///
/// ```
/// use exedra_constructive::path::PathCursor3;
/// let rail = PathCursor3::new([0.0; 3], [0.0, 1.0, 0.0])
///     .tangent_arc(40.0, core::f64::consts::PI, [0.0, 0.0, 1.0])
///     .tangent_arc(40.0, -core::f64::consts::FRAC_PI_2, [0.0, 0.0, 1.0])
///     .line_by([-40.0, 0.0, 0.0])
///     .finish([1.0, 0.0, 0.0])?;
/// # Ok::<(), exedra_constructive::path::PathDiscretizeError>(())
/// ```
#[derive(Clone, Debug)]
#[must_use = "finish the cursor to construct a retained path"]
pub struct PathCursor3 {
    start: [f64; 3],
    current: [f64; 3],
    tangent: [f64; 3],
    segments: Vec<PathSegment3>,
    closure: PathClosure,
    joins: PathJoin,
    origin: [f64; 2],
    discontinuity: Option<usize>,
    first_tangent: Option<[f64; 3]>,
    error: Option<PathDiscretizeError>,
}

impl PathCursor3 {
    /// Starts at `start` with the direction used by the first tangent arc.
    /// A first line/cubic establishes its own direction. Tangents need not be unit.
    pub fn new(start: [f64; 3], tangent: [f64; 3]) -> Self {
        let error = if start.iter().any(|x| !x.is_finite()) {
            Some(PathDiscretizeError::InvalidPath)
        } else if unit(tangent).is_none() {
            Some(PathDiscretizeError::StationaryTangent {
                segment: 0,
                parameter: 0.0,
            })
        } else {
            None
        };
        Self {
            start,
            current: start,
            tangent: unit(tangent).unwrap_or([0.0; 3]),
            segments: Vec::new(),
            closure: PathClosure::Open,
            joins: PathJoin::Smooth,
            origin: [0.0; 2],
            discontinuity: None,
            first_tangent: None,
            error,
        }
    }

    /// Current exact authored/evaluated endpoint. After a failed step it stays put.
    #[must_use]
    pub const fn current(&self) -> [f64; 3] {
        self.current
    }

    /// Current unit endpoint tangent. Inspect failures through [`Self::finish`].
    #[must_use]
    pub const fn tangent(&self) -> [f64; 3] {
        self.tangent
    }

    /// Appends a straight segment, establishing its direction for later arcs.
    pub fn line_to(mut self, to: [f64; 3]) -> Self {
        if self.error.is_some() {
            return self;
        }
        let segment = self.segments.len();
        let Some(direction) =
            unit(sub(to, self.current)).filter(|_| to.iter().all(|x| x.is_finite()))
        else {
            return self.fail(PathDiscretizeError::InvalidSegment { segment });
        };
        self.append(PathSegment3::Line { to }, to, direction, direction);
        self
    }

    /// Appends a line relative to the current endpoint.
    pub fn line_by(self, offset: [f64; 3]) -> Self {
        let to = add(self.current, offset);
        self.line_to(to)
    }

    /// Continues tangentially along a circle of `radius` in the `normal` plane.
    ///
    /// `radius` must be positive and finite; the signed right-handed sweep is
    /// in radians, nonzero and at most one turn. `normal` must be perpendicular
    /// to the current tangent within the existing 1e-10 direction tolerance.
    /// Its magnitude does not affect the arc. A negative sweep turns the other way.
    /// Derived axes/endpoints use f64; evaluation bounds apply to those retained curves.
    pub fn tangent_arc(mut self, radius: f64, sweep: f64, normal: [f64; 3]) -> Self {
        if self.error.is_some() {
            return self;
        }
        let segment = self.segments.len();
        let Some(normal) = unit(normal) else {
            return self.fail(PathDiscretizeError::InvalidSegment { segment });
        };
        if !radius.is_finite() || radius <= 0.0 || dot(normal, self.tangent).abs() > 1e-10 {
            return self.fail(PathDiscretizeError::InvalidSegment { segment });
        }
        let axis_origin = add(
            self.current,
            scale(cross(normal, self.tangent), radius * sweep.signum()),
        );
        let arc = match ArcGeometry::new(self.current, axis_origin, normal, sweep, segment) {
            Ok(arc) => arc,
            Err(error) => return self.fail(error),
        };
        if !same_tangent(self.tangent, arc.first_t) {
            return self.fail(PathDiscretizeError::InvalidSegment { segment });
        }
        let end = match arc.station(1.0) {
            Ok(end) => end,
            Err(error) => return self.fail(error),
        };
        if end.point.iter().any(|x| !x.is_finite()) {
            return self.fail(PathDiscretizeError::NumericLimit { segment });
        }
        self.append(
            PathSegment3::Arc {
                axis_origin,
                axis: normal,
                sweep,
            },
            end.point,
            arc.first_t,
            end.tangent,
        );
        self
    }

    /// Appends a cubic with exact controls/endpoints; endpoint handles determine tangents.
    /// A coincident endpoint handle is refused rather than choosing a higher derivative.
    pub fn cubic_to(mut self, control1: [f64; 3], control2: [f64; 3], to: [f64; 3]) -> Self {
        if self.error.is_some() {
            return self;
        }
        let segment = self.segments.len();
        if control1
            .iter()
            .chain(&control2)
            .chain(&to)
            .any(|x| !x.is_finite())
        {
            return self.fail(PathDiscretizeError::InvalidSegment { segment });
        }
        let Some(first) = unit(sub(control1, self.current)) else {
            return self.fail(PathDiscretizeError::StationaryTangent {
                segment,
                parameter: 0.0,
            });
        };
        let Some(last) = unit(sub(to, control2)) else {
            return self.fail(PathDiscretizeError::StationaryTangent {
                segment,
                parameter: 1.0,
            });
        };
        self.append(
            PathSegment3::Cubic {
                control1,
                control2,
                to,
            },
            to,
            first,
            last,
        );
        self
    }

    /// Selects the ordinary smooth or bounded miter join policy; defaults to smooth.
    pub fn joins(mut self, joins: PathJoin) -> Self {
        self.joins = joins;
        self
    }

    /// Selects explicit endpoint connectivity; defaults to open. No closing segment is added.
    pub fn closure(mut self, closure: PathClosure) -> Self {
        self.closure = closure;
        self
    }

    /// Sets the profile-space datum placed on the path, without recentering.
    pub fn section_origin(mut self, origin: [f64; 2]) -> Self {
        self.origin = origin;
        self
    }

    /// Finishes an ordinary retained path with authored initial section-X direction.
    /// Sampling/work limits and swept-surface checks still belong to evaluation.
    ///
    /// # Errors
    ///
    /// Returns the first step failure, invalid join/connectivity/plane, or an
    /// authored tangent discontinuity under the smooth join policy. Section
    /// frame validity is checked by the sweep operation, as for a literal `Path3`.
    pub fn finish(self, section_x: [f64; 3]) -> Result<Path3, PathDiscretizeError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        validate_path_structure(self.start, &self.segments)?;
        if let PathJoin::Miter { limit } = self.joins {
            if !limit.is_finite() || limit < 1.0 {
                return Err(PathDiscretizeError::InvalidJoin);
            }
        } else if let Some(segment) = self.discontinuity {
            return Err(PathDiscretizeError::DiscontinuousTangent { segment });
        }
        match self.closure {
            PathClosure::Open if self.current == self.start => {
                return Err(PathDiscretizeError::InvalidPath);
            }
            PathClosure::ClosedPlanar { normal } => {
                if self.current != self.start {
                    return Err(PathDiscretizeError::NotClosed {
                        start: self.start,
                        end: self.current,
                    });
                }
                validate_planar_curves(self.start, &self.segments, normal)?;
                if self.joins == PathJoin::Smooth
                    && !same_tangent(self.tangent, self.first_tangent.expect("nonempty path"))
                {
                    return Err(PathDiscretizeError::DiscontinuousTangent { segment: 0 });
                }
            }
            PathClosure::Open => {}
        }
        Ok(Path3::Curves {
            start: self.start,
            segments: self.segments,
            section_x,
            section_origin: self.origin,
            closure: self.closure,
            joins: self.joins,
        })
    }

    fn append(&mut self, segment: PathSegment3, to: [f64; 3], first: [f64; 3], last: [f64; 3]) {
        if !self.segments.is_empty() && !same_tangent(self.tangent, first) {
            self.discontinuity.get_or_insert(self.segments.len());
        }
        self.first_tangent.get_or_insert(first);
        self.segments.push(segment);
        self.current = to;
        self.tangent = last;
    }

    fn fail(mut self, error: PathDiscretizeError) -> Self {
        self.error.get_or_insert(error);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::{PathDiscretizePolicy, discretize_path};
    use core::f64::consts::{FRAC_PI_2, PI, TAU};

    #[test]
    fn radius_arcs_and_sampling_use_identical_endpoints_and_tangents() {
        for size in [1e-100, 1.0, 1e100] {
            for sweep in [FRAC_PI_2, -FRAC_PI_2, PI, -PI] {
                let cursor = PathCursor3::new([size, 0.0, 0.0], [0.0, 1.0, 0.0]).tangent_arc(
                    2.0 * size,
                    sweep,
                    [0.0, 0.0, 7.0],
                );
                let end = cursor.current();
                let tangent = cursor.tangent();
                let Path3::Curves {
                    start,
                    segments,
                    closure,
                    joins,
                    ..
                } = cursor.finish([1.0, 0.0, 0.0]).unwrap()
                else {
                    panic!("curve path");
                };
                let sampled = discretize_path(
                    start,
                    &segments,
                    closure,
                    joins,
                    &PathDiscretizePolicy {
                        chord_tolerance: 0.01 * size,
                        ..PathDiscretizePolicy::default()
                    },
                )
                .unwrap();
                let last = sampled.stations.last().unwrap();
                assert_eq!(last.point, end);
                assert_eq!(last.tangent, tangent);
            }
        }
    }

    #[test]
    fn chained_arcs_do_not_need_axes_or_repeated_endpoints() {
        let cursor = PathCursor3::new([0.0; 3], [0.0, 1.0, 0.0])
            .tangent_arc(40.0, PI, [0.0, 0.0, 1.0])
            .tangent_arc(40.0, -FRAC_PI_2, [0.0, 0.0, 1.0])
            .line_by([-40.0, 0.0, 0.0]);
        assert!(
            cursor
                .current()
                .into_iter()
                .zip([-160.0, -40.0, 0.0])
                .all(|(a, b)| (a - b).abs() < 1e-12)
        );
        cursor.finish([1.0, 0.0, 0.0]).unwrap();
        let cursor = PathCursor3::new([0.0; 3], [1.0, 0.0, 0.0])
            .cubic_to([1.0, 0.0, 0.0], [2.0, 1.0, 0.0], [3.0, 1.0, 0.0])
            .tangent_arc(2.0, FRAC_PI_2, [0.0, 0.0, 1.0]);
        assert!(
            cursor
                .current()
                .into_iter()
                .zip([5.0, 3.0, 0.0])
                .all(|(a, b)| (a - b).abs() < 1e-14)
        );
    }

    #[test]
    fn full_turns_close_exactly_and_smooth_seams_are_checked() {
        let rail = PathCursor3::new([3.0, 0.0, 0.0], [0.0, 1.0, 0.0])
            .tangent_arc(3.0, TAU, [0.0, 0.0, 1.0])
            .closure(PathClosure::ClosedPlanar {
                normal: [0.0, 0.0, 1.0],
            })
            .finish([0.0, 0.0, 1.0])
            .unwrap();
        let Path3::Curves {
            start,
            segments,
            closure,
            joins,
            ..
        } = rail
        else {
            panic!("curve path");
        };
        let sampled = discretize_path(
            start,
            &segments,
            closure,
            joins,
            &PathDiscretizePolicy::default(),
        )
        .unwrap();
        assert_eq!(
            sampled.stations.first().unwrap().point,
            sampled.stations.last().unwrap().point
        );
        let bad_seam = PathCursor3::new([0.0; 3], [1.0, 0.0, 0.0])
            .cubic_to([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0; 3])
            .closure(PathClosure::ClosedPlanar {
                normal: [0.0, 0.0, 1.0],
            })
            .finish([0.0, 0.0, 1.0]);
        assert_eq!(
            bad_seam,
            Err(PathDiscretizeError::DiscontinuousTangent { segment: 0 })
        );
    }

    #[test]
    fn invalid_steps_keep_their_original_segment_and_corner_policy_is_explicit() {
        let failed = PathCursor3::new([0.0; 3], [1.0, 0.0, 0.0])
            .line_by([1.0, 0.0, 0.0])
            .tangent_arc(-2.0, 1.0, [0.0, 0.0, 1.0])
            .line_by([1.0, 0.0, 0.0]);
        assert_eq!(failed.current(), [1.0, 0.0, 0.0]);
        assert_eq!(
            failed.finish([0.0, 1.0, 0.0]),
            Err(PathDiscretizeError::InvalidSegment { segment: 1 })
        );
        let corner = || {
            PathCursor3::new([0.0; 3], [1.0, 0.0, 0.0])
                .line_by([1.0, 0.0, 0.0])
                .line_by([0.0, 1.0, 0.0])
        };
        assert_eq!(
            corner().finish([0.0, 0.0, 1.0]),
            Err(PathDiscretizeError::DiscontinuousTangent { segment: 1 })
        );
        corner()
            .joins(PathJoin::Miter { limit: 2.0 })
            .finish([0.0, 0.0, 1.0])
            .unwrap();
        assert!(
            PathCursor3::new([0.0; 3], [1.0, 0.0, 0.0])
                .tangent_arc(1.0, 1.0, [1.0, 0.0, 0.0])
                .finish([0.0, 1.0, 0.0])
                .is_err()
        );
    }
}
