// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared helpers for deterministic UV projection operators.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use exedra::{CornerId, FaceId, Mesh};

use crate::{
    Artifacts, DiagCode, DiagLevel, Diagnostic, OpError, OpErrorKind, canonicalize_face_set,
    uv_planar::UvScope,
};

pub(crate) struct SelectedFaces {
    pub(crate) faces: Vec<FaceId>,
    pub(crate) changed: bool,
}

pub(crate) fn select_faces(mesh: &Mesh, scope: &UvScope) -> SelectedFaces {
    match scope {
        UvScope::WholeMesh => SelectedFaces {
            // TODO(cam-v5ko): avoid allocation by streaming whole-mesh traversal.
            faces: mesh.faces().collect::<Vec<_>>(),
            changed: false,
        },
        UvScope::FaceSet(values) => {
            let mut faces = values.clone();
            let changed = canonicalize_face_set(&mut faces);
            SelectedFaces { faces, changed }
        }
    }
}

pub(crate) fn corner_position(mesh: &Mesh, corner: CornerId) -> [f32; 3] {
    let vertex = mesh
        .from_vertex(corner)
        .expect("face loop corner must have source vertex");
    *mesh
        .vertex_position(vertex)
        .expect("live vertex must have builtin position")
}

pub(crate) fn face_normal(mesh: &Mesh, face: FaceId) -> Option<[f32; 3]> {
    let mut corners = mesh.face_loop(face);
    let first = corners.next()?;
    let second = corners.next()?;

    let p0 = corner_position(mesh, first);
    let mut prev = corner_position(mesh, second);
    let mut seen = 2_usize;
    let mut nx = 0.0_f32;
    let mut ny = 0.0_f32;
    let mut nz = 0.0_f32;
    for corner in corners {
        seen = seen.saturating_add(1);
        let curr = corner_position(mesh, corner);
        let ax = prev[0] - p0[0];
        let ay = prev[1] - p0[1];
        let az = prev[2] - p0[2];
        let bx = curr[0] - p0[0];
        let by = curr[1] - p0[1];
        let bz = curr[2] - p0[2];
        nx += ay * bz - az * by;
        ny += az * bx - ax * bz;
        nz += ax * by - ay * bx;
        prev = curr;
    }
    if seen < 3 { None } else { Some([nx, ny, nz]) }
}

pub(crate) fn stale_face_error(op_name: &str, face: FaceId, artifacts: Artifacts) -> OpError {
    OpError::new(
        OpErrorKind::PreconditionFailed,
        vec![Diagnostic::new(
            DiagLevel::Error,
            DiagCode::PreconditionFailed,
            format!("{op_name} received stale face id: {}", face.index()),
        )],
        artifacts,
    )
}
