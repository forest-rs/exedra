// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Boundary-loop bridge operator.

use alloc::format;

use crate::op_common::op_error;
use crate::{Artifacts, DiagCode, EditOperator, OpContext, OpError, OpErrorKind, OpReport};

pub use exedra_mesh_ops::bridge::{
    BridgeBoundaryLoopsOutput, BridgeBoundaryLoopsParams, BridgeBoundaryLoopsPlan,
};

/// `edit.bridge.loops` operator.
///
/// v0.1 scope:
/// - both inputs must be canonical boundary loops,
/// - both loops must have identical edge counts,
/// - loop pairing is deterministic and chosen by minimum geometric mismatch,
/// - generated bridge-strip boundaries are marked sharp by default.
#[derive(Copy, Clone, Debug, Default)]
pub struct BridgeBoundaryLoops;

impl EditOperator for BridgeBoundaryLoops {
    type Params = BridgeBoundaryLoopsParams;
    type Plan = BridgeBoundaryLoopsPlan;
    type Output = BridgeBoundaryLoopsOutput;

    fn name(&self) -> &'static str {
        "edit.bridge.loops"
    }

    fn compile(
        &self,
        mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        BridgeBoundaryLoopsPlan::prepare(mesh, params).map_err(|error| bridge_error(ctx, error))
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let (stats, output) = plan.apply(txn).map_err(|error| bridge_error(ctx, error))?;
        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        report.stats.counters.selections_canonicalized = u64::from(stats.selections_canonicalized);
        report.stats.elements_touched.half_edges = stats.boundary_edges;
        report.stats.elements_created.faces = output.bridge_faces.len() as u64;
        if stats.reversed_winding {
            ctx.diagnostics.push(crate::Diagnostic::new(crate::DiagLevel::Warn, DiagCode::PreconditionFailed, "bridge: frame winding fallback to reverse orientation due to boundary reuse direction"));
        }
        Ok((report, output))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        let mut hasher = crate::plan::PlanHasher::new();
        hasher.write_str(self.name());
        hasher.write_len(plan.loop_a().len());
        for edge in plan.loop_a() {
            hasher.write_u32(edge.index());
        }
        hasher.write_len(plan.loop_b().len());
        for edge in plan.loop_b() {
            hasher.write_u32(edge.index());
        }
        for vertex in plan.aligned_a_vertices() {
            hasher.write_u32(vertex.index());
        }
        for vertex in plan.aligned_b_vertices() {
            hasher.write_u32(vertex.index());
        }
        hasher.write_u8(u8::from(plan.selections_canonicalized()));
        hasher.finish()
    }
}

fn bridge_error(ctx: &OpContext, error: exedra_mesh_ops::bridge::BridgeError) -> OpError {
    use exedra_mesh_ops::bridge::BridgeError as E;
    let (kind, code) = match &error {
        E::InvalidBoundary { .. } | E::InvalidGeneratedFace | E::SourceSelection(_) => (
            OpErrorKind::InvalidMesh,
            DiagCode::InternalInvariantViolation,
        ),
        E::AddFace(_) => (
            OpErrorKind::InternalInvariantViolation,
            DiagCode::InternalInvariantViolation,
        ),
        E::NumericLimit => (OpErrorKind::NumericFailure, DiagCode::NumericToleranceIssue),
        _ => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
        ),
    };
    op_error(ctx, kind, code, format!("{error}"))
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use exedra_mesh::Mesh;

    use super::{BridgeBoundaryLoops, BridgeBoundaryLoopsParams};
    use crate::{
        MeshEdit, OperatorRunner, SelectBoundaryEdgeLoop, SelectBoundaryEdgeLoopParams,
        test_support::commit,
    };

    fn two_parallel_quads() -> (Mesh, exedra_mesh::FaceId, exedra_mesh::FaceId) {
        let mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ],
            &[&[0, 1, 2, 3], &[4, 7, 6, 5]],
        )
        .expect("parallel quads should build");
        let faces = mesh.faces().collect::<Vec<_>>();
        (mesh, faces[0], faces[1])
    }

    #[test]
    fn bridge_parallel_quads_creates_quad_strip() {
        let (mut mesh, face_a, face_b) = two_parallel_quads();
        let mut runner = OperatorRunner::new();
        let seed_a = mesh.face_edge(face_a).expect("face edge should exist");
        let seed_b = mesh.face_edge(face_b).expect("face edge should exist");
        let loop_a = commit(
            &mut runner,
            &mut mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams { seed_edge: seed_a },
        )
        .expect("boundary select should succeed")
        .output;
        let loop_b = commit(
            &mut runner,
            &mut mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams { seed_edge: seed_b },
        )
        .expect("boundary select should succeed")
        .output;

        let result = commit(
            &mut runner,
            &mut mesh,
            &BridgeBoundaryLoops,
            &BridgeBoundaryLoopsParams { loop_a, loop_b },
        )
        .expect("bridge should succeed");
        assert_eq!(result.output.bridge_faces.len(), 4);
        assert_eq!(mesh.faces().count(), 6);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn bridge_rejects_unequal_length_loops() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [4.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3], &[4, 5, 6]],
        )
        .expect("mixed loops should build");
        let faces = mesh.faces().collect::<Vec<_>>();
        let mut runner = OperatorRunner::new();
        let seed_a = mesh.face_edge(faces[0]).expect("face edge should exist");
        let seed_b = mesh.face_edge(faces[1]).expect("face edge should exist");
        let loop_a = commit(
            &mut runner,
            &mut mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams { seed_edge: seed_a },
        )
        .expect("boundary select should succeed")
        .output;
        let loop_b = commit(
            &mut runner,
            &mut mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams { seed_edge: seed_b },
        )
        .expect("boundary select should succeed")
        .output;

        let err = runner
            .compile(
                &mesh,
                &BridgeBoundaryLoops,
                &BridgeBoundaryLoopsParams { loop_a, loop_b },
            )
            .expect_err("unequal loops should fail");
        assert_eq!(err.kind, crate::OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn mesh_edit_bridge_matches_direct_operator() {
        let (mut direct_mesh, face_a, face_b) = two_parallel_quads();
        let mut fluent_mesh = direct_mesh.clone();
        let seed_a = direct_mesh
            .face_edge(face_a)
            .expect("face edge should exist");
        let seed_b = direct_mesh
            .face_edge(face_b)
            .expect("face edge should exist");
        let loop_a = crate::select_boundary_edge_loop(&direct_mesh, seed_a)
            .expect("loop query should succeed")
            .edges;
        let loop_b = crate::select_boundary_edge_loop(&direct_mesh, seed_b)
            .expect("loop query should succeed")
            .edges;

        let mut direct_runner = OperatorRunner::new();
        let plan = direct_runner
            .compile(
                &direct_mesh,
                &BridgeBoundaryLoops,
                &BridgeBoundaryLoopsParams {
                    loop_a: loop_a.clone(),
                    loop_b: loop_b.clone(),
                },
            )
            .expect("bridge compile should succeed");
        let _ = direct_runner
            .apply_in_place(&mut direct_mesh, &BridgeBoundaryLoops, &plan)
            .expect("bridge apply should succeed");

        let flow = MeshEdit::new()
            .select(crate::Selection::from(loop_a))
            .bridge_boundary_loops(loop_b);
        let mut fluent_runner = OperatorRunner::new();
        let _ = flow
            .apply(&mut fluent_runner, &mut fluent_mesh)
            .expect("fluent bridge should succeed");

        assert_eq!(
            crate::mesh_signature(&direct_mesh),
            crate::mesh_signature(&fluent_mesh)
        );
    }
}
