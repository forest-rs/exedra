// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Runtime adapter for read-only selection inspection.

use crate::{Artifact, Artifacts, EditOperator, OpContext, OpError, OpReport, Selection};
pub use exedra_mesh_ops::inspect::{
    SelectionSummaryDetail, SelectionSummaryOutput, SelectionSummaryParams,
};

/// `inspect.select.summary` operator.
#[derive(Copy, Clone, Debug, Default)]
pub struct InspectSelectionSummary;

impl EditOperator for InspectSelectionSummary {
    type Params = SelectionSummaryParams;
    type Plan = SelectionSummaryParams;
    type Output = SelectionSummaryOutput;

    fn name(&self) -> &'static str {
        "inspect.select.summary"
    }

    fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let inspection = exedra_mesh_ops::inspect::selection_summary(txn.mesh(), params);
        let output = inspection.summary;
        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        report.stats.counters.selections_canonicalized =
            u64::from(inspection.selections_canonicalized);
        match inspection.selection {
            Selection::Faces(faces) => {
                report.stats.elements_touched.faces = output.item_count;
                report.stats.counters.faces_processed = output.item_count;
                let _ = report.artifacts.push(Artifact::FaceSet {
                    name: self.name().into(),
                    faces,
                });
            }
            Selection::Edges(half_edges) => {
                report.stats.elements_touched.half_edges = output.item_count;
                let _ = report.artifacts.push(Artifact::EdgeSet {
                    name: self.name().into(),
                    half_edges,
                });
            }
            Selection::Vertices(_) => report.stats.elements_touched.vertices = output.item_count,
        }
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

    use exedra_mesh::{BuildParams, FaceId, HalfEdgeId, Id, Mesh, VertexId};

    use super::{
        InspectSelectionSummary, SelectionSummaryDetail, SelectionSummaryOutput,
        SelectionSummaryParams,
    };
    use crate::{OperatorRunner, Selection, SelectionKind, mesh_signature, test_support::commit};

    fn one_quad_mesh() -> (Mesh, FaceId, HalfEdgeId, VertexId) {
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
        let edge = mesh
            .face_loop(face)
            .next()
            .expect("face loop should provide one edge");
        let vertex = mesh
            .to_vertex(edge)
            .expect("edge destination vertex should exist");
        (mesh, face, edge, vertex)
    }

    #[test]
    fn inspect_selection_summary_faces_and_empty() {
        let (mut mesh, face, _, _) = one_quad_mesh();
        let mut runner = OperatorRunner::new();
        let op = InspectSelectionSummary;

        let result = commit(
            &mut runner,
            &mut mesh,
            &op,
            &SelectionSummaryParams {
                selection: Selection::from(vec![face]),
                detail: SelectionSummaryDetail::Basic,
            },
        )
        .expect("summary should succeed");
        assert_eq!(result.output.kind, SelectionKind::Faces);
        assert_eq!(result.output.live_count, 1);
        assert_eq!(result.output.stale_count, 0);
        assert!(result.output.all_live);
        assert!(
            result
                .report
                .artifacts
                .iter()
                .any(|artifact| artifact.name() == "inspect.select.summary")
        );

        let empty = commit(
            &mut runner,
            &mut mesh,
            &op,
            &SelectionSummaryParams::default(),
        )
        .expect("empty summary should succeed");
        assert_eq!(empty.output.item_count, 0);
    }

    #[test]
    fn inspect_selection_summary_edges_reports_noncanonical_with_topology_detail() {
        let (mut mesh, _, edge, _) = one_quad_mesh();
        let mut runner = OperatorRunner::new();
        let twin = mesh.twin(edge).expect("edge should have twin");
        let result = commit(
            &mut runner,
            &mut mesh,
            &InspectSelectionSummary,
            &SelectionSummaryParams {
                selection: Selection::from(vec![twin]),
                detail: SelectionSummaryDetail::Topology,
            },
        )
        .expect("edge summary should succeed");
        assert_eq!(result.output.kind, SelectionKind::Edges);
        assert_eq!(result.output.item_count, 1);
        assert_eq!(result.output.live_count, 1);
        assert_eq!(result.output.non_canonical_edge_count, 1);
    }

    #[test]
    fn inspect_selection_summary_vertices_reports_stale() {
        let (mut mesh, _, _, vertex) = one_quad_mesh();
        let mut runner = OperatorRunner::new();
        let stale = VertexId::from(Id::new(999, core::num::NonZeroU32::MIN));
        let result = commit(
            &mut runner,
            &mut mesh,
            &InspectSelectionSummary,
            &SelectionSummaryParams {
                selection: Selection::from(vec![vertex, stale]),
                detail: SelectionSummaryDetail::Basic,
            },
        )
        .expect("vertex summary should succeed");
        assert_eq!(result.output.kind, SelectionKind::Vertices);
        assert_eq!(result.output.live_count, 1);
        assert_eq!(result.output.stale_count, 1);
        assert!(!result.output.all_live);
    }

    #[test]
    fn inspect_selection_summary_does_not_mutate_mesh() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle build should succeed");
        let sig_before = mesh_signature(&mesh);
        let mut runner = OperatorRunner::new();
        let _ = commit(
            &mut runner,
            &mut mesh,
            &InspectSelectionSummary,
            &SelectionSummaryParams {
                selection: Selection::from(vec![FaceId::OUTSIDE]),
                detail: SelectionSummaryDetail::Basic,
            },
        )
        .expect("summary should succeed");
        let sig_after = mesh_signature(&mesh);
        assert_eq!(sig_before, sig_after);
    }

    #[test]
    fn inspect_selection_summary_output_defaults_are_stable() {
        let output = SelectionSummaryOutput::default();
        assert_eq!(output.kind, SelectionKind::Faces);
        assert_eq!(output.item_count, 0);
        assert_eq!(output.live_count, 0);
        assert_eq!(output.stale_count, 0);
        assert_eq!(output.outside_count, 0);
        assert_eq!(output.non_canonical_edge_count, 0);
        assert!(output.all_live);
    }

    #[test]
    fn inspect_selection_summary_accepts_stale_edge_without_error() {
        let (mut mesh, _, edge, _) = one_quad_mesh();
        let mut runner = OperatorRunner::new();
        let stale = HalfEdgeId::from(Id::new(edge.index() + 10_000, core::num::NonZeroU32::MIN));
        let result = commit(
            &mut runner,
            &mut mesh,
            &InspectSelectionSummary,
            &SelectionSummaryParams {
                selection: Selection::from(vec![edge, stale]),
                detail: SelectionSummaryDetail::Basic,
            },
        )
        .expect("stale edge summary should still succeed");
        assert_eq!(result.output.stale_count, 1);
    }
}
