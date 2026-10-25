// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction-coordinate delivery, independent of texture mapping policy.

use super::{EdgeRole, FaceRole, Revolution, loops};
use crate::End;
use crate::body::FaceId;
use crate::realize::{RealizationError, RealizationPolicy, RealizedBody, VertexSource};
use alloc::vec::Vec;
use exedra_profile::{LoopRef, SegmentRef};
use exedra_triangulate::predicates::{MAX_COORDINATE, Orientation, orient2d};

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
    theta: f64,
}

impl Revolution {
    /// Realizes with caller regions and checked construction-coordinate mapping.
    ///
    /// `map` receives one triangle's native face role and corner coordinates:
    /// walls use `(theta, profile_distance)` and caps `(radius, height)`. Each
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
        mut map: impl FnMut(FaceRole, [[f64; 2]; 3]) -> Result<[[f32; 2]; 3], E>,
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
                let next = total + libm::hypot(segment.to[0] - a[0], segment.to[1] - a[1]);
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
                vertices[edge.start.index()] = Some(RestPoint { vertex, theta: 0.0 });
                if edge.end != edge.start {
                    vertices[edge.end.index()] = Some(RestPoint {
                        vertex,
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
                    vertices[id.index()].expect("revolution vertices belong to orbits")
                }
                VertexSource::Edge { edge, parameter } => {
                    let edge = &self.body.edges()[edge.index()];
                    let EdgeRole::Orbit { vertex } = edge.role else {
                        unreachable!("straight meridians have no interior samples")
                    };
                    RestPoint {
                        vertex,
                        theta: edge.range.at(parameter).expect("bounded native sample"),
                    }
                }
            })
            .collect();
        let mut mapped = Vec::with_capacity(result.mesh.half_edge_count());
        for &(mesh_face, face) in &result.mesh_faces {
            let role = self.body.faces()[face.index()].role;
            let mut corners = result.mesh.face_loop(mesh_face);
            let corners = core::array::from_fn::<_, 3, _>(|_| {
                corners.next().expect("realization emits triangles")
            });
            let points = corners.map(|corner| {
                rest[result
                    .mesh
                    .to_vertex(corner)
                    .expect("live mesh corner")
                    .index() as usize]
            });
            let mut coordinates = [[0.0; 2]; 3];
            for i in 0..3 {
                let point = points[i];
                let ring = match point.vertex.loop_ref {
                    LoopRef::Outer => self.profile.outer(),
                    LoopRef::Hole(i) => &self.profile.holes()[i],
                };
                let from = ring.segments()
                    [(point.vertex.segment + ring.segments().len() - 1) % ring.segments().len()]
                .to;
                coordinates[i] = match role {
                    FaceRole::Cap { .. } => from,
                    FaceRole::Wall { segment, .. } => {
                        let ring_index = match segment.loop_ref {
                            LoopRef::Outer => 0,
                            LoopRef::Hole(i) => i + 1,
                        };
                        let local = if point.vertex == segment {
                            segment.segment
                        } else {
                            (segment.segment + 1) % ring.segments().len()
                        };
                        // The final segment ends at loop length, although its
                        // endpoint reuses the authored start vertex.
                        let index = if local == segment.segment {
                            segment.segment
                        } else {
                            segment.segment + 1
                        };
                        [point.theta, distances[offsets[ring_index] + index]]
                    }
                };
            }
            if matches!(role, FaceRole::Wall { .. }) && self.sweep == core::f64::consts::TAU {
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
            let expected = if matches!(role, FaceRole::Cap { end: End::End }) {
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
            let uv = map(role, coordinates)
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
                    |_, points| -> Result<_, core::convert::Infallible> {
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
                |_, _| Err::<[[f32; 2]; 3], _>("invalid texture scale"),
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
                |_, _| Ok::<_, core::convert::Infallible>([[f32::NAN; 2]; 3]),
            )
            .unwrap_err();
        assert!(matches!(error, RevolutionUvError::NonFiniteUv { .. }));
    }
}
