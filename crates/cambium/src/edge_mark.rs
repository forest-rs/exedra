// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared implementation for edge-boolean tagging operators.

use crate::selection::{EdgeSet, canonicalize_edge_set};
use crate::{
    Artifacts, DiagCode, DiagLevel, Diagnostic, OpContext, OpError, OpErrorKind, OpReport,
};

pub(crate) fn apply_edge_bool_tag<F>(
    txn: &mut exedra::Txn<'_>,
    edges: &EdgeSet,
    value: bool,
    ctx: &mut OpContext,
    op_name: &'static str,
    error_message: &'static str,
    mut setter: F,
) -> Result<OpReport, OpError>
where
    F: FnMut(&mut exedra::Txn<'_>, exedra::HalfEdgeId, bool) -> bool,
{
    let mut canonical_input = edges.clone();
    let mut report = OpReport::new(
        op_name,
        Artifacts::new(
            ctx.policy.limits.max_artifact_items,
            ctx.policy.limits.max_artifact_bytes,
        ),
    );
    if canonicalize_edge_set(&mut canonical_input) {
        report.stats.counters.selections_canonicalized = report
            .stats
            .counters
            .selections_canonicalized
            .saturating_add(1);
    }

    // Canonicalize against mesh topology so either half-edge of a pair maps
    // to one deterministic edge write.
    let mut canonical_topology = EdgeSet::new();
    for edge in canonical_input {
        let Some(canonical_edge) = txn.mesh().canonical_edge(edge) else {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                error_message,
            ));
        };
        canonical_topology.push(canonical_edge);
    }
    if canonicalize_edge_set(&mut canonical_topology) {
        report.stats.counters.selections_canonicalized = report
            .stats
            .counters
            .selections_canonicalized
            .saturating_add(1);
    }

    for edge in canonical_topology {
        if !setter(txn, edge, value) {
            debug_assert!(
                false,
                "edge bool setter failed after canonical-edge validation"
            );
            return Err(op_error(
                ctx,
                OpErrorKind::InternalInvariantViolation,
                DiagCode::InternalInvariantViolation,
                "edge bool setter failed after canonical-edge validation",
            ));
        }
        // One canonical sparse edge slot is written per surviving edge.
        report.stats.counters.edges_written = report.stats.counters.edges_written.saturating_add(1);
    }

    Ok(report)
}

pub(crate) fn op_error(
    ctx: &OpContext,
    kind: OpErrorKind,
    code: DiagCode,
    message: &'static str,
) -> OpError {
    OpError::new(
        kind,
        alloc::vec![Diagnostic::new(DiagLevel::Error, code, message)],
        Artifacts::new(
            ctx.policy.limits.max_artifact_items,
            ctx.policy.limits.max_artifact_bytes,
        ),
    )
}
