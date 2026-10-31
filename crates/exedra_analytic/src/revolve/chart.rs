// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction-coordinate delivery, independent of texture mapping policy.

use super::{EdgeRole, FaceRole, Revolution, loops};
use crate::End;
use crate::body::FaceId;
use crate::realize::{RealizationError, RealizationPolicy, RealizedBody, VertexSource};
use alloc::vec::Vec;
use exedra_math::predicates::{MAX_COORDINATE, Orientation, orient2d};
use exedra_profile::{LoopRef, SegmentRef};

/// Why a charted revolution was refused. The caller receives no partial result.
#[derive(Clone, Debug, PartialEq)]
pub enum RevolutionUvError<E> {
    /// Geometry realization was refused before mapping.
    Realization(RealizationError),
    /// Cumulative profile distance cannot preserve a segment's chart span.
    Metric {
        /// Source segment whose distance collapsed or overflowed.
        segment: SegmentRef,
    },
    /// Construction coordinates cannot represent a native face without a fold.
    Coordinates {
        /// Native face that could not be charted.
        face: FaceId,
    },
    /// Caller-supplied texture mapping was refused.
    Mapping {
        /// Native face whose mapping failed.
        face: FaceId,
        /// Caller mapping diagnosis.
        error: E,
    },
    /// The mapping produced a non-finite texture coordinate.
    NonFiniteUv {
        /// Native face whose stored UVs were invalid.
        face: FaceId,
    },
}
impl<E: core::fmt::Display> core::fmt::Display for RevolutionUvError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Realization(error) => write!(f, "revolution realization: {error}"),
            Self::Metric { segment } => {
                write!(f, "revolution profile distance cannot preserve {segment:?}")
            }
            Self::Coordinates { face } => write!(
                f,
                "revolution construction coordinates fold or exceed limits on {face:?}"
            ),
            Self::Mapping { face, error } => {
                write!(f, "revolution texture mapping on {face:?}: {error}")
            }
            Self::NonFiniteUv { face } => {
                write!(f, "revolution mapping on {face:?} produced non-finite UVs")
            }
        }
    }
}
impl<E: core::error::Error + 'static> core::error::Error for RevolutionUvError<E> {}

#[derive(Copy, Clone)]
struct RestPoint {
    vertex: SegmentRef,
    fraction: f64,
    theta: f64,
}

