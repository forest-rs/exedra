// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operator runner for preview/commit execution.

use alloc::vec::Vec;

use exedra::{ChangeSet, Mesh, ValidationError};
#[cfg(all(not(target_arch = "wasm32"), feature = "std"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

#[cfg(any(target_arch = "wasm32", feature = "std"))]
use crate::timing::duration_nanos_u64;
use crate::{Diagnostic, EditOperator, OpContext, OpError, OpReport};

/// Result from a committed operator run.
#[derive(Clone, Debug)]
pub struct OpResult {
    /// Transaction commit summary.
    pub change_set: ChangeSet,
    /// Operator report for this run.
    pub report: OpReport,
}

/// Result from a preview operator run.
#[derive(Clone, Debug)]
pub struct PreviewResult {
    /// Preview mesh produced by running on a cloned base mesh.
    pub preview_mesh: Mesh,
    /// Operator report for this run.
    pub report: OpReport,
}

/// Stateful runner owning reusable operator context.
#[derive(Debug, Default)]
pub struct OperatorRunner {
    /// Reusable execution context.
    pub ctx: OpContext,
}

impl OperatorRunner {
    /// Creates a runner with default context.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one operator in commit mode.
    ///
    /// Semantics:
    /// - mesh mutations are committed before optional post-commit validation
    /// - when `validate.fail_on_error` is enabled, this can return `Err` after
    ///   commit; in that case the attached `OpError` carries the committed
    ///   `change_set` for reconciliation
    pub fn run_commit<O: EditOperator>(
        &mut self,
        mesh: &mut Mesh,
        op: &O,
        params: &O::Params,
    ) -> Result<OpResult, OpError> {
        self.reset_for_run();
        let mut txn = mesh.begin();

        #[cfg(any(target_arch = "wasm32", feature = "std"))]
        let op_apply_start = Instant::now();
        let mut report = {
            match op.apply(&mut txn, params, &mut self.ctx) {
                Ok(report) => report,
                Err(error) => return Err(self.attach_context_diagnostics(error)),
            }
        };
        #[cfg(any(target_arch = "wasm32", feature = "std"))]
        self.ctx
            .clock
            .add_nanos("op.apply", duration_nanos_u64(op_apply_start.elapsed()));
        #[cfg(not(any(target_arch = "wasm32", feature = "std")))]
        self.ctx.clock.add_nanos("op.apply", 0);
        let change_set = {
            let _bucket = self.ctx.clock.bucket("txn.commit");
            txn.commit()
        };

        if self.ctx.policy.validate.validate_on_commit {
            let errors = {
                let _bucket = self.ctx.clock.bucket("validate");
                mesh.validate_deep()
            };
            if !errors.is_empty() && self.ctx.policy.validate.fail_on_error {
                return Err(self.post_commit_validation_error(&errors, &report, change_set));
            }
        }

        report.timings = self.ctx.clock.timings();
        Ok(OpResult { change_set, report })
    }

    /// Runs one operator in preview mode against a cloned mesh.
    pub fn run_preview<O: EditOperator>(
        &mut self,
        mesh: &Mesh,
        op: &O,
        params: &O::Params,
    ) -> Result<PreviewResult, OpError> {
        self.reset_for_run();
        let mut preview_mesh = mesh.clone();
        let mut txn = preview_mesh.begin();

        #[cfg(any(target_arch = "wasm32", feature = "std"))]
        let op_apply_start = Instant::now();
        let mut report = {
            match op.apply(&mut txn, params, &mut self.ctx) {
                Ok(report) => report,
                Err(error) => return Err(self.attach_context_diagnostics(error)),
            }
        };
        #[cfg(any(target_arch = "wasm32", feature = "std"))]
        self.ctx
            .clock
            .add_nanos("op.apply", duration_nanos_u64(op_apply_start.elapsed()));
        #[cfg(not(any(target_arch = "wasm32", feature = "std")))]
        self.ctx.clock.add_nanos("op.apply", 0);
        {
            let _bucket = self.ctx.clock.bucket("txn.commit");
            // Preview still runs the same transaction path to produce report
            // timings; the preview change-set is intentionally discarded.
            let _ = txn.commit();
        }

        if self.ctx.policy.validate.validate_on_preview {
            let errors = {
                let _bucket = self.ctx.clock.bucket("validate");
                preview_mesh.validate_deep()
            };
            if !errors.is_empty() && self.ctx.policy.validate.fail_on_error {
                return Err(OpError::from_validation_errors(
                    &errors,
                    self.context_diagnostics_snapshot(),
                    report.artifacts.clone(),
                ));
            }
        }

        report.timings = self.ctx.clock.timings();
        Ok(PreviewResult {
            preview_mesh,
            report,
        })
    }

