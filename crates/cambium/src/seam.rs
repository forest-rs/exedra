// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Edge seam tagging operator.

use crate::edge_mark::apply_edge_bool_tag;
use crate::selection::EdgeSet;
use crate::{EditOperator, OpContext, OpError, OpReport};

/// Parameters for [`MarkEdgeSeam`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MarkEdgeSeamParams {
    /// Edge half-edges to mark/unmark.
    pub edges: EdgeSet,
    /// Target seam state.
    pub seam: bool,
}

/// Marks or clears explicit edge seam tags.
#[derive(Copy, Clone, Debug, Default)]
pub struct MarkEdgeSeam;

impl EditOperator for MarkEdgeSeam {
    type Params = MarkEdgeSeamParams;

    fn name(&self) -> &'static str {
        "mark.edge.seam"
    }

    fn apply(
        &self,
        txn: &mut exedra::Txn<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<OpReport, OpError> {
        apply_edge_bool_tag(
            txn,
            &params.edges,
            params.seam,
            ctx,
            self.name(),
            "edge set contains invalid/stale half-edge id",
            |txn, edge, seam| txn.set_edge_seam(edge, seam),
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{MarkEdgeSeam, MarkEdgeSeamParams};
    use crate::{EditOperator, OperatorRunner, test_support::shared_edge_mesh};

    #[test]
    fn mark_edge_seam_sets_and_clears_tag() {
        let (mut mesh, shared, twin) = shared_edge_mesh();
        let mut runner = OperatorRunner::new();
        let op = MarkEdgeSeam;

        let set_params = MarkEdgeSeamParams {
            edges: vec![twin],
            seam: true,
        };
        let mut set_result = runner
            .run_commit(&mut mesh, &op, &set_params)
            .expect("mark edge seam should succeed");
        assert_eq!(set_result.report.name, op.name());
        assert_eq!(mesh.edge_seam(shared), Some(true));
        assert_eq!(mesh.edge_seam(twin), Some(true));
        let mut dirty = Vec::new();
        set_result.change_set.dirty.drain_corners_into(&mut dirty);
        assert_eq!(dirty.len(), 2);

        let clear_params = MarkEdgeSeamParams {
            edges: vec![shared],
            seam: false,
        };
        let _ = runner
            .run_commit(&mut mesh, &op, &clear_params)
            .expect("clear edge seam should succeed");
        assert_eq!(mesh.edge_seam(shared), Some(false));
        assert_eq!(mesh.edge_seam(twin), Some(false));
    }

    #[test]
    fn mark_edge_seam_canonicalizes_duplicate_selection() {
        let (mut mesh, shared, twin) = shared_edge_mesh();
        let mut runner = OperatorRunner::new();
        let result = runner
            .run_commit(
                &mut mesh,
                &MarkEdgeSeam,
                &MarkEdgeSeamParams {
                    edges: vec![shared, twin, shared],
                    seam: true,
                },
            )
            .expect("mark edge seam should succeed");
        assert_eq!(result.report.stats.counters.selections_canonicalized, 2);
        assert_eq!(result.report.stats.counters.corners_written, 1);
    }
}
