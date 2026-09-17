// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared implementation for edge tagging operators.

use crate::op_common::op_error;
use crate::selection::EdgeSet;
use crate::{Artifacts, DiagCode, OpContext, OpError, OpErrorKind, OpReport};

pub(crate) fn apply_edge_tag<S, T, F>(
    txn: &mut exedra_mesh::EditSession<'_, S>,
    edges: &EdgeSet,
    value: T,
    ctx: &mut OpContext,
    op_name: &'static str,
    error_message: &'static str,
    mut setter: F,
) -> Result<(OpReport, EdgeSet), OpError>
where
    S: exedra_mesh::ChangeSink,
    T: Copy,
    F: FnMut(&mut exedra_mesh::EditSession<'_, S>, exedra_mesh::HalfEdgeId, T) -> bool,
{
    let resolved =
        exedra_mesh_ops::selection::resolve_edge_set(txn.mesh(), edges).map_err(|_| {
            op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                error_message,
            )
        })?;
    let mut report = OpReport::new(
        op_name,
        Artifacts::new(
            ctx.policy.limits.max_artifact_items,
            ctx.policy.limits.max_artifact_bytes,
        ),
    );
    report.stats.counters.selections_canonicalized = u64::from(resolved.canonicalization_steps);
    let canonical_topology = resolved.edges;

    for edge in canonical_topology.iter().copied() {
        if !setter(txn, edge, value) {
            debug_assert!(false, "edge setter failed after canonical-edge validation");
            return Err(op_error(
                ctx,
                OpErrorKind::InternalInvariantViolation,
                DiagCode::InternalInvariantViolation,
                "edge setter failed after canonical-edge validation",
            ));
        }
        // One canonical sparse edge slot is written per surviving edge.
        report.stats.counters.edges_written = report.stats.counters.edges_written.saturating_add(1);
    }

    Ok((report, canonical_topology))
}
