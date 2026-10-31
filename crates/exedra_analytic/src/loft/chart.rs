// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact reference-section distances with caller-authored longitudinal stations.

use super::{EdgeRole, FaceRole, Loft, loop_index, loops};
use crate::{
    End,
    body::{Face, FaceId},
    realize::{RealizationError, RealizationPolicy, RealizedBody, VertexSource},
};
use alloc::vec::Vec;
use exedra_math::predicates::{MAX_COORDINATE, Orientation, orient2d};
use exedra_profile::{LoopRef, SegmentRef};

/// A native loft could not supply checked construction coordinates or mapped UVs.
#[derive(Clone, Debug, PartialEq)]
pub enum LoftUvError<E> {
    /// Native mesh realization failed.
    Realization(RealizationError),
    /// Reference section or increasing finite longitudinal station list is invalid.
    Metric,
    /// A native face's construction coordinates cannot be represented without folding.
    Coordinates {
        /// Refused native face.
        face: FaceId,
    },
    /// Caller texture mapping failed.
    Mapping {
        /// Refused native face.
        face: FaceId,
        /// Original caller diagnosis.
        error: E,
    },
    /// Caller mapping returned a non-finite f32 coordinate.
    NonFiniteUv {
        /// Refused native face.
        face: FaceId,
    },
}
impl<E: core::fmt::Display> core::fmt::Display for LoftUvError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Realization(e) => write!(f, "loft realization: {e}"),
            Self::Metric => write!(f, "loft reference section or station metric is invalid"),
            Self::Coordinates { face } => write!(
                f,
                "loft construction coordinates fold or overflow on {face:?}"
            ),
            Self::Mapping { face, error } => write!(f, "loft mapping on {face:?}: {error}"),
            Self::NonFiniteUv { face } => {
                write!(f, "loft mapping on {face:?} returned non-finite UVs")
            }
        }
    }
}
impl<E: core::error::Error + 'static> core::error::Error for LoftUvError<E> {}

