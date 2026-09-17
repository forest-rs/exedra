// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Runtime adapter for read-only bounds inspection.

use crate::op_common::op_error;
use crate::{Artifacts, DiagCode, EditOperator, OpContext, OpError, OpErrorKind, OpReport};
pub use exedra_mesh_ops::inspect::{BoundsOutput, BoundsParams, BoundsScope, BoundsSummary};

/// Deterministic bounds inspection operator.
///
/// # Example
/// ```rust
/// use exedra_edit::{BoundsParams, InspectBounds, OperatorRunner};
/// use exedra_mesh::{BuildParams, Mesh};
///
/// let mut mesh = Mesh::from_indexed_triangles(
///     &[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
///     &[[0, 1, 2]],
///     &BuildParams::default(),
/// )
/// .expect("triangle build should succeed");
/// let mut runner = OperatorRunner::new();
/// let plan = runner
///     .compile(&mesh, &InspectBounds, &BoundsParams::default())
///     .expect("compile should succeed");
/// let result = runner
///     .apply_in_place(&mut mesh, &InspectBounds, &plan)
///     .expect("bounds should succeed");
/// let bounds = result.output.bounds.expect("triangle should have bounds");
/// assert_eq!(bounds.min, [0.0, 0.0, 0.0]);
/// assert_eq!(bounds.max, [2.0, 2.0, 0.0]);
/// ```
#[derive(Copy, Clone, Debug, Default)]
pub struct InspectBounds;

impl EditOperator for InspectBounds {
    type Params = BoundsParams;
    type Plan = BoundsParams;
    type Output = BoundsOutput;

    fn name(&self) -> &'static str {
        "inspect.bounds"
    }

    fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let output = exedra_mesh_ops::inspect::bounds(txn.mesh(), params).map_err(|error| {
            use exedra_mesh_ops::inspect::BoundsError as E;
            let (kind, code) = match &error {
                E::InvalidFace { .. } => (
                    OpErrorKind::PreconditionFailed,
                    DiagCode::PreconditionFailed,
                ),
                E::NumericLimit => (OpErrorKind::NumericFailure, DiagCode::NumericToleranceIssue),
                E::InvalidCorner { .. } | E::MissingPosition { .. } => (
                    OpErrorKind::InvalidMesh,
                    DiagCode::InternalInvariantViolation,
                ),
            };
            op_error(ctx, kind, code, alloc::format!("{error}"))
        })?;
        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        report.stats.elements_touched.vertices = output.vertex_count;
        report.stats.elements_touched.faces = output.face_count;
        report.stats.counters.faces_processed = output.face_count;
        report.stats.counters.selections_canonicalized = u64::from(output.selections_canonicalized);
        Ok((report, output))
    }

    fn compile(
        &self,
        _mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        _ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        Ok(params.clone())
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        self.apply(txn, plan, ctx)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use exedra_mesh::{BuildParams, FaceId, Id, Mesh};

    use super::{BoundsParams, BoundsScope, InspectBounds};
    use crate::{OpErrorKind, OperatorRunner, test_support::commit};

    #[test]
    fn bounds_empty_mesh_returns_none() {
        let mut mesh = Mesh::new();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &InspectBounds,
            &BoundsParams::default(),
        )
        .expect("bounds should succeed");
        assert!(result.output.bounds.is_none());
        assert_eq!(result.output.vertex_count, 0);
        assert_eq!(result.output.face_count, 0);
    }

    #[test]
    fn bounds_whole_mesh_triangle_matches_expected_values() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle build should succeed");
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &InspectBounds,
            &BoundsParams::default(),
        )
        .expect("bounds should succeed");
        let bounds = result.output.bounds.expect("bounds should exist");
        assert_eq!(bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(bounds.max, [2.0, 2.0, 0.0]);
        assert_eq!(bounds.centroid, [2.0 / 3.0, 2.0 / 3.0, 0.0]);
        assert!((bounds.diagonal - (8.0_f32).sqrt()).abs() < 1e-6);
        assert_eq!(result.output.vertex_count, 3);
        assert_eq!(result.output.face_count, 1);
    }

    #[test]
    fn bounds_face_set_uses_selected_faces_only() {
        let mut mesh = Mesh::from_indexed_triangles(
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
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &InspectBounds,
            &BoundsParams {
                scope: BoundsScope::FaceSet(vec![faces[1]]),
            },
        )
        .expect("bounds should succeed");
        let bounds = result.output.bounds.expect("bounds should exist");
        assert_eq!(bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(bounds.max, [1.0, 1.0, 0.0]);
        assert_eq!(result.output.vertex_count, 3);
        assert_eq!(result.output.face_count, 1);
    }

    #[test]
    fn bounds_face_set_multiple_faces_dedupes_vertices() {
        let mut mesh = Mesh::from_indexed_triangles(
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
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &InspectBounds,
            &BoundsParams {
                scope: BoundsScope::FaceSet(vec![faces[1], faces[0]]),
            },
        )
        .expect("bounds should succeed");
        let bounds = result.output.bounds.expect("bounds should exist");
        assert_eq!(bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(bounds.max, [1.0, 1.0, 0.0]);
        assert_eq!(bounds.centroid, [0.5, 0.5, 0.0]);
        assert!((bounds.diagonal - (2.0_f32).sqrt()).abs() < 1e-6);
        assert_eq!(result.output.vertex_count, 4);
        assert_eq!(result.output.face_count, 2);
    }

    #[test]
    fn bounds_face_set_rejects_stale_face() {
        let mut mesh = Mesh::new();
        let stale = FaceId::from(Id::new(999, NonZeroU32::MIN));
        let mut runner = OperatorRunner::new();
        let err = commit(
            &mut runner,
            &mut mesh,
            &InspectBounds,
            &BoundsParams {
                scope: BoundsScope::FaceSet(vec![stale]),
            },
        )
        .expect_err("stale face should fail");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }
}
