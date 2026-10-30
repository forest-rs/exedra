// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact construction distances, delivered separately from texture policy.

use super::{EdgeRole, FaceRole, Sweep};
use crate::{
    End,
    body::FaceId,
    realize::{RealizationError, RealizationPolicy, RealizedBody, VertexSource},
};
use alloc::vec::Vec;
use exedra_math::predicates::{MAX_COORDINATE, Orientation, orient2d};
use exedra_profile::{Arc, LoopRef, SegmentRef};

/// A native sweep could not deliver valid construction coordinates or mapped UVs.
#[derive(Clone, Debug, PartialEq)]
pub enum SweepUvError<E> {
    /// Native realization was refused.
    Realization(RealizationError),
    /// Profile/path distances overflow or fail to advance.
    Metric {
        /// Profile segment, or `None` for the path metric.
        segment: Option<SegmentRef>,
    },
    /// A native face's construction coordinates fold or cannot be represented.
    Coordinates {
        /// Offending face.
        face: FaceId,
    },
    /// Caller texture mapping failed.
    Mapping {
        /// Offending face.
        face: FaceId,
        /// Original caller error.
        error: E,
    },
    /// Caller mapping returned non-finite f32 coordinates.
    NonFiniteUv {
        /// Offending face.
        face: FaceId,
    },
}
impl<E: core::fmt::Display> core::fmt::Display for SweepUvError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Realization(e) => write!(f, "sweep realization: {e}"),
            Self::Metric { segment } => {
                write!(f, "sweep construction metric cannot preserve {segment:?}")
            }
            Self::Coordinates { face } => {
                write!(f, "sweep construction coordinates fold on {face:?}")
            }
            Self::Mapping { face, error } => {
                write!(f, "sweep texture mapping on {face:?}: {error}")
            }
            Self::NonFiniteUv { face } => {
                write!(f, "sweep mapping on {face:?} returned non-finite UVs")
            }
        }
    }
}
impl<E: core::error::Error + 'static> core::error::Error for SweepUvError<E> {}

struct Metric {
    from: [f64; 2],
    to: [f64; 2],
    arc: Option<Arc>,
    start: f64,
    end: f64,
    next: usize,
}
#[derive(Copy, Clone)]
struct RestPoint {
    profile: usize,
    profile_fraction: f64,
    station: usize,
    path_fraction: f64,
}

