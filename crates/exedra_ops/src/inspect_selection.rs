// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Selection summary inspection operator.

use crate::{
    Artifact, Artifacts, EditOperator, FaceSet, OpContext, OpError, OpReport, Selection,
    SelectionKind,
};

/// Summary detail level for [`InspectSelectionSummary`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum SelectionSummaryDetail {
    /// Count only liveness and domain totals.
    #[default]
    Basic,
    /// Include extra lightweight topology checks (for example edge canonicality).
    Topology,
}

/// Parameters for [`InspectSelectionSummary`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionSummaryParams {
    /// Input selection to summarize.
    pub selection: Selection,
    /// Summary detail level.
    pub detail: SelectionSummaryDetail,
}

impl Default for SelectionSummaryParams {
    fn default() -> Self {
        Self {
            selection: Selection::from(FaceSet::new()),
            detail: SelectionSummaryDetail::Basic,
        }
    }
}

/// Typed output from [`InspectSelectionSummary`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SelectionSummaryOutput {
    /// Selection domain.
    pub kind: SelectionKind,
    /// Canonicalized item count.
    pub item_count: u64,
    /// Number of live IDs in the mesh.
    pub live_count: u64,
    /// Number of stale IDs in the selection.
    pub stale_count: u64,
    /// Number of `FaceId::OUTSIDE` entries (face-domain only).
    pub outside_count: u64,
    /// Number of non-canonical undirected edge IDs (edge-domain + topology detail).
    pub non_canonical_edge_count: u64,
    /// True when no stale IDs were found.
    pub all_live: bool,
}

impl Default for SelectionSummaryOutput {
    fn default() -> Self {
        Self {
            kind: SelectionKind::Faces,
            item_count: 0,
            live_count: 0,
            stale_count: 0,
            outside_count: 0,
            non_canonical_edge_count: 0,
            all_live: true,
        }
    }
}

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

    fn apply<S: exedra::ChangeSink>(
        &self,
        txn: &mut exedra::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let mut selection = params.selection.clone();
        let canonicalized = selection.canonicalize();

        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        if canonicalized {
            report.stats.counters.selections_canonicalized = 1;
        }

        let mut output = SelectionSummaryOutput {
            kind: selection.kind(),
            ..SelectionSummaryOutput::default()
        };

        match &selection {
            Selection::Faces(faces) => {
                output.item_count = u64::try_from(faces.len()).expect("face count should fit u64");
                report.stats.elements_touched.faces = output.item_count;
                report.stats.counters.faces_processed = output.item_count;
                for &face in faces {
                    if face == exedra::FaceId::OUTSIDE {
                        output.outside_count = output.outside_count.saturating_add(1);
                        continue;
                    }
                    if txn.mesh().face_edge(face).is_some() {
                        output.live_count = output.live_count.saturating_add(1);
                    } else {
                        output.stale_count = output.stale_count.saturating_add(1);
                    }
                }
                let _ = report.artifacts.push(Artifact::FaceSet {
                    name: self.name().into(),
                    faces: faces.clone(),
                });
            }
            Selection::Edges(edges) => {
                output.item_count = u64::try_from(edges.len()).expect("edge count should fit u64");
                report.stats.elements_touched.half_edges = output.item_count;
                for &edge in edges {
                    if txn.mesh().twin(edge).is_some() {
                        output.live_count = output.live_count.saturating_add(1);
                        if params.detail == SelectionSummaryDetail::Topology
                            && txn
                                .mesh()
                                .canonical_edge(edge)
                                .is_some_and(|canonical| canonical != edge)
                        {
                            output.non_canonical_edge_count =
                                output.non_canonical_edge_count.saturating_add(1);
                        }
                    } else {
                        output.stale_count = output.stale_count.saturating_add(1);
                    }
                }
                let _ = report.artifacts.push(Artifact::EdgeSet {
                    name: self.name().into(),
                    half_edges: edges.clone(),
                });
            }
            Selection::Vertices(vertices) => {
                output.item_count =
                    u64::try_from(vertices.len()).expect("vertex count should fit u64");
                report.stats.elements_touched.vertices = output.item_count;
                for &vertex in vertices {
                    if txn.mesh().vertex_position(vertex).is_some() {
                        output.live_count = output.live_count.saturating_add(1);
                    } else {
                        output.stale_count = output.stale_count.saturating_add(1);
                    }
                }
            }
        }

        output.all_live = output.stale_count == 0;
        Ok((report, output))
    }

    fn compile(
        &self,
        _mesh: &exedra::Mesh,
        params: &Self::Params,
        _ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        Ok(params.clone())
    }

    fn apply_plan<S: exedra::ChangeSink>(
        &self,
        txn: &mut exedra::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        self.apply(txn, plan, ctx)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use exedra::{BuildParams, FaceId, HalfEdgeId, Id, Mesh, VertexId};

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