impl Revolution {
    /// Realizes with caller regions and checked construction-coordinate mapping.
    ///
    /// `map` receives one triangle's native face ID, role and corner coordinates:
    /// curved walls use `(theta, profile_distance)`, radial disks use
    /// their supporting plane's local X/Y, and angular caps `(radius, height)`.
    /// The face ID permits choosing a planar mapping without changing source
    /// roles: a radial BASE segment remains a wall, not an angular cap. Each
    /// loop's distance starts at its authored seam. Full-turn angular corners
    /// retain both zero and `TAU` on the shared seam, without duplicating geometry.
    /// Placement does not change these rest coordinates.
    ///
    /// A construction-aware revolution chart maps wall theta through its authored
    /// reference radius before its texture transform. The mapper owns that
    /// policy and f32 mapping checks; the kernel never interprets texture scale.
    /// UVs are written as face-varying primary coordinates only after all faces
    /// map successfully. The uncharted realization path allocates none of this
    /// coordinate storage.
    ///
    /// # Errors
    /// Returns geometry, rest-coordinate or caller mapping refusals. Non-finite
    /// mapped values are refused even when the caller does not check them.
    pub fn realize_with_uvs<E>(
        &self,
        policy: &RealizationPolicy,
        region: impl Fn(FaceId, &crate::body::Face<FaceRole>) -> u32,
        mut map: impl FnMut(FaceId, FaceRole, [[f64; 2]; 3]) -> Result<[[f32; 2]; 3], E>,
    ) -> Result<RealizedBody, RevolutionUvError<E>> {
        let mut result = self
            .realize_with_regions(policy, region)
            .map_err(RevolutionUvError::Realization)?;
        let mut distances = Vec::new();
        let mut offsets = Vec::new();
        for (loop_ref, ring) in loops(&self.profile) {
            offsets.push(distances.len());
            distances.push(0.0);
            let mut total = 0.0;
            for (i, (a, segment)) in ring.iter_with_starts().enumerate() {
                let next = total
                    + ring.arc(i).map_or_else(
                        || libm::hypot(segment.to[0] - a[0], segment.to[1] - a[1]),
                        |a| a.length(),
                    );
                if !next.is_finite() || next <= total {
                    return Err(RevolutionUvError::Metric {
                        segment: SegmentRef {
                            loop_ref,
                            segment: i,
                        },
                    });
                }
                distances.push(next);
                total = next;
            }
        }
        let mut vertices = alloc::vec![None;self.body.vertices().len()];
        for edge in self.body.edges() {
            if let EdgeRole::Orbit { vertex } = edge.role {
                vertices[edge.start.index()] = Some(RestPoint {
                    vertex,
                    fraction: 0.0,
                    theta: 0.0,
                });
                if edge.end != edge.start {
                    vertices[edge.end.index()] = Some(RestPoint {
                        vertex,
                        fraction: 0.0,
                        theta: self.sweep,
                    });
                }
            }
        }
        let rest: Vec<_> = result
            .mesh_vertices
            .iter()
            .map(|(_, source)| match *source {
                VertexSource::Vertex(id) => {
                    Some(vertices[id.index()].expect("revolution vertices belong to orbits"))
                }
                VertexSource::Edge { edge, parameter } => {
                    let edge = &self.body.edges()[edge.index()];
                    Some(match edge.role {
                        EdgeRole::Orbit { vertex } => RestPoint {
                            vertex,
                            fraction: 0.0,
                            theta: edge
                                .regular()
                                .expect("interior edge sample is regular")
                                .1
                                .at(parameter)
                                .expect("bounded native sample"),
                        },
                        EdgeRole::Meridian { segment, end } => RestPoint {
                            vertex: segment,
                            fraction: parameter,
                            theta: if end == End::Start { 0.0 } else { self.sweep },
                        },
                    })
                }
                // Interior vertices have supporting-surface coordinates, not a
                // fictitious boundary segment. Convert them in their own face.
                VertexSource::Face { .. } => None,
            })
            .collect();
        let mut mapped = Vec::with_capacity(result.mesh.half_edge_count());
        for &(mesh_face, face) in &result.mesh_faces {
            let native_face = &self.body.faces()[face.index()];
            let role = native_face.role;
            let planar_wall = matches!(role, FaceRole::Wall { .. })
                && matches!(native_face.surface, crate::geometry::Surface::Plane(_))
                && native_face.loops.len() == 1;
            let mut corners = result.mesh.face_loop(mesh_face);
            let corners = core::array::from_fn::<_, 3, _>(|_| {
                corners.next().expect("realization emits triangles")
            });
            let mut coordinates = [[0.0; 2]; 3];
            for i in 0..3 {
                let corner = corners[i];
                let vertex = result
                    .mesh
                    .to_vertex(corner)
                    .expect("live mesh corner")
                    .index() as usize;
                if let VertexSource::Face {
                    face: source_face,
                    uv,
                } = result.mesh_vertices[vertex].1
                {
                    if source_face != face {
                        return Err(RevolutionUvError::Coordinates { face });
                    }
                    coordinates[i] = self
                        .face_coordinates(face, uv, &distances, &offsets)
                        .ok_or(RevolutionUvError::Coordinates { face })?;
                    continue;
                }
                let mut point = rest[vertex].expect("boundary source has rest coordinates");
                if let Some(pole) = result.pole_corners.get(corner) {
                    point.theta = pole.uv[0];
                }
                let ring = match point.vertex.loop_ref {
                    LoopRef::Outer => self.profile.outer(),
                    LoopRef::Hole(i) => &self.profile.holes()[i],
                };
                let from = ring.segments()
                    [(point.vertex.segment + ring.segments().len() - 1) % ring.segments().len()]
                .to;
                coordinates[i] = match role {
                    FaceRole::Cap { .. } => ring.arc(point.vertex.segment).map_or_else(
                        || {
                            core::array::from_fn(|k| {
                                from[k]
                                    + point.fraction
                                        * (ring.segments()[point.vertex.segment].to[k] - from[k])
                            })
                        },
                        |a| a.point_at(point.fraction),
                    ),
                    FaceRole::Wall { segment, .. } => {
                        let ring_index = match segment.loop_ref {
                            LoopRef::Outer => 0,
                            LoopRef::Hole(i) => i + 1,
                        };
                        let base = offsets[ring_index] + segment.segment;
                        let distance = if point.vertex == segment {
                            distances[base]
                                + point.fraction * (distances[base + 1] - distances[base])
                        } else if point.vertex.loop_ref == segment.loop_ref
                            && point.vertex.segment == (segment.segment + 1) % ring.segments().len()
                            && point.fraction == 0.0
                        {
                            // The final segment ends at loop length while its
                            // endpoint shares the authored start vertex.
                            distances[base + 1]
                        } else {
                            return Err(RevolutionUvError::Coordinates { face });
                        };
                        if planar_wall {
                            let radius = from[0]
                                + point.fraction
                                    * (ring.segments()[point.vertex.segment].to[0] - from[0]);
                            [
                                radius * libm::cos(point.theta),
                                radius * libm::sin(point.theta),
                            ]
                        } else {
                            [point.theta, distance]
                        }
                    }
                };
            }
            if matches!(role, FaceRole::Wall { .. })
                && !planar_wall
                && self.sweep == core::f64::consts::TAU
            {
                let min = coordinates
                    .iter()
                    .map(|p| p[0])
                    .fold(f64::INFINITY, f64::min);
                let max = coordinates
                    .iter()
                    .map(|p| p[0])
                    .fold(f64::NEG_INFINITY, f64::max);
                if max - min > core::f64::consts::PI {
                    for p in &mut coordinates {
                        if p[0] < core::f64::consts::PI {
                            p[0] += core::f64::consts::TAU;
                        }
                    }
                }
            }
            let expected = if matches!(role, FaceRole::Cap { end: End::End })
                || (planar_wall && native_face.sense == crate::body::Sense::Reverse)
            {
                Orientation::Cw
            } else {
                Orientation::Ccw
            };
            if coordinates
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
                || orient2d(coordinates[0], coordinates[1], coordinates[2]) != expected
            {
                return Err(RevolutionUvError::Coordinates { face });
            }
            let uv = map(face, role, coordinates)
                .map_err(|error| RevolutionUvError::Mapping { face, error })?;
            if !uv.iter().flatten().all(|v| v.is_finite()) {
                return Err(RevolutionUvError::NonFiniteUv { face });
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
        .expect("fresh triangle corners are unique and sorted; no existing UV channel");
        let _: () = edit.finish();
        Ok(result)
    }

    fn face_coordinates(
        &self,
        face: FaceId,
        uv: [f64; 2],
        distances: &[f64],
        offsets: &[usize],
    ) -> Option<[f64; 2]> {
        let record = self.body.face(face)?;
        let FaceRole::Wall { segment, .. } = record.role else {
            // Angular cap supports use the authored radius/height frame.
            return Some(uv);
        };
        if matches!(record.surface, crate::geometry::Surface::Plane(_)) && record.loops.len() == 1 {
            // Radial disks/sectors use the support's Cartesian coordinates.
            return Some(uv);
        }
        let ring_index = match segment.loop_ref {
            LoopRef::Outer => 0,
            LoopRef::Hole(i) => i + 1,
        };
        let ring = match segment.loop_ref {
            LoopRef::Outer => self.profile.outer(),
            LoopRef::Hole(i) => &self.profile.holes()[i],
        };
        let base = offsets[ring_index] + segment.segment;
        let from = ring.segments()
            [(segment.segment + ring.segments().len() - 1) % ring.segments().len()]
        .to;
        let to = ring.segments()[segment.segment].to;
        let fraction = if let Some(arc) = ring.arc(segment.segment) {
            let angle =
                interval_coordinate(uv[1], arc.start_angle, arc.start_angle + arc.sweep, true)?;
            (angle - arc.start_angle) / arc.sweep
        } else {
            let point = self.body.evaluate_support(face, uv)?.point;
            let height = interval_coordinate(point[1], from[1], to[1], false)?;
            (height - from[1]) / (to[1] - from[1])
        };
        Some([
            interval_coordinate(uv[0], 0.0, self.sweep, true)?,
            distances[base] + fraction * (distances[base + 1] - distances[base]),
        ])
    }
}

// Inverse surface parameters may use a principal angle. Choose its authored
// interval representative; only endpoint roundoff is clamped.
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
    use crate::revolve::Revolution;
    use alloc::vec;
    use exedra_math::Placement3;
    use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
    fn solid(sweep: f64) -> Revolution {
        let ring = Loop::new(vec![
            Segment::line([2.0, 0.0]),
            Segment::line([3.0, 3.0]),
            Segment::line([1.0, 3.0]),
            Segment::line([1.0, 0.0]),
        ])
        .unwrap();
        Revolution::new(
            Profile::new(ring, vec![], &ProfilePolicy::new(1e-9)).unwrap(),
            Placement3::IDENTITY,
            sweep,
        )
        .unwrap()
    }
    #[test]
    fn full_and_partial_charts_preserve_geometry_and_open_only_texture_seams() {
        for sweep in [core::f64::consts::TAU, 1.8] {
            let solid = solid(sweep);
            let policy = RealizationPolicy::default();
            let plain = solid.realize(&policy).unwrap();
            let charted = solid
                .realize_with_uvs(
                    &policy,
                    |id, _| id.0,
                    |_, _, points| -> Result<_, core::convert::Infallible> {
                        Ok(points.map(|p| {
                            let p = exedra_math::narrow([p[0], p[1], 0.0]);
                            [p[0], p[1]]
                        }))
                    },
                )
                .unwrap();
            assert!(charted.mesh.boundary_loops().unwrap().is_empty());
            assert_eq!(charted.report, plain.report);
            let positions = |mesh: &exedra_mesh::Mesh| {
                mesh.vertices()
                    .map(|id| mesh.vertex_position(id).unwrap().map(f32::to_bits))
                    .collect::<Vec<_>>()
            };
            assert_eq!(positions(&plain.mesh), positions(&charted.mesh));
            let uvs = charted
                .mesh
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_UV)
                .unwrap();
            assert!(
                charted
                    .mesh
                    .half_edges()
                    .all(|c| uvs.get(c.as_id()).is_some())
            );
            if sweep == core::f64::consts::TAU {
                let vertex = charted.mesh_vertices[0].0;
                let values: Vec<_> = charted
                    .mesh
                    .half_edges()
                    .filter(|&c| charted.mesh.to_vertex(c) == Some(vertex))
                    .map(|c| uvs.get(c.as_id()).unwrap()[0])
                    .collect();
                assert!(values.contains(&0.0));
                assert!(values.iter().any(|&u| (f64::from(u) - sweep).abs() < 1e-6));
            }
        }
    }
    #[test]
    fn mapping_errors_and_nonfinite_values_are_typed_refusals() {
        let solid = solid(1.8);
        let error = solid
            .realize_with_uvs(
                &RealizationPolicy::default(),
                |_, _| 0,
                |_, _, _| Err::<[[f32; 2]; 3], _>("invalid texture scale"),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            RevolutionUvError::Mapping {
                error: "invalid texture scale",
                ..
            }
        ));
        let error = solid
            .realize_with_uvs(
                &RealizationPolicy::default(),
                |_, _| 0,
                |_, _, _| Ok::<_, core::convert::Infallible>([[f32::NAN; 2]; 3]),
            )
            .unwrap_err();
        assert!(matches!(error, RevolutionUvError::NonFiniteUv { .. }));
    }

    #[test]
    fn curved_profile_charts_cover_meridians_and_face_interiors() {
        for sweep in [core::f64::consts::TAU, 1.8] {
            let ring = Loop::new(vec![
                Segment::arc([8.0, 0.0], 1.0),
                Segment::arc([12.0, 0.0], 1.0),
            ])
            .unwrap();
            let solid = Revolution::new(
                Profile::new(ring, vec![], &ProfilePolicy::new(1e-9)).unwrap(),
                Placement3::IDENTITY,
                sweep,
            )
            .unwrap();
            let policy = RealizationPolicy {
                tolerance: 0.05,
                quality: Some(crate::realize::TriangleQuality::new(0.5)),
                ..RealizationPolicy::default()
            };
            let output = solid
                .realize_with_uvs(
                    &policy,
                    |_, _| 0,
                    |_, _, p| -> Result<_, core::convert::Infallible> {
                        Ok(p.map(|v| {
                            let n = exedra_math::narrow([v[0], v[1], 0.0]);
                            [n[0], n[1]]
                        }))
                    },
                )
                .unwrap();
            assert!(output.report.within_tolerance());
            assert!(output.mesh.boundary_loops().unwrap().is_empty());
            let uvs = output
                .mesh
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_UV)
                .unwrap();
            assert!(
                output
                    .mesh
                    .half_edges()
                    .all(|c| uvs.get(c.as_id()).is_some())
            );
            let max = output
                .mesh
                .half_edges()
                .map(|c| uvs.get(c.as_id()).unwrap()[1])
                .fold(0.0_f32, f32::max);
            assert!((f64::from(max) - 4.0 * core::f64::consts::PI).abs() < 1e-5);
        }
    }

    #[test]
    fn quality_face_interiors_preserve_full_and_partial_charts_after_placement() {
        for sweep in [core::f64::consts::TAU, 4.5, 1.8] {
            let mut solid = solid(sweep);
            solid.frame = Placement3::rotate_y_then_translate(0.37, 3.0, -2.0, 4.0);
            let policy = RealizationPolicy {
                quality: Some(crate::realize::TriangleQuality::new(0.5)),
                ..Default::default()
            };
            let plain = solid.realize(&policy).unwrap();
            let mut calls = 0;
            let mut seam = [false; 2];
            let mapped = solid
                .realize_with_uvs(
                    &policy,
                    |id, _| id.0,
                    |face, role, points| -> Result<_, core::convert::Infallible> {
                        let (triangle, native) = plain.mesh_faces[calls];
                        calls += 1;
                        assert_eq!(native, face);
                        let planar = matches!(
                            solid.body.faces()[face.index()].surface,
                            crate::geometry::Surface::Plane(_)
                        ) && solid.body.faces()[face.index()].loops.len() == 1;
                        for (corner, coordinate) in plain.mesh.face_loop(triangle).zip(points) {
                            let local = match role {
                                FaceRole::Cap { end } => {
                                    let theta = if end == End::Start { 0.0 } else { sweep };
                                    [
                                        coordinate[0] * libm::cos(theta),
                                        coordinate[1],
                                        -coordinate[0] * libm::sin(theta),
                                    ]
                                }
                                FaceRole::Wall { segment, .. } => {
                                    if planar {
                                        let ring = match segment.loop_ref {
                                            LoopRef::Outer => solid.profile.outer(),
                                            LoopRef::Hole(i) => &solid.profile.holes()[i],
                                        };
                                        [
                                            coordinate[0],
                                            ring.segments()[segment.segment].to[1],
                                            -coordinate[1],
                                        ]
                                    } else {
                                        assert!(
                                            coordinate[0] >= -1e-12
                                                && coordinate[0] <= sweep + 1e-12,
                                            "wall theta must stay in the authored angular interval"
                                        );
                                        seam[0] |= coordinate[0].abs() < 1e-12;
                                        seam[1] |= (coordinate[0] - sweep).abs() < 1e-12;
                                        let p =
                                            profile_point(&solid.profile, segment, coordinate[1]);
                                        [
                                            p[0] * libm::cos(coordinate[0]),
                                            p[1],
                                            -p[0] * libm::sin(coordinate[0]),
                                        ]
                                    }
                                }
                            };
                            let expected = solid.frame.transform_point(local);
                            let actual = plain
                                .mesh
                                .vertex_position(plain.mesh.to_vertex(corner).unwrap())
                                .unwrap()
                                .map(f64::from);
                            assert!(
                                exedra_math::norm(exedra_math::sub(expected, actual))
                                    <= plain.report.max_vertex_displacement + 1e-10,
                                "construction coordinates must reconstruct the emitted geometry"
                            );
                        }
                        Ok(points.map(|p| {
                            let p = exedra_math::narrow([p[0], p[1], 0.0]);
                            [p[0], p[1]]
                        }))
                    },
                )
                .unwrap();
            assert_eq!(calls, plain.mesh_faces.len());
            if sweep == core::f64::consts::TAU {
                assert!(
                    seam[0] && seam[1],
                    "full turns must preserve both angular seam coordinates"
                );
            }
            assert_eq!(mapped.mesh_vertices, plain.mesh_vertices);
            assert_eq!(mapped.mesh_faces, plain.mesh_faces);
            assert_eq!(mapped.report, plain.report);
            assert!(
                mapped
                    .mesh_vertices
                    .iter()
                    .any(|(_, s)| matches!(s, VertexSource::Face { .. })),
                "quality must exercise face-interior coordinates"
            );
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
    fn profile_point(profile: &Profile, source: SegmentRef, distance: f64) -> [f64; 2] {
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
