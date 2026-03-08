// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::format;

use exedra::op::AddFaceError;
use exedra::{FaceId, VertexId, op};

use crate::op_common::op_error;
use crate::{DiagCode, DiagLevel, Diagnostic, OpContext, OpError, OpErrorKind};

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

pub(crate) fn add_frame_face_with_orientation<S: exedra::ChangeSink>(
    txn: &mut exedra::EditSession<'_, S>,
    current: VertexId,
    next: VertexId,
    current_inner: VertexId,
    next_inner: VertexId,
    orientation: &mut FrameOrientationState,
    ctx: &mut OpContext,
    op_name: &'static str,
) -> Result<FaceId, OpError> {
    let forward_outer = [current, next, next_inner, current_inner];
    let reverse_outer = [next, current, current_inner, next_inner];
    match orientation.winding {
        Some(FrameWinding::UseForwardOuterEdge) => {
            op::add_face(txn, &forward_outer).map_err(|err| frame_face_error(ctx, op_name, err))
        }
        Some(FrameWinding::UseReverseOuterEdge) => {
            op::add_face(txn, &reverse_outer).map_err(|err| frame_face_error(ctx, op_name, err))
        }
        None => match op::add_face(txn, &forward_outer) {
            Ok(face) => {
                orientation.winding = Some(FrameWinding::UseForwardOuterEdge);
                Ok(face)
            }
            Err(AddFaceError::NonManifoldEdge { .. }) => {
                ctx.diagnostics.push(Diagnostic::new(
                    DiagLevel::Warn,
                    DiagCode::PreconditionFailed,
                    format!(
                        "{op_name}: frame winding fallback to reverse orientation due to boundary reuse direction"
                    ),
                ));
                let face = op::add_face(txn, &reverse_outer)
                    .map_err(|err| frame_face_error(ctx, op_name, err))?;
                orientation.winding = Some(FrameWinding::UseReverseOuterEdge);
                Ok(face)
            }
            Err(err) => Err(frame_face_error(ctx, op_name, err)),
        },
    }
}

fn frame_face_error(ctx: &OpContext, op_name: &'static str, err: AddFaceError) -> OpError {
    op_error(
        ctx,
        OpErrorKind::InternalInvariantViolation,
        DiagCode::InternalInvariantViolation,
        format!("{op_name} frame face creation failed unexpectedly: {err}"),
    )
}
