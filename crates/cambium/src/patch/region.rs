// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::vec::Vec;

use exedra::{FaceId, HalfEdgeId, VertexId};

use crate::op_common::op_error;
use crate::patch::attrs::SourceEdgeAttrs;
use crate::patch::geom::normalized_face_normal;
use crate::{DiagCode, OpContext, OpError, OpErrorKind};

#[derive(Clone, Debug)]
pub(crate) struct SelectedFace {
    pub(crate) face: FaceId,
    pub(crate) vertices: Vec<VertexId>,
    pub(crate) vertex_uvs: Vec<Option<[f32; 2]>>,
    pub(crate) edge_attrs: Vec<SourceEdgeAttrs>,
    pub(crate) normal: [f32; 3],
    pub(crate) region: u32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) struct FaceEdgeRef {
    pub(crate) face: FaceId,
    pub(crate) edge: HalfEdgeId,
    pub(crate) from: VertexId,
    pub(crate) to: VertexId,
    pub(crate) edge_index: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct SharedRegionEdge {
    pub(crate) key: (VertexId, VertexId),
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "consumed by downstream loop/adjacency helpers")
    )]
    pub(crate) incidences: Vec<FaceEdgeRef>,
}

#[derive(Clone, Debug)]
pub(crate) struct SelectedFaceRegion {
    pub(crate) faces: Vec<SelectedFace>,
    pub(crate) boundary_edges: Vec<FaceEdgeRef>,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "consumed by downstream loop/duplication helpers")
    )]
    pub(crate) interior_edges: Vec<SharedRegionEdge>,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "consumed by downstream duplication helpers")
    )]
    pub(crate) incident_vertices: Vec<VertexId>,
    pub(crate) boundary_lies_on_mesh_boundary: bool,
}

pub(crate) fn selected_face_region(
    mesh: &exedra::Mesh,
    faces: &[FaceId],
    require_normal: bool,
    ctx: &OpContext,
) -> Result<SelectedFaceRegion, OpError> {
    let mut selected_faces = Vec::<SelectedFace>::with_capacity(faces.len());
    let mut incident_vertices = BTreeSet::<VertexId>::new();
    let mut edge_membership = BTreeMap::<(VertexId, VertexId), Vec<FaceEdgeRef>>::new();

    for &face in faces {
        if face == FaceId::OUTSIDE {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                "selection contains FaceId::OUTSIDE",
            ));
        }
        if mesh.face_edge(face).is_none() {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                format!("selection contains stale face id: {}", face.index()),
            ));
        }

        let mut vertices = Vec::<VertexId>::new();
        let mut corners = Vec::<HalfEdgeId>::new();
        for corner in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(corner).ok_or_else(|| {
                op_error(
                    ctx,
                    OpErrorKind::InvalidMesh,
                    DiagCode::InternalInvariantViolation,
                    "face loop contains corner with missing destination vertex",
                )
            })?;
            incident_vertices.insert(vertex);
            vertices.push(vertex);
            corners.push(corner);
        }
        if vertices.len() < 3 {
            return Err(op_error(
                ctx,
                OpErrorKind::InvalidMesh,
                DiagCode::InternalInvariantViolation,
                "face loop degree is less than 3",
            ));
        }

        let normal = if require_normal {
            normalized_face_normal(mesh, &vertices).ok_or_else(|| {
                op_error(
                    ctx,
                    OpErrorKind::NumericFailure,
                    DiagCode::NumericToleranceIssue,
                    format!("cannot extrude degenerate face {}", face.index()),
                )
            })?
        } else {
            [0.0, 0.0, 0.0]
        };
        let region = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .and_then(|layer| layer.get(face.as_id()).copied())
            .unwrap_or(0);
        let mut vertex_uvs = Vec::with_capacity(corners.len());
        for &corner in &corners {
            let uv = mesh
                .attrs()
                .sparse(exedra::attr::CORNER_UV)
                .and_then(|layer| layer.get(corner.as_id()).copied());
            vertex_uvs.push(uv);
        }
        let mut edge_attrs = Vec::with_capacity(vertices.len());
        for i in 0..vertices.len() {
            let edge_corner = corners[(i + 1) % corners.len()];
            edge_attrs.push(SourceEdgeAttrs {
                seam: mesh.edge_seam(edge_corner),
                sharpness: mesh.edge_sharpness(edge_corner),
            });
            let from = vertices[i];
            let to = vertices[(i + 1) % vertices.len()];
            let key = canonical_edge_key(from, to);
            edge_membership.entry(key).or_default().push(FaceEdgeRef {
                face,
                edge: edge_corner,
                from,
                to,
                edge_index: i,
            });
        }

        selected_faces.push(SelectedFace {
            face,
            vertices,
            vertex_uvs,
            edge_attrs,
            normal,
            region,
        });
    }

    let mut boundary_edges = Vec::<FaceEdgeRef>::new();
    let mut interior_edges = Vec::<SharedRegionEdge>::new();
    for (key, incidences) in edge_membership {
        if incidences.len() == 1 {
            boundary_edges.push(incidences[0]);
        } else {
            interior_edges.push(SharedRegionEdge { key, incidences });
        }
    }
    boundary_edges.sort_by_key(|edge| (edge.face.index(), edge.edge_index));
    interior_edges.sort_by_key(|edge| edge.key);

    let boundary_lies_on_mesh_boundary = !boundary_edges.is_empty()
        && boundary_edges.iter().all(|boundary| {
            mesh.twin(boundary.edge)
                .and_then(|twin| mesh.face(twin))
                .is_some_and(|face| face == FaceId::OUTSIDE)
        });

    Ok(SelectedFaceRegion {
        faces: selected_faces,
        boundary_edges,
        interior_edges,
        incident_vertices: incident_vertices.into_iter().collect(),
        boundary_lies_on_mesh_boundary,
    })
}

