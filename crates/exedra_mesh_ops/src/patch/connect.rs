// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_mesh::op::AddFaceError;
use exedra_mesh::{FaceId, VertexId, op};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum FrameWinding {
    UseForwardOuterEdge,
    UseReverseOuterEdge,
}

#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct FrameOrientationState {
    winding: Option<FrameWinding>,
}

impl FrameOrientationState {
    pub(crate) const fn prefers_forward_outer_edge(self) -> bool {
        matches!(self.winding, Some(FrameWinding::UseForwardOuterEdge))
    }
}

pub(crate) fn add_frame_face_with_orientation<S: exedra_mesh::ChangeSink>(
    txn: &mut exedra_mesh::EditSession<'_, S>,
    current: VertexId,
    next: VertexId,
    current_inner: VertexId,
    next_inner: VertexId,
    orientation: &mut FrameOrientationState,
) -> Result<FaceId, AddFaceError> {
    let forward_outer = [current, next, next_inner, current_inner];
    let reverse_outer = [next, current, current_inner, next_inner];
    match orientation.winding {
        Some(FrameWinding::UseForwardOuterEdge) => op::add_face(txn, &forward_outer),
        Some(FrameWinding::UseReverseOuterEdge) => op::add_face(txn, &reverse_outer),
        None => match op::add_face(txn, &forward_outer) {
            Ok(face) => {
                orientation.winding = Some(FrameWinding::UseForwardOuterEdge);
                Ok(face)
            }
            Err(AddFaceError::NonManifoldEdge { .. }) => {
                let face = op::add_face(txn, &reverse_outer)?;
                orientation.winding = Some(FrameWinding::UseReverseOuterEdge);
                Ok(face)
            }
            Err(err) => Err(err),
        },
    }
}
