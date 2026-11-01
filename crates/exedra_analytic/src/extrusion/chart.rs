// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction coordinates for caller texture mapping.

use super::{BoundarySegment, Extrusion, FaceRole};
use crate::body::{Face, FaceId};
use crate::geometry::Surface;
use crate::realize::{RealizationError, RealizationPolicy, RealizedBody, VertexSource};
use alloc::vec::Vec;
use exedra_math::predicates::{MAX_COORDINATE, Orientation, orient2d};

/// Native realization or caller texture mapping could not be completed.
#[derive(Clone, Debug, PartialEq)]
pub enum ExtrusionUvError<E> {
    /// Native realization failed before texture mapping.
    Realization(RealizationError),
    /// A native source coordinate could not be represented.
    Coordinates {
        /// Native face.
        face: FaceId,
    },
    /// Caller texture mapping failed. No partially mapped result is returned.
    Mapping {
        /// Native face.
        face: FaceId,
        /// Original caller error.
        error: E,
    },
    /// Caller mapping supplied a nonfinite texture coordinate.
    NonFiniteUv {
        /// Native face.
        face: FaceId,
    },
}
impl<E: core::fmt::Display> core::fmt::Display for ExtrusionUvError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Realization(e) => write!(f, "extrusion realization: {e}"),
            Self::Coordinates { face } => {
                write!(f, "extrusion coordinates unavailable on {face:?}")
            }
            Self::Mapping { face, error } => write!(f, "extrusion mapping on {face:?}: {error}"),
            Self::NonFiniteUv { face } => {
                write!(f, "extrusion mapping returned nonfinite UVs on {face:?}")
            }
        }
    }
}
impl<E: core::error::Error + 'static> core::error::Error for ExtrusionUvError<E> {}

impl Extrusion {
    /// Realizes with exact construction coordinates and caller-owned texture
    /// policy.
    ///
    /// Caps and pocket floors receive local profile `(x, y)`. Walls receive
    /// `(u, z)`: `u` the fraction along the wall's boundary segment, by
    /// length for a line, by angle for an arc and by curve parameter, **not
    /// arc length**, for a Bezier segment, and `z` the model-unit height.
    /// Each segment has its own seam. These coordinates are independent of
    /// display tolerance and placement. Geometry vertices remain shared while
    /// corners can carry distinct UVs.
    ///
    /// # Errors
    /// Returns realization, coordinate, caller mapping or nonfinite UV
    /// failures. No partially mapped mesh is returned.
    pub fn realize_with_uvs<E>(
        &self,
        policy: &RealizationPolicy,
        region: impl Fn(FaceId, &Face<FaceRole>) -> u32,
        mut map: impl FnMut(FaceRole, [[f64; 2]; 3]) -> Result<[[f32; 2]; 3], E>,
    ) -> Result<RealizedBody, ExtrusionUvError<E>> {
        let mut result = self
            .body
            .realize_with_regions(self.frame, policy, region)
            .map_err(ExtrusionUvError::Realization)?;
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
                coordinates[i] = self
                    .coordinates(face, source)
                    .ok_or(ExtrusionUvError::Coordinates { face })?;
            }
            // Walls and upward planar faces run counter-clockwise in their
            // coordinates, as seen from outside.
            let upward = match role {
                FaceRole::Wall { .. } => true,
                _ => self
                    .planes()
                    .iter()
                    .find(|plane| plane.face == face.index())
                    .is_some_and(|plane| plane.upward),
            };
            let expected = if upward {
                Orientation::Ccw
            } else {
                Orientation::Cw
            };
            if coordinates
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
                || orient2d(coordinates[0], coordinates[1], coordinates[2]) != expected
            {
                return Err(ExtrusionUvError::Coordinates { face });
            }
            let uvs = map(role, coordinates)
                .map_err(|error| ExtrusionUvError::Mapping { face, error })?;
            if uvs.iter().flatten().any(|v| !v.is_finite()) {
                return Err(ExtrusionUvError::NonFiniteUv { face });
            }
            for (corner, uv) in corners.into_iter().zip(uvs) {
                mapped.push((corner, uv));
            }
        }
        mapped.sort_unstable_by_key(|p| p.0.index());
        let mut edit = result.mesh.edit();
        exedra_mesh::op::set_corner_uvs(
            &mut edit,
            exedra_mesh::uv_mapping::UvSet::Primary,
            &mapped,
        )
        .expect("unique sorted fresh triangle corners");
        let _: () = edit.finish();
        Ok(result)
    }

    fn coordinates(&self, face: FaceId, source: VertexSource) -> Option<[f64; 2]> {
        let record = self.body.faces().get(face.index())?;
        let FaceRole::Wall { segment, .. } = record.role else {
            // A planar face's parameters are the profile plane's.
            let p = match source {
                VertexSource::Vertex(id) => *self.body.vertices().get(id.index())?,
                VertexSource::Edge { edge, parameter } => self.body.edge_point(edge, parameter)?,
                VertexSource::Face { face: owner, uv } => return (owner == face).then_some(uv),
            };
            return Some([p[0], p[1]]);
        };
        let band = self.band_of(segment.boundary);
        let [low, high] = band.heights;
        let up = |fraction: f64| low + fraction * (high - low);
        let n = self.boundary_loop(segment.boundary).segments().len();
        let k = segment.segment;
        let previous = (k + n - 1) % n;
        match source {
            VertexSource::Face { face: owner, uv } => {
                if owner != face {
                    return None;
                }
                let [lo, hi] = record.domain.u.bounds();
                let along = (uv[0] - lo) / (hi - lo);
                Some(match &record.surface {
                    Surface::Plane(_) => [along, uv[1]],
                    Surface::Cylinder(c) => [along, uv[1] * c.frame().rows[2][2]],
                    Surface::Ruled(_) => [along, up(uv[1])],
                    Surface::Cone(_) | Surface::Torus(_) | Surface::Sphere(_) => return None,
                })
            }
            VertexSource::Vertex(id) => {
                let role = self.vertex_role(id)?;
                let slot = band.levels.iter().position(|&l| l == role.level)?;
                // Each vertex is the end of the segment it belongs to.
                let at = |segment| BoundarySegment {
                    boundary: band.boundary,
                    segment,
                };
                let u = if role.segment == at(k) {
                    1.0
                } else if role.segment == at(previous) {
                    0.0
                } else {
                    return None;
                };
                Some([u, band.heights[slot]])
            }
            VertexSource::Edge { edge, parameter } => {
                let e = edge.index();
                if e == band.edges[0] + k {
                    Some([parameter, low])
                } else if e == band.edges[1] + k {
                    Some([parameter, high])
                } else if e == band.seams + k {
                    Some([1.0, up(parameter)])
                } else if e == band.seams + previous {
                    Some([0.0, up(parameter)])
                } else {
                    None
                }
            }
        }
    }
}