pub(crate) fn canonical_edge_key(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use exedra::{BuildParams, Mesh};

    use super::selected_face_region;
    use crate::OpContext;

    #[test]
    fn region_single_face_has_only_boundary_edges() {
        let mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let ctx = OpContext::default();

        let region = selected_face_region(&mesh, &[face], true, &ctx).expect("region should load");
        assert_eq!(region.faces.len(), 1);
        assert_eq!(region.boundary_edges.len(), 4);
        assert!(region.interior_edges.is_empty());
        assert_eq!(region.incident_vertices.len(), 4);
        assert!(region.boundary_lies_on_mesh_boundary);
    }

    #[test]
    fn region_adjacent_faces_classify_shared_edge_as_interior() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let ctx = OpContext::default();

        let region = selected_face_region(&mesh, &faces, false, &ctx).expect("region should load");
        assert_eq!(region.boundary_edges.len(), 4);
        assert_eq!(region.interior_edges.len(), 1);
        assert_eq!(region.interior_edges[0].incidences.len(), 2);
        assert_eq!(region.incident_vertices.len(), 4);
        assert!(region.boundary_lies_on_mesh_boundary);
    }

    #[test]
    fn region_disjoint_faces_keep_disjoint_boundary_membership() {
        let mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [4.0, 1.0, 0.0],
                [3.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [4, 5, 6]],
            &BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let ctx = OpContext::default();

        let region = selected_face_region(&mesh, &faces, false, &ctx).expect("region should load");
        assert_eq!(region.faces.len(), 2);
        assert_eq!(region.boundary_edges.len(), 6);
        assert!(region.interior_edges.is_empty());
        assert_eq!(region.incident_vertices.len(), 6);
    }

    #[test]
    fn region_closed_volume_boundary_is_not_open_surface() {
        let mesh = Mesh::from_polygons(
            &[
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            &[
                &[0, 1, 2, 3],
                &[4, 7, 6, 5],
                &[0, 4, 5, 1],
                &[1, 5, 6, 2],
                &[2, 6, 7, 3],
                &[3, 7, 4, 0],
            ],
        )
        .expect("cube build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let ctx = OpContext::default();

        let region = selected_face_region(&mesh, &faces, false, &ctx).expect("region should load");
        assert!(region.boundary_edges.is_empty());
        assert!(!region.boundary_lies_on_mesh_boundary);
    }
}
