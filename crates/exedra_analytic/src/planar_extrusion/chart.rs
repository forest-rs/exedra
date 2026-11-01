// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::PlanarExtrusion;
use crate::{
    body::{Face, FaceId},
    extrusion::FaceRole,
    realize::{RealizationError, RealizationPolicy, RealizedBody, VertexSource},
};
use alloc::vec::Vec;

/// Native realization or caller texture mapping could not be completed.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanarExtrusionUvError<E> {
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
impl<E: core::fmt::Display> core::fmt::Display for PlanarExtrusionUvError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Realization(e) => write!(f, "planar extrusion realization: {e}"),
            Self::Coordinates { face } => {
                write!(f, "planar extrusion coordinates unavailable on {face:?}")
            }
            Self::Mapping { face, error } => {
                write!(f, "planar extrusion mapping on {face:?}: {error}")
            }
            Self::NonFiniteUv { face } => write!(
                f,
                "planar extrusion mapping returned nonfinite UVs on {face:?}"
            ),
        }
    }
}
impl<E: core::error::Error + 'static> core::error::Error for PlanarExtrusionUvError<E> {}

impl PlanarExtrusion {
    /// Realizes with exact construction parameters and caller-owned texture policy.
    ///
    /// Caps receive local profile `(x,y)`. Walls receive `(u,z)`, where `u` is
    /// normalized traversal of that source segment and `z` is model-unit height.
    /// `u` is a curve parameter, **not arc length**; each segment has its own seam.
    /// These coordinates are independent of display tolerance and placement.
    /// Geometry vertices remain shared while corners can carry distinct UVs.
    ///
    /// # Errors
    /// Returns realization, coordinate, caller mapping or nonfinite UV failures.
    /// No partially mapped mesh is returned.
    pub fn realize_with_uvs<E>(
        &self,
        policy: &RealizationPolicy,
        region: impl Fn(FaceId, &Face<FaceRole>) -> u32,
        mut map: impl FnMut(FaceRole, [[f64; 2]; 3]) -> Result<[[f32; 2]; 3], E>,
    ) -> Result<RealizedBody, PlanarExtrusionUvError<E>> {
        let mut result = self
            .body
            .realize_with_regions(self.frame, policy, region)
            .map_err(PlanarExtrusionUvError::Realization)?;
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
                    .ok_or(PlanarExtrusionUvError::Coordinates { face })?;
            }
            let expected = if matches!(role, FaceRole::Cap(crate::End::Start)) {
                exedra_math::predicates::Orientation::Cw
            } else {
                exedra_math::predicates::Orientation::Ccw
            };
            if coordinates
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > exedra_math::predicates::MAX_COORDINATE)
                || exedra_math::predicates::orient2d(coordinates[0], coordinates[1], coordinates[2])
                    != expected
            {
                return Err(PlanarExtrusionUvError::Coordinates { face });
            }
            let uvs = map(role, coordinates)
                .map_err(|error| PlanarExtrusionUvError::Mapping { face, error })?;
            if uvs.iter().flatten().any(|v| !v.is_finite()) {
                return Err(PlanarExtrusionUvError::NonFiniteUv { face });
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
        if let VertexSource::Face { face: owner, uv } = source {
            return (owner == face).then(|| {
                if matches!(self.body.faces()[face.index()].role, FaceRole::Wall { .. }) {
                    [uv[0], uv[1] * self.length]
                } else {
                    uv
                }
            });
        }
        let f = &self.body.faces()[face.index()];
        if matches!(f.role, FaceRole::Cap(_)) {
            let p = match source {
                VertexSource::Vertex(id) => self.body.vertices()[id.index()],
                VertexSource::Edge { edge, parameter } => self.body.edge_point(edge, parameter)?,
                VertexSource::Face { .. } => return None,
            };
            return Some([p[0], p[1]]);
        }
        let ring = &self.body.loops()[f.loops[0].index()].coedges;
        let lower = self.body.edge(ring[0].edge)?;
        let upper = self.body.edge(ring[2].edge)?;
        let (a, b) = if ring[0].reversed {
            (lower.end, lower.start)
        } else {
            (lower.start, lower.end)
        };
        let (c, d) = if ring[2].reversed {
            (upper.start, upper.end)
        } else {
            (upper.end, upper.start)
        };
        match source {
            VertexSource::Vertex(id) => {
                if id == a {
                    Some([0.0, 0.0])
                } else if id == b {
                    Some([1.0, 0.0])
                } else if id == c {
                    Some([0.0, self.length])
                } else if id == d {
                    Some([1.0, self.length])
                } else {
                    None
                }
            }
            VertexSource::Edge { edge, parameter } => {
                if edge == ring[0].edge {
                    Some([
                        if ring[0].reversed {
                            1.0 - parameter
                        } else {
                            parameter
                        },
                        0.0,
                    ])
                } else if edge == ring[2].edge {
                    Some([
                        if ring[2].reversed {
                            parameter
                        } else {
                            1.0 - parameter
                        },
                        self.length,
                    ])
                } else if edge == ring[1].edge {
                    Some([1.0, parameter * self.length])
                } else if edge == ring[3].edge {
                    Some([0.0, parameter * self.length])
                } else {
                    None
                }
            }
            VertexSource::Face { .. } => None,
        }
    }
}