    fn reset_for_run(&mut self) {
        self.ctx.scratch.clear();
        self.ctx.diagnostics = crate::DiagnosticsSink::new(self.ctx.policy.limits.max_diagnostics);
        self.ctx.clock = crate::Clock::default();
    }

    fn attach_context_diagnostics(&self, mut error: OpError) -> OpError {
        error
            .diagnostics
            .extend(self.context_diagnostics_snapshot());
        error
    }

    fn context_diagnostics_snapshot(&self) -> Vec<Diagnostic> {
        self.ctx.diagnostics.iter().cloned().collect()
    }

    fn post_commit_validation_error(
        &self,
        errors: &[ValidationError],
        report: &OpReport,
        change_set: ChangeSet,
    ) -> OpError {
        OpError::from_validation_errors(
            errors,
            self.context_diagnostics_snapshot(),
            report.artifacts.clone(),
        )
        .with_change_set(change_set)
    }
}

#[cfg(test)]
mod tests {
    use exedra::Txn;

    use super::OperatorRunner;
    use crate::{Artifacts, EditOperator, OpContext, OpError, OpReport};

    struct AddVertexOperator;

    impl EditOperator for AddVertexOperator {
        type Params = [f32; 3];

        fn name(&self) -> &'static str {
            "test.add_vertex"
        }

        fn apply(
            &self,
            txn: &mut Txn<'_>,
            params: &Self::Params,
            ctx: &mut OpContext,
        ) -> Result<OpReport, OpError> {
            assert!(ctx.scratch.u32s.is_empty());
            ctx.scratch.u32s.push(7);
            let _ = txn.add_vertex(*params);
            Ok(OpReport::new(self.name(), Artifacts::default()))
        }
    }

    #[test]
    fn run_commit_mutates_mesh_and_returns_change_set() {
        let mut mesh = exedra::Mesh::new();
        let op = AddVertexOperator;
        let mut runner = OperatorRunner::new();
        let result = runner
            .run_commit(&mut mesh, &op, &[1.0, 2.0, 3.0])
            .expect("commit should succeed");

        assert_eq!(mesh.vertices().count(), 1);
        assert_eq!(result.change_set.created_vertices.len(), 1);
        assert_eq!(result.report.name, "test.add_vertex");
        assert!(
            result
                .report
                .timings
                .iter()
                .any(|bucket| bucket.name == "op.apply")
        );
        assert!(
            result
                .report
                .timings
                .iter()
                .any(|bucket| bucket.name == "txn.commit")
        );
    }

    #[test]
    fn run_preview_does_not_mutate_base_mesh() {
        let base = exedra::Mesh::new();
        let op = AddVertexOperator;
        let mut runner = OperatorRunner::new();
        let result = runner
            .run_preview(&base, &op, &[4.0, 5.0, 6.0])
            .expect("preview should succeed");

        assert_eq!(base.vertices().count(), 0);
        assert_eq!(result.preview_mesh.vertices().count(), 1);
    }

    #[test]
    fn scratch_is_cleared_before_each_run() {
        let mut mesh = exedra::Mesh::new();
        let op = AddVertexOperator;
        let mut runner = OperatorRunner::new();

        let _ = runner
            .run_commit(&mut mesh, &op, &[0.0, 0.0, 0.0])
            .expect("first run should succeed");
        let _ = runner
            .run_commit(&mut mesh, &op, &[1.0, 0.0, 0.0])
            .expect("second run should succeed");
    }

    #[test]
    fn post_commit_validation_error_carries_change_set() {
        let runner = OperatorRunner::new();
        let report = OpReport::new("test", Artifacts::default());
        let change_set = exedra::ChangeSet::default();
        let error = runner.post_commit_validation_error(
            &[exedra::ValidationError::FaceLoopNotClosed { face: 0 }],
            &report,
            change_set,
        );
        assert!(error.change_set.is_some());
    }

    #[test]
    fn run_preview_with_validation_enabled_still_succeeds_for_valid_mesh() {
        let base = exedra::Mesh::new();
        let op = AddVertexOperator;
        let mut runner = OperatorRunner::new();
        runner.ctx.policy.validate.validate_on_preview = true;
        runner.ctx.policy.validate.fail_on_error = true;

        let result = runner
            .run_preview(&base, &op, &[0.0, 0.0, 0.0])
            .expect("preview on valid mesh should succeed");
        assert!(
            result
                .report
                .timings
                .iter()
                .any(|bucket| bucket.name == "validate")
        );
    }
}