impl Sweep {
    /// Realizes with checked construction coordinates and caller texture policy.
    ///
    /// Walls receive `(profile_distance, centerline_distance)`, measured exactly
    /// on the source curves rather than on sampled chords. Each profile loop
    /// begins at its authored seam. Caps receive their original profile `(x,y)`.
    /// These rest coordinates survive occurrence placement and intentionally
    /// stretch on inner/outer bend rails. No geometry vertices are duplicated
    /// for the profile's texture seam, and uncharted realization has no UV scratch.
    ///
    /// # Errors
    /// Returns native realization, metric/coordinate or caller mapping failures.
    /// No partially mapped mesh is returned. Non-finite mapped UVs are refused.
    pub fn realize_with_uvs<E>(
        &self,
        policy: &RealizationPolicy,
        region: impl Fn(FaceId, &crate::body::Face<FaceRole>) -> u32,
        mut map: impl FnMut(FaceRole, [[f64; 2]; 3]) -> Result<[[f32; 2]; 3], E>,
    ) -> Result<RealizedBody, SweepUvError<E>> {
        let mut result = self
            .realize_with_regions(policy, region)
            .map_err(SweepUvError::Realization)?;
        let mut metrics = Vec::new();
        let mut offsets = Vec::new();
        for (loop_ref, ring) in core::iter::once((LoopRef::Outer, self.profile.outer())).chain(
            self.profile
                .holes()
                .iter()
                .enumerate()
                .map(|(i, r)| (LoopRef::Hole(i), r)),
        ) {
            let base = metrics.len();
            offsets.push(base);
            let mut total = 0.0;
            for (i, (from, segment)) in ring.iter_with_starts().enumerate() {
                let arc = ring.arc(i);
                let length = arc.map_or_else(
                    || libm::hypot(segment.to[0] - from[0], segment.to[1] - from[1]),
                    |a| a.length(),
                );
                let end = total + length;
                if !end.is_finite() || end <= total {
                    return Err(SweepUvError::Metric {
                        segment: Some(SegmentRef {
                            loop_ref,
                            segment: i,
                        }),
                    });
                }
                metrics.push(Metric {
                    from,
                    to: segment.to,
                    arc,
                    start: total,
                    end,
                    next: base + (i + 1) % ring.segments().len(),
                });
                total = end;
            }
        }
        let stations = &self.path_distances;
        if stations.windows(2).any(|p| p[1] <= p[0]) {
            return Err(SweepUvError::Metric { segment: None });
        }
        let index = |source: SegmentRef| {
            offsets[match source.loop_ref {
                LoopRef::Outer => 0,
                LoopRef::Hole(i) => i + 1,
            }] + source.segment
        };
        let count = metrics.len();
        let rest: Vec<_> = result
            .mesh_vertices
            .iter()
            .map(|(_, source)| {
                Ok(match *source {
                    VertexSource::Face { face, uv } => {
                        let record = &self.body.faces()[face.index()];
                        let FaceRole::Wall {
                            path_segment,
                            segment,
                            ..
                        } = record.role
                        else {
                            return Err(SweepUvError::Coordinates { face });
                        };
                        let ring = match segment.loop_ref {
                            LoopRef::Outer => self.profile.outer(),
                            LoopRef::Hole(i) => &self.profile.holes()[i],
                        };
                        let arc = ring
                            .arc(segment.segment)
                            .ok_or(SweepUvError::Coordinates { face })?;
                        let start = record.domain.v.bounds()[usize::from(arc.sweep > 0.0)];
                        let profile_fraction = (uv[1] - start) / (-arc.sweep);
                        let path_fraction =
                            (uv[0] - record.domain.u.bounds()[0]) / record.domain.u.span();
                        if !(0.0..=1.0).contains(&profile_fraction)
                            || !(0.0..=1.0).contains(&path_fraction)
                        {
                            return Err(SweepUvError::Coordinates { face });
                        }
                        RestPoint {
                            profile: index(segment),
                            profile_fraction,
                            station: path_segment,
                            path_fraction,
                        }
                    }
                    VertexSource::Vertex(id) => RestPoint {
                        profile: id.index() % count,
                        profile_fraction: 0.0,
                        station: id.index() / count,
                        path_fraction: 0.0,
                    },
                    VertexSource::Edge { edge, parameter } => {
                        match self.body.edges()[edge.index()].role {
                            EdgeRole::Section { station, segment } => RestPoint {
                                profile: index(segment),
                                profile_fraction: parameter,
                                station,
                                path_fraction: 0.0,
                            },
                            EdgeRole::Rail {
                                path_segment,
                                vertex,
                            } => RestPoint {
                                profile: index(vertex),
                                profile_fraction: 0.0,
                                station: path_segment,
                                path_fraction: parameter,
                            },
                        }
                    }
                })
            })
            .collect::<Result<_, _>>()?;
        let mut mapped = Vec::with_capacity(result.mesh.half_edge_count());
        for &(mesh_face, face) in &result.mesh_faces {
            let role = self.body.faces()[face.index()].role;
            let mut corners = result.mesh.face_loop(mesh_face);
            let corners =
                core::array::from_fn::<_, 3, _>(|_| corners.next().expect("triangle realization"));
            let mut coords = [[0.0; 2]; 3];
            for (i, corner) in corners.iter().enumerate() {
                let point =
                    rest[result.mesh.to_vertex(*corner).expect("live corner").index() as usize];
                coords[i] = match role {
                    FaceRole::Wall {
                        path_segment,
                        segment,
                        ..
                    } => {
                        let segment = index(segment);
                        let metric = &metrics[segment];
                        let u = if point.profile == segment {
                            metric.start + (metric.end - metric.start) * point.profile_fraction
                        } else if point.profile == metric.next && point.profile_fraction == 0.0 {
                            metric.end
                        } else {
                            return Err(SweepUvError::Coordinates { face });
                        };
                        let v = if point.station == path_segment {
                            stations[path_segment]
                                + (stations[path_segment + 1] - stations[path_segment])
                                    * point.path_fraction
                        } else if point.station == path_segment + 1 && point.path_fraction == 0.0 {
                            stations[path_segment + 1]
                        } else {
                            return Err(SweepUvError::Coordinates { face });
                        };
                        [u, v]
                    }
                    FaceRole::Cap { end } => {
                        let station = if end == End::Start {
                            0
                        } else {
                            self.path_segments.len()
                        };
                        if point.station != station || point.path_fraction != 0.0 {
                            return Err(SweepUvError::Coordinates { face });
                        }
                        let metric = &metrics[point.profile];
                        metric.arc.map_or_else(
                            || {
                                core::array::from_fn(|j| {
                                    metric.from[j]
                                        + point.profile_fraction * (metric.to[j] - metric.from[j])
                                })
                            },
                            |a| a.point_at(point.profile_fraction),
                        )
                    }
                };
            }
            let expected = if matches!(role, FaceRole::Cap { end: End::Start }) {
                Orientation::Cw
            } else {
                Orientation::Ccw
            };
            if coords
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
                || orient2d(coords[0], coords[1], coords[2]) != expected
            {
                return Err(SweepUvError::Coordinates { face });
            }
            let uv = map(role, coords).map_err(|error| SweepUvError::Mapping { face, error })?;
            if uv.iter().flatten().any(|v| !v.is_finite()) {
                return Err(SweepUvError::NonFiniteUv { face });
            }
            mapped.extend(corners.into_iter().zip(uv));
        }
        if mapped.windows(2).any(|p| p[0].0.index() >= p[1].0.index()) {
            mapped.sort_unstable_by_key(|p| p.0.index());
        }
        let mut edit = result.mesh.edit();
        exedra_mesh::op::set_corner_uvs(
            &mut edit,
            exedra_mesh::uv_mapping::UvSet::Primary,
            &mapped,
        )
        .expect("fresh unique triangle corners");
        let _: () = edit.finish();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sweep::tests::{profile, rail, view};
    use exedra_math::Placement3;
    #[test]
    fn charted_rail_keeps_geometry_and_source_distance_while_mapping_once_per_face() {
        let rail = rail();
        let solid = Sweep::new(profile(), &view(&rail), Placement3::IDENTITY).unwrap();
        let policy = RealizationPolicy {
            tolerance: 0.02,
            ..RealizationPolicy::default()
        };
        let plain = solid.realize(&policy).unwrap();
        let mut calls = 0;
        let mapped = solid
            .realize_with_uvs(
                &policy,
                |id, _| id.0,
                |_, p| -> Result<_, core::convert::Infallible> {
                    calls += 1;
                    Ok(p.map(|p| {
                        let p = exedra_math::narrow([p[0], p[1], 0.0]);
                        [p[0], p[1]]
                    }))
                },
            )
            .unwrap();
        assert_eq!(calls, mapped.mesh_faces.len());
        assert_eq!(plain.mesh.faces().count(), mapped.mesh.faces().count());
        assert_eq!(
            plain
                .mesh
                .vertices()
                .map(|v| plain.mesh.vertex_position(v).unwrap())
                .collect::<Vec<_>>(),
            mapped
                .mesh
                .vertices()
                .map(|v| mapped.mesh.vertex_position(v).unwrap())
                .collect::<Vec<_>>()
        );
        assert!(matches!(
            solid.realize_with_uvs(&policy, |_, _| 0, |_, _| -> Result<_, usize> { Err(17) }),
            Err(SweepUvError::Mapping { error: 17, .. })
        ));
        assert!(matches!(
            solid.realize_with_uvs(
                &policy,
                |_, _| 0,
                |_, _| -> Result<_, usize> { Ok([[f32::NAN; 2]; 3]) }
            ),
            Err(SweepUvError::NonFiniteUv { .. })
        ));
    }
}
