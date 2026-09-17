// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Runtime reporting for direct UV projection.

use crate::{
    Artifact, Artifacts, DiagCode, DiagLevel, Diagnostic, FaceSet, OpContext, OpError, OpErrorKind,
    OpReport,
};
use alloc::format;
use exedra_mesh_ops::uv::{UvError, UvOutput};

pub(crate) fn projection_error(ctx: &OpContext, error: UvError) -> OpError {
    let (kind, code) = match &error {
        UvError::InvalidParameters | UvError::NumericLimit => {
            (OpErrorKind::NumericFailure, DiagCode::NumericToleranceIssue)
        }
        UvError::InvalidFace { .. } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
        ),
        UvError::InvalidCorner { .. } | UvError::Write(_) => (
            OpErrorKind::InvalidMesh,
            DiagCode::InternalInvariantViolation,
        ),
    };
    crate::op_common::op_error(ctx, kind, code, format!("{error}"))
}

pub(crate) fn projection_report(
    ctx: &mut OpContext,
    name: &'static str,
    output: UvOutput,
) -> (OpReport, FaceSet) {
    let mut report = OpReport::new(
        name,
        Artifacts::new(
            ctx.policy.limits.max_artifact_items,
            ctx.policy.limits.max_artifact_bytes,
        ),
    );
    report.stats.counters.faces_processed = output.faces.len() as u64;
    report.stats.counters.corners_written = output.corners_written;
    report.stats.counters.corners_skipped_existing = output.corners_skipped_existing;
    report.stats.counters.selections_canonicalized = u64::from(output.selections_canonicalized);
    for face in output.fallback_faces {
        let fallback = if name == "uv.box" {
            "+Z plane"
        } else {
            "WorldXY"
        };
        ctx.diagnostics.push(Diagnostic::new(
            DiagLevel::Warn,
            DiagCode::NumericToleranceIssue,
            format!(
                "{name} face {} has degenerate normal; falling back to {fallback}",
                face.index()
            ),
        ));
    }
    if !output.faces.is_empty() {
        let _ = report.artifacts.push(Artifact::FaceSet {
            name: format!("{name}.affected_faces"),
            faces: output.faces.clone(),
        });
    }
    (report, output.faces)
}
