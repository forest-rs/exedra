// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;
use exedra_mesh::{FaceId, HalfEdgeId, Mesh, VertexId};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CaptureError {
    Selection(exedra_mesh::SelectedFacePatchError),
    NumericLimit { face: FaceId },
}

#[derive(Clone, Debug)]
pub(crate) struct CornerInput {
    pub(crate) corner: HalfEdgeId,
    pub(crate) vertex: VertexId,
    pub(crate) position: [f32; 3],
    pub(crate) uv: Option<[f32; 2]>,
    pub(crate) twin: HalfEdgeId,
    pub(crate) neighbor: FaceId,
    pub(crate) seam: Option<bool>,
    pub(crate) sharpness: Option<f32>,
}

// Captured floating-point input is compared by representation: even signed-zero
// changes must not reuse old cached corner attributes in an unfinished edit.
impl PartialEq for CornerInput {
    fn eq(&self, other: &Self) -> bool {
        self.corner == other.corner
            && self.vertex == other.vertex
            && self.position.map(f32::to_bits) == other.position.map(f32::to_bits)
            && self.uv.map(|uv| uv.map(f32::to_bits)) == other.uv.map(|uv| uv.map(f32::to_bits))
            && self.twin == other.twin
            && self.neighbor == other.neighbor
            && self.seam == other.seam
            && self.sharpness.map(f32::to_bits) == other.sharpness.map(f32::to_bits)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FaceInput {
    pub(crate) face: FaceId,
    pub(crate) region: u32,
    pub(crate) corners: Vec<CornerInput>,
}

pub(crate) fn capture_inputs(
    mesh: &Mesh,
    faces: &[FaceId],
) -> Result<Vec<FaceInput>, CaptureError> {
    mesh.selected_face_patch_topology(faces)
        .map_err(CaptureError::Selection)?;
    let mut inputs = Vec::with_capacity(faces.len());
    for &face in faces {
        let invalid = || {
            CaptureError::Selection(exedra_mesh::SelectedFacePatchError::InvalidFaceLoop { face })
        };
        let mut corners = Vec::new();
        for corner in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(corner).ok_or_else(invalid)?;
            let position = mesh.vertex_position(vertex).copied().ok_or_else(invalid)?;
            if !exedra_math::finite(position) {
                return Err(CaptureError::NumericLimit { face });
            }
            let twin = mesh.twin(corner).ok_or_else(invalid)?;
            let neighbor = mesh.face(twin).ok_or_else(invalid)?;
            let uv = mesh
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_UV)
                .and_then(|layer| layer.get(corner.as_id()).copied());
            let sharpness = mesh.edge_sharpness(corner);
            if uv.is_some_and(|uv| uv.iter().any(|v| !v.is_finite()))
                || sharpness.is_some_and(|v| !v.is_finite())
            {
                return Err(CaptureError::NumericLimit { face });
            }
            corners.push(CornerInput {
                corner,
                vertex,
                position,
                twin,
                neighbor,
                uv,
                seam: mesh.edge_seam(corner),
                sharpness,
            });
        }
        if corners.len() < 3 {
            return Err(invalid());
        }
        let region = mesh
            .attrs()
            .dense(exedra_mesh::attr::FACE_REGION)
            .and_then(|layer| layer.get(face.as_id()).copied())
            .unwrap_or(0);
        inputs.push(FaceInput {
            face,
            region,
            corners,
        });
    }
    Ok(inputs)
}