impl Loft {
    /// Remeshes with exact construction coordinates and caller texture mapping.
    ///
    /// Walls receive `(reference_profile_distance, station_distance)`. Every
    /// loop starts at its authored seam; its metric follows exact source curves
    /// in `reference_section`. `stations` supplies one finite, strictly increasing
    /// rest distance per section. It deliberately controls stretch independently
    /// of the loft's datum motion. Caps receive their original profile `(x,y)`.
    /// Occurrence placement preserves these rest coordinates and geometric seams
    /// share vertices. No partially mapped result is returned.
    ///
    /// # Errors
    /// Returns realization, rest-metric, coordinate or caller mapping failures.
    pub fn realize_with_uvs<E>(
        &self,
        policy: &RealizationPolicy,
        reference_section: usize,
        stations: &[f64],
        region: impl Fn(FaceId, &Face<FaceRole>) -> u32,
        mut map: impl FnMut(FaceRole, [[f64; 2]; 3]) -> Result<[[f32; 2]; 3], E>,
    ) -> Result<RealizedBody, LoftUvError<E>> {
        if reference_section >= self.sections.len()
            || stations.len() != self.sections.len()
            || stations.iter().any(|v| !v.is_finite())
            || stations
                .windows(2)
                .any(|w| !((w[1] - w[0]).is_finite() && w[1] > w[0]))
        {
            return Err(LoftUvError::Metric);
        }
        let mut distances = Vec::with_capacity(self.ring_offsets.len());
        for (_, ring) in loops(&self.sections[reference_section].1) {
            let mut values = Vec::with_capacity(ring.segments().len() + 1);
            values.push(0.0);
            let mut total = 0.0;
            for (i, (from, seg)) in ring.iter_with_starts().enumerate() {
                let end = total
                    + ring.arc(i).map_or_else(
                        || libm::hypot(seg.to[0] - from[0], seg.to[1] - from[1]),
                        |a| a.length(),
                    );
                if !end.is_finite() || end <= total {
                    return Err(LoftUvError::Metric);
                }
                values.push(end);
                total = end;
            }
            distances.push(values);
        }
        let mut result = self
            .realize_with_regions(policy, region)
            .map_err(LoftUvError::Realization)?;
        let mut mapped = Vec::with_capacity(result.mesh.half_edge_count());
        for &(mesh_face, face) in &result.mesh_faces {
            let role = self.body.faces()[face.index()].role;
            let mut corners = result.mesh.face_loop(mesh_face);
            let corners =
                core::array::from_fn::<_, 3, _>(|_| corners.next().expect("realized triangle"));
            let mut coordinates = [[0.0; 2]; 3];
            for (i, &corner) in corners.iter().enumerate() {
                let vertex = result.mesh.to_vertex(corner).expect("live corner");
                let source = result.mesh_vertices[vertex.index() as usize].1;
                if matches!(role, FaceRole::Cap { .. })
                    && let VertexSource::Face { face: owner, uv } = source
                {
                    if owner != face {
                        return Err(LoftUvError::Coordinates { face });
                    }
                    // Cap supports use the authored section frame. Native plane
                    // parameters therefore already are the profile's rest X/Y,
                    // independently of cap sense or occurrence placement.
                    coordinates[i] = uv;
                    continue;
                }
                coordinates[i] = match role {
                    FaceRole::Wall { band, segment, .. } => {
                        let uv = self
                            .wall_parameter(face, band, segment, source)
                            .ok_or(LoftUvError::Coordinates { face })?;
                        let ds = &distances[loop_index(segment.loop_ref)];
                        [
                            ds[segment.segment]
                                + uv[0] * (ds[segment.segment + 1] - ds[segment.segment]),
                            stations[band] + uv[1] * (stations[band + 1] - stations[band]),
                        ]
                    }
                    FaceRole::Cap { end } => {
                        let section = if end == End::Start {
                            0
                        } else {
                            self.sections.len() - 1
                        };
                        let profile = &self.sections[section].1;
                        let (segment, fraction) = match source {
                            VertexSource::Vertex(id) => {
                                if id.index() / self.section_vertices != section {
                                    return Err(LoftUvError::Coordinates { face });
                                }
                                let k = id.index() % self.section_vertices;
                                let lr = self.ring_offsets.partition_point(|&o| o <= k) - 1;
                                (
                                    SegmentRef {
                                        loop_ref: if lr == 0 {
                                            LoopRef::Outer
                                        } else {
                                            LoopRef::Hole(lr - 1)
                                        },
                                        segment: k - self.ring_offsets[lr],
                                    },
                                    0.0,
                                )
                            }
                            VertexSource::Edge { edge, parameter } => {
                                match self.body.edges()[edge.index()].role {
                                    EdgeRole::Section {
                                        section: s,
                                        segment,
                                    } if s == section => (segment, parameter),
                                    _ => return Err(LoftUvError::Coordinates { face }),
                                }
                            }
                            _ => return Err(LoftUvError::Coordinates { face }),
                        };
                        let ring = match segment.loop_ref {
                            LoopRef::Outer => profile.outer(),
                            LoopRef::Hole(i) => &profile.holes()[i],
                        };
                        let from = ring.segments()
                            [(segment.segment + ring.segments().len() - 1) % ring.segments().len()]
                        .to;
                        ring.arc(segment.segment).map_or_else(
                            || {
                                core::array::from_fn(|k| {
                                    from[k]
                                        + fraction
                                            * (ring.segments()[segment.segment].to[k] - from[k])
                                })
                            },
                            |a| a.point_at(fraction),
                        )
                    }
                };
            }
            let expected = if matches!(role, FaceRole::Cap { end: End::Start }) {
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
                return Err(LoftUvError::Coordinates { face });
            }
            let uv =
                map(role, coordinates).map_err(|error| LoftUvError::Mapping { face, error })?;
            if uv.iter().flatten().any(|v| !v.is_finite()) {
                return Err(LoftUvError::NonFiniteUv { face });
            }
            mapped.extend(corners.into_iter().zip(uv));
        }
        if mapped.windows(2).any(|w| w[0].0.index() >= w[1].0.index()) {
            mapped.sort_unstable_by_key(|p| p.0.index());
        }
        let mut edit = result.mesh.edit();
        exedra_mesh::op::set_corner_uvs(
            &mut edit,
            exedra_mesh::uv_mapping::UvSet::Primary,
            &mapped,
        )
        .expect("fresh unique sorted triangle corners");
        let _: () = edit.finish();
        Ok(result)
    }
    fn wall_parameter(
        &self,
        face: FaceId,
        band: usize,
        segment: SegmentRef,
        source: VertexSource,
    ) -> Option<[f64; 2]> {
        let lr = loop_index(segment.loop_ref);
        let base = self.ring_offsets[lr];
        let n = if lr + 1 < self.ring_offsets.len() {
            self.ring_offsets[lr + 1] - base
        } else {
            self.section_vertices - base
        };
        let next = SegmentRef {
            segment: (segment.segment + 1) % n,
            ..segment
        };
        let u_for = |vertex: SegmentRef| {
            if vertex == segment {
                Some(0.0)
            } else if vertex == next {
                Some(1.0)
            } else {
                None
            }
        };
        let v_for = |station: usize| {
            if station == band {
                Some(0.0)
            } else if station == band + 1 {
                Some(1.0)
            } else {
                None
            }
        };
        match source {
            VertexSource::Face { face: f, uv } if f == face => Some(uv),
            VertexSource::Vertex(id) => {
                let k = id.index() % self.section_vertices;
                let s = if k == base + segment.segment {
                    segment
                } else if k == base + next.segment {
                    next
                } else {
                    return None;
                };
                Some([u_for(s)?, v_for(id.index() / self.section_vertices)?])
            }
            VertexSource::Edge { edge, parameter } => match self.body.edges()[edge.index()].role {
                EdgeRole::Section {
                    section,
                    segment: s,
                } if s == segment => Some([parameter, v_for(section)?]),
                EdgeRole::Rail { band: b, vertex } if b == band => {
                    Some([u_for(vertex)?, parameter])
                }
                _ => None,
            },
            _ => None,
        }
    }
}
