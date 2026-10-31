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
    // One-based index leaves a niche for interior vertices' absent rest record
    // without enlarging the existing per-vertex boundary scratch allocation.
    profile: core::num::NonZeroUsize,
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
            .map(|(_, source)| match *source {
                VertexSource::Vertex(id) => Some(RestPoint {
                    profile: core::num::NonZeroUsize::new(id.index() % count + 1)
                        .expect("profile vertex"),
                    profile_fraction: 0.0,
                    station: id.index() / count,
                    path_fraction: 0.0,
                }),
                VertexSource::Edge { edge, parameter } => {
                    Some(match self.body.edges()[edge.index()].role {
                        EdgeRole::Section { station, segment } => RestPoint {
                            profile: core::num::NonZeroUsize::new(index(segment) + 1)
                                .expect("profile segment"),
                            profile_fraction: parameter,
                            station,
                            path_fraction: 0.0,
                        },
                        EdgeRole::Rail {
                            path_segment,
                            vertex,
                        } => RestPoint {
                            profile: core::num::NonZeroUsize::new(index(vertex) + 1)
                                .expect("profile vertex"),
                            profile_fraction: 0.0,
                            station: path_segment,
                            path_fraction: parameter,
                        },
                    })
                }
                // Supporting-surface interior coordinates are converted per face,
                // without inventing a source profile vertex or path station.
                VertexSource::Face { .. } => None,
            })
            .collect();
        let mut mapped = Vec::with_capacity(result.mesh.half_edge_count());
        for &(mesh_face, face) in &result.mesh_faces {
            let role = self.body.faces()[face.index()].role;
            let mut corners = result.mesh.face_loop(mesh_face);
            let corners =
                core::array::from_fn::<_, 3, _>(|_| corners.next().expect("triangle realization"));
            let mut coords = [[0.0; 2]; 3];
            for (i, corner) in corners.iter().enumerate() {
                let vertex = result.mesh.to_vertex(*corner).expect("live corner").index() as usize;
                if let VertexSource::Face {
                    face: source_face,
                    uv,
                } = result.mesh_vertices[vertex].1
                {
                    if source_face != face {
                        return Err(SweepUvError::Coordinates { face });
                    }
                    coords[i] = self
                        .face_coordinates(face, uv, &metrics, &offsets)
                        .ok_or(SweepUvError::Coordinates { face })?;
                    continue;
                }
                let point = rest[vertex].expect("boundary source has rest coordinates");
                let profile = point.profile.get() - 1;
                coords[i] = match role {
                    FaceRole::Wall {
                        path_segment,
                        segment,
                        ..
                    } => {
                        let segment = index(segment);
                        let metric = &metrics[segment];
                        let u = if profile == segment {
                            metric.start + (metric.end - metric.start) * point.profile_fraction
                        } else if profile == metric.next && point.profile_fraction == 0.0 {
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
                        let metric = &metrics[profile];
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

    fn face_coordinates(
        &self,
        face: FaceId,
        uv: [f64; 2],
        metrics: &[Metric],
        offsets: &[usize],
    ) -> Option<[f64; 2]> {
        use crate::geometry::Curve;
        use exedra_math::{dot, scale, sub};
        let record = self.body.face(face)?;
        let FaceRole::Wall {
            path_segment,
            segment,
            ..
        } = record.role
        else {
            // Cap supports are the authored moving-section frames.
            return Some(uv);
        };
        let metric = &metrics[offsets[match segment.loop_ref {
            LoopRef::Outer => 0,
            LoopRef::Hole(i) => i + 1,
        }] + segment.segment];
        let (mut section, mut rail) = (None, None);
        for &ring in &record.loops {
            for coedge in &self.body.boundary_loop(ring)?.coedges {
                let edge = self.body.edge(coedge.edge)?;
                match edge.role {
                    EdgeRole::Section {
                        station,
                        segment: source,
                    } if station == path_segment && source == segment => section = edge.regular(),
                    EdgeRole::Rail {
                        path_segment: span,
                        vertex,
                    } if span == path_segment && vertex == segment => rail = edge.regular(),
                    _ => {}
                }
            }
        }
        let (section, section_range) = section?;
        let (rail, rail_range) = rail?;
        let point = self.body.evaluate_support(face, uv)?.point;
        let parameter = curve_parameter(rail, rail_range, point)?;
        let angle = parameter - rail_range.bounds()[0];
        let path_fraction = angle / rail_range.span();
        let rest = match rail {
            Curve::Line(line) => sub(point, scale(line.direction(), angle)),
            Curve::Circle(circle) => {
                // Undo only this span's rotation about its authoritative rail
                // axis. Axial profile offsets do not affect the rail angle.
                let frame = circle.frame();
                let delta = sub(point, frame.rows.map(|r| r[3]));
                let local =
                    core::array::from_fn::<_, 3, _>(|i| dot(delta, frame.rows.map(|r| r[i])));
                let (s, c) = (libm::sin(angle), libm::cos(angle));
                frame.transform_point([
                    c * local[0] + s * local[1],
                    -s * local[0] + c * local[1],
                    local[2],
                ])
            }
            _ => return None,
        };
        let profile_fraction = (curve_parameter(section, section_range, rest)?
            - section_range.bounds()[0])
            / section_range.span();
        Some([
            metric.start + (metric.end - metric.start) * profile_fraction,
            self.path_distances[path_segment]
                + (self.path_distances[path_segment + 1] - self.path_distances[path_segment])
                    * path_fraction,
        ])
    }
}

fn curve_parameter(
    curve: &crate::geometry::Curve,
    range: crate::geometry::ParameterRange,
    point: [f64; 3],
) -> Option<f64> {
    use crate::geometry::Curve;
    use exedra_math::{dot, sub};
    let (value, periodic) = match curve {
        Curve::Line(line) => {
            let direction = line.direction();
            (
                dot(sub(point, line.origin()), direction) / dot(direction, direction),
                false,
            )
        }
        Curve::Circle(circle) => {
            let frame = circle.frame();
            let delta = sub(point, frame.rows.map(|r| r[3]));
            (
                libm::atan2(
                    dot(delta, frame.rows.map(|r| r[1])),
                    dot(delta, frame.rows.map(|r| r[0])),
                ),
                true,
            )
        }
        _ => return None,
    };
    let [start, end] = range.bounds();
    interval_coordinate(value, start, end, periodic)
}

fn interval_coordinate(mut value: f64, start: f64, end: f64, periodic: bool) -> Option<f64> {
    let (lo, hi) = (start.min(end), start.max(end));
    if periodic && !(lo..=hi).contains(&value) {
        let period = core::f64::consts::TAU;
        value += period * libm::round((0.5 * start + 0.5 * end - value) / period);
    }
    let slack = 64.0 * f64::EPSILON * (1.0 + lo.abs() + hi.abs());
    (value.is_finite() && value >= lo - slack && value <= hi + slack).then(|| value.clamp(lo, hi))
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

    #[test]
    fn quality_face_interiors_preserve_straight_and_curved_sweep_charts() {
        use alloc::vec;
        use exedra_math::path::PathSegment3;
        use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
        let profiles = [
            Loop::new(vec![
                Segment::line([2.0, -2.0]),
                Segment::line([2.0, 2.0]),
                Segment::line([-2.0, 2.0]),
                Segment::line([-2.0, -2.0]),
            ])
            .unwrap(),
            Loop::new(vec![
                Segment::arc([2.0, 0.0], 1.0),
                Segment::arc([-2.0, 0.0], 1.0),
            ])
            .unwrap(),
            Loop::new(vec![
                Segment::line([2.0, -2.0]),
                Segment::line([3.0, 2.0]),
                Segment::line([-2.0, 2.0]),
                Segment::line([-3.0, -2.0]),
            ])
            .unwrap(),
        ];
        let mut face_interiors = 0;
        for ring in profiles {
            for path in [
                vec![PathSegment3::Line {
                    to: [0.0, 0.0, 8.0],
                }],
                vec![PathSegment3::Arc {
                    axis_origin: [-40.0, 0.0, 0.0],
                    axis: [0.0, 0.0, 1.0],
                    sweep: 4.5,
                }],
                vec![PathSegment3::Arc {
                    axis_origin: [-40.0, 0.0, 0.0],
                    axis: [0.0, 0.0, 1.0],
                    sweep: -core::f64::consts::PI,
                }],
                rail().to_vec(),
            ] {
                let profile =
                    Profile::new(ring.clone(), vec![], &ProfilePolicy::new(1e-9)).unwrap();
                let solid = Sweep::new(
                    profile,
                    &view(&path),
                    Placement3::rotate_y_then_translate(0.37, 3.0, -2.0, 4.0),
                )
                .unwrap();
                let policy = RealizationPolicy {
                    tolerance: 0.02,
                    quality: Some(crate::realize::TriangleQuality::new(1.0)),
                    ..Default::default()
                };
                let plain = solid.realize(&policy).unwrap();
                let mut calls = 0;
                let mapped = solid.realize_with_uvs(&policy, |id, _| id.0,
                    |role, points| -> Result<_, core::convert::Infallible> {
                        let (triangle, face) = plain.mesh_faces[calls];
                        calls += 1;
                        assert_eq!(solid.body.faces()[face.index()].role, role);
                        // Independent closed forms cover the straight and first
                        // circular span; the multi-span case also checks mapping.
                        if path.len() == 1 {
                            for (corner, coordinate) in plain.mesh.face_loop(triangle).zip(points) {
                                let (profile, station) = match role {
                                    FaceRole::Cap { end } => (coordinate, if end == End::Start { 0.0 } else { solid.path_length() }),
                                    FaceRole::Wall { segment, .. } => (profile_point(&solid.profile, segment, coordinate[0]), coordinate[1]),
                                };
                                let local = if matches!(path[0], PathSegment3::Line { .. }) {
                                    [profile[0], profile[1], station]
                                } else {
                                    let PathSegment3::Arc { sweep, .. } = path[0] else { unreachable!() };
                                    let sign = sweep.signum();
                                    let theta = sign*station/40.0;
                                    [-40.0+(40.0+profile[0])*libm::cos(theta), (40.0+profile[0])*libm::sin(theta), -sign*profile[1]]
                                };
                                let expected = solid.frame.transform_point(local);
                                let actual = plain.mesh.vertex_position(plain.mesh.to_vertex(corner).unwrap()).unwrap().map(f64::from);
                                assert!(exedra_math::norm(exedra_math::sub(expected, actual)) <= plain.report.max_vertex_displacement + 1e-10,
                                    "construction coordinates must reconstruct the emitted geometry");
                            }
                        }
                        Ok(points.map(|p| { let p = exedra_math::narrow([p[0],p[1],0.0]); [p[0],p[1]] }))
                    }).unwrap();
                assert_eq!(calls, plain.mesh_faces.len());
                assert_eq!(mapped.mesh_vertices, plain.mesh_vertices);
                assert_eq!(mapped.mesh_faces, plain.mesh_faces);
                assert_eq!(mapped.report, plain.report);
                face_interiors += mapped
                    .mesh_vertices
                    .iter()
                    .filter(|(_, s)| matches!(s, VertexSource::Face { .. }))
                    .count();
                let uvs = mapped
                    .mesh
                    .attrs()
                    .sparse(exedra_mesh::attr::CORNER_UV)
                    .unwrap();
                assert!(
                    mapped.mesh.half_edges().all(|c| uvs
                        .get(c.as_id())
                        .is_some_and(|p| p.iter().all(|v| v.is_finite()))),
                    "every triangle corner must have finite UVs"
                );
            }
        }
        assert!(
            face_interiors > 0,
            "quality cases must exercise face-interior coordinates"
        );
    }
    fn profile_point(
        profile: &exedra_profile::Profile,
        source: SegmentRef,
        distance: f64,
    ) -> [f64; 2] {
        let ring = match source.loop_ref {
            LoopRef::Outer => profile.outer(),
            LoopRef::Hole(i) => &profile.holes()[i],
        };
        let mut start = 0.0;
        for (i, (from, segment)) in ring.iter_with_starts().enumerate() {
            let arc = ring.arc(i);
            let length = arc.map_or_else(
                || libm::hypot(segment.to[0] - from[0], segment.to[1] - from[1]),
                |a| a.length(),
            );
            if i == source.segment {
                let fraction = (distance - start) / length;
                assert!(
                    (-1e-10..=1.0 + 1e-10).contains(&fraction),
                    "profile distance must belong to its owning segment"
                );
                return arc.map_or_else(
                    || core::array::from_fn(|k| from[k] + fraction * (segment.to[k] - from[k])),
                    |a| a.point_at(fraction),
                );
            }
            start += length;
        }
        panic!("live source segment");
    }
}
