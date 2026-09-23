// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use exedra_mesh::attributes::CapturedAttributes;
use exedra_mesh::{FaceId, HalfEdgeId, VertexId};

use crate::face_edit::FaceEditError;
use crate::patch::attrs::SourceEdgeAttrs;
use crate::patch::geom::normalized_face_normal;

#[derive(Clone, Debug)]
pub(crate) struct SelectedFace {
    pub(crate) face: FaceId,
    pub(crate) vertices: Vec<VertexId>,
    pub(crate) vertex_uvs: Vec<Option<[f32; 2]>>,
    /// Caller-defined corner values, parallel to `vertices`.
    pub(crate) corner_layers: Vec<CapturedAttributes>,
    /// Caller-defined face values.
    pub(crate) face_layers: CapturedAttributes,
    pub(crate) edge_attrs: Vec<SourceEdgeAttrs>,
    pub(crate) normal: [f32; 3],
    pub(crate) region: u32,
}

pub(crate) type FaceEdgeRef = exedra_mesh::SelectedFacePatchEdge;

#[derive(Clone, Debug)]
pub(crate) struct SelectedFaceRegion {
    pub(crate) faces: Vec<SelectedFace>,
    pub(crate) boundary_edges: Vec<FaceEdgeRef>,
    pub(crate) boundary_lies_on_mesh_boundary: bool,
}

pub(crate) fn selected_face_region(
    mesh: &exedra_mesh::Mesh,
    faces: &[FaceId],
    require_normal: bool,
) -> Result<SelectedFaceRegion, FaceEditError> {
    let _ = crate::patch::source::capture_inputs(mesh, faces)?;
    let mut selected_faces = Vec::<SelectedFace>::with_capacity(faces.len());
    let patch = mesh
        .selected_face_patch_topology(faces)
        .map_err(FaceEditError::Selection)?;

    for &face in faces {
        let mut vertices = Vec::<VertexId>::new();
        let mut corners = Vec::<HalfEdgeId>::new();
        for corner in mesh.face_loop(face) {
            let vertex = mesh
                .to_vertex(corner)
                .ok_or(FaceEditError::InvalidFace { face })?;
            vertices.push(vertex);
            corners.push(corner);
        }
        if vertices.len() < 3 {
            return Err(FaceEditError::InvalidFace { face });
        }

        let normal = if require_normal {
            normalized_face_normal(mesh, &vertices).ok_or(FaceEditError::DegenerateFace { face })?
        } else {
            [0.0, 0.0, 0.0]
        };
        let region = mesh
            .attrs()
            .dense(exedra_mesh::attr::FACE_REGION)
            .and_then(|layer| layer.get(face.as_id()).copied())
            .unwrap_or(0);
        let mut vertex_uvs = Vec::with_capacity(corners.len());
        for &corner in &corners {
            let uv = mesh
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_UV)
                .and_then(|layer| layer.get(corner.as_id()).copied());
            vertex_uvs.push(uv);
        }
        let corner_layers = corners
            .iter()
            .map(|&corner| mesh.capture_attributes(&[(corner, 1.0)]))
            .collect();
        let face_layers = mesh.capture_attributes(&[(face, 1.0)]);
        let mut edge_attrs = Vec::with_capacity(vertices.len());
        for i in 0..vertices.len() {
            let edge_corner = corners[(i + 1) % corners.len()];
            edge_attrs.push(SourceEdgeAttrs {
                seam: mesh.edge_seam(edge_corner),
                sharpness: mesh.edge_sharpness(edge_corner),
            });
        }

        selected_faces.push(SelectedFace {
            face,
            vertices,
            vertex_uvs,
            corner_layers,
            face_layers,
            edge_attrs,
            normal,
            region,
        });
    }

    Ok(SelectedFaceRegion {
        faces: selected_faces,
        boundary_edges: patch.boundary_edges,
        boundary_lies_on_mesh_boundary: patch.boundary_lies_on_mesh_boundary,
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use exedra_mesh::{BuildParams, Mesh};

    use super::selected_face_region;

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

        let region = selected_face_region(&mesh, &[face], true).expect("region should load");
        assert_eq!(region.faces.len(), 1);
        assert_eq!(region.boundary_edges.len(), 4);
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

        let region = selected_face_region(&mesh, &faces, false).expect("region should load");
        assert_eq!(region.boundary_edges.len(), 4);
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

        let region = selected_face_region(&mesh, &faces, false).expect("region should load");
        assert_eq!(region.faces.len(), 2);
        assert_eq!(region.boundary_edges.len(), 6);
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

        let region = selected_face_region(&mesh, &faces, false).expect("region should load");
        assert!(region.boundary_edges.is_empty());
        assert!(!region.boundary_lies_on_mesh_boundary);
    }
}
