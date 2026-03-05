// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Edge sharpness tagging operator.
//!
//! `mark.edge.sharp` writes explicit `f32` sharpness values to selected
//! undirected edges:
//! - `0.0` means smooth,
//! - values above `0.0` mean authored sharpness.

use crate::edge_mark::apply_edge_tag;
use crate::selection::EdgeSet;
use crate::{EditOperator, OpContext, OpError, OpReport};

/// Parameters for [`MarkEdgeSharp`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkEdgeSharpParams {
    /// Edge half-edges to mark/unmark.
    pub edges: EdgeSet,
    /// Target edge sharpness value (`0.0` is smooth).
    pub sharp: f32,
}

/// Marks or clears explicit edge sharpness tags.
#[derive(Copy, Clone, Debug, Default)]
pub struct MarkEdgeSharp;

impl EditOperator for MarkEdgeSharp {
    type Params = MarkEdgeSharpParams;
    type Plan = MarkEdgeSharpParams;
    type Output = EdgeSet;

    fn name(&self) -> &'static str {
        "mark.edge.sharp"
    }

    fn apply(
        &self,
        txn: &mut exedra::EditSession<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        apply_edge_tag(
            txn,
            &params.edges,
            params.sharp,
            ctx,
            self.name(),
            "edge set contains invalid/stale half-edge id",
            |txn, edge, sharp| txn.set_edge_sharpness(edge, sharp),
        )
    }

    fn compile(
        &self,
        _mesh: &exedra::Mesh,
        params: &Self::Params,
        _ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        Ok(params.clone())
    }

    fn apply_plan(
        &self,
        txn: &mut exedra::EditSession<'_>,
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

    use super::{MarkEdgeSharp, MarkEdgeSharpParams};
    use crate::{
        EditOperator, OperatorRunner,
        test_support::{commit, shared_edge_mesh},
    };

    #[test]
    fn mark_edge_sharp_sets_and_clears_tag() {
        let (mut mesh, shared, twin) = shared_edge_mesh();
        let mut runner = OperatorRunner::new();
        let op = MarkEdgeSharp;

        let set_params = MarkEdgeSharpParams {
            edges: vec![twin],
            sharp: 2.5,
        };
        let mut set_result = commit(&mut runner, &mut mesh, &op, &set_params)
            .expect("mark edge sharp should succeed");
        assert_eq!(set_result.report.name, op.name());
        assert_eq!(mesh.edge_sharpness(shared), Some(2.5));
        assert_eq!(mesh.edge_sharpness(twin), Some(2.5));
        let mut dirty = Vec::new();
        set_result.change_set.dirty.drain_corners_into(&mut dirty);
        assert_eq!(dirty.len(), 2);

        let clear_params = MarkEdgeSharpParams {
            edges: vec![shared],
            sharp: 0.0,
        };
        let _ = commit(&mut runner, &mut mesh, &op, &clear_params)
            .expect("clear edge sharp should succeed");
        assert_eq!(mesh.edge_sharpness(shared), Some(0.0));
        assert_eq!(mesh.edge_sharpness(twin), Some(0.0));
    }

    #[test]
    fn mark_edge_sharp_canonicalizes_duplicate_selection() {
        let (mut mesh, shared, twin) = shared_edge_mesh();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &MarkEdgeSharp,
            &MarkEdgeSharpParams {
                edges: vec![shared, twin, shared],
                sharp: 1.0,
            },
        )
        .expect("mark edge sharp should succeed");
        assert_eq!(result.report.stats.counters.selections_canonicalized, 2);
        assert_eq!(result.report.stats.counters.edges_written, 1);
        assert_eq!(result.report.stats.counters.corners_written, 0);
    }
}
