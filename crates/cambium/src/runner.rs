// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operator runner for compile/apply and preview execution.

use alloc::vec;
use alloc::vec::Vec;

use exedra::{ChangeSet, ChangeSetBuilder, Mesh, ValidationError};
#[cfg(all(not(target_arch = "wasm32"), feature = "std"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

#[cfg(any(target_arch = "wasm32", feature = "std"))]
use crate::timing::duration_nanos_u64;
use crate::{Diagnostic, EditOperator, EditPlan, OpContext, OpError, OpErrorKind, OpReport};

/// Result from [`OperatorRunner::apply_in_place`].
#[derive(Clone, Debug)]
pub struct OpResult<T = ()> {
    /// Recorded Exedra edit summary.
    pub change_set: ChangeSet,
    /// Operator report for this run.
    pub report: OpReport,
    /// Typed operator output.
    pub output: T,
}

/// Result from [`OperatorRunner::preview_on_clone`].
#[derive(Clone, Debug)]
pub struct PreviewResult<T = ()> {
    /// Preview mesh produced by running on a cloned base mesh.
    pub preview_mesh: Mesh,
    /// Operator report for this run.
    pub report: OpReport,
    /// Typed operator output.
    pub output: T,
}

/// Stateful runner owning reusable operator context.
///
/// This is the main entry point for executing Cambium operators.
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

    /// Compiles one operator plan from immutable mesh state.
    pub fn compile<O: EditOperator>(
        &mut self,
        mesh: &Mesh,
        op: &O,
        params: &O::Params,
    ) -> Result<EditPlan<O::Plan>, OpError> {
        self.reset_for_run();
        self.compile_with_ctx(mesh, op, params)
    }

    /// Applies one compiled operator plan in place.
    pub fn apply_in_place<O: EditOperator>(
        &mut self,
        mesh: &mut Mesh,
        op: &O,
        plan: &EditPlan<O::Plan>,
    ) -> Result<OpResult<O::Output>, OpError> {
        self.reset_for_run();
        self.apply_in_place_with_ctx(mesh, op, plan)
    }

    /// Applies one compiled operator plan to a cloned mesh.
    pub fn preview_on_clone<O: EditOperator>(
        &mut self,
        mesh: &Mesh,
        op: &O,
        plan: &EditPlan<O::Plan>,
    ) -> Result<PreviewResult<O::Output>, OpError> {
        self.reset_for_run();
        self.preview_on_clone_with_ctx(mesh, op, plan)
    }

    fn compile_with_ctx<O: EditOperator>(
        &mut self,
        mesh: &Mesh,
        op: &O,
        params: &O::Params,
    ) -> Result<EditPlan<O::Plan>, OpError> {
        #[cfg(any(target_arch = "wasm32", feature = "std"))]
        let compile_start = Instant::now();
        let payload = op
            .compile(mesh, params, &mut self.ctx)
            .map_err(|error| self.attach_context_diagnostics(error))?;
        #[cfg(any(target_arch = "wasm32", feature = "std"))]
        self.ctx
            .clock
            .add_nanos("op.compile", duration_nanos_u64(compile_start.elapsed()));
        #[cfg(not(any(target_arch = "wasm32", feature = "std")))]
        self.ctx.clock.add_nanos("op.compile", 0);

        Ok(EditPlan {
            operator: op.name(),
            fingerprint: op.plan_fingerprint(&payload),
            payload,
        })
    }

    fn apply_in_place_with_ctx<O: EditOperator>(
        &mut self,
        mesh: &mut Mesh,
        op: &O,
        plan: &EditPlan<O::Plan>,
    ) -> Result<OpResult<O::Output>, OpError> {
        if plan.operator != op.name() {
            return Err(self.plan_operator_mismatch_error(plan.operator, op.name()));
        }
        let mut txn = mesh.edit_with(ChangeSetBuilder::new());

        #[cfg(any(target_arch = "wasm32", feature = "std"))]
        let op_apply_start = Instant::now();
        let (mut report, output) = op
            .apply_plan(&mut txn, &plan.payload, &mut self.ctx)
            .map_err(|error| self.attach_context_diagnostics(error))?;
        #[cfg(any(target_arch = "wasm32", feature = "std"))]
        self.ctx
            .clock
            .add_nanos("op.apply", duration_nanos_u64(op_apply_start.elapsed()));
        #[cfg(not(any(target_arch = "wasm32", feature = "std")))]
        self.ctx.clock.add_nanos("op.apply", 0);
        let change_set = {
            let _bucket = self.ctx.clock.bucket("edit.finish");
            txn.finish()
        };
        report.stats.counters.deleted_vertices = u64::try_from(change_set.deleted_vertices.len())
            .expect("deleted vertex count should fit u64");
        self.ctx.cache_dirty.mark_from_change_set(&change_set);

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
        Ok(OpResult {
            change_set,
            report,
            output,
        })
    }

    fn preview_on_clone_with_ctx<O: EditOperator>(
        &mut self,
        mesh: &Mesh,
        op: &O,
        plan: &EditPlan<O::Plan>,
    ) -> Result<PreviewResult<O::Output>, OpError> {
        if plan.operator != op.name() {
            return Err(self.plan_operator_mismatch_error(plan.operator, op.name()));
        }
        let cache_dirty_before = self.ctx.cache_dirty.clone();
        let result = (|| {
            let mut preview_mesh = mesh.clone();
            let mut txn = preview_mesh.edit();

            #[cfg(any(target_arch = "wasm32", feature = "std"))]
            let op_apply_start = Instant::now();
            let (mut report, output) = op
                .apply_plan(&mut txn, &plan.payload, &mut self.ctx)
                .map_err(|error| self.attach_context_diagnostics(error))?;
            #[cfg(any(target_arch = "wasm32", feature = "std"))]
            self.ctx
                .clock
                .add_nanos("op.apply", duration_nanos_u64(op_apply_start.elapsed()));
            #[cfg(not(any(target_arch = "wasm32", feature = "std")))]
            self.ctx.clock.add_nanos("op.apply", 0);
            {
                let _bucket = self.ctx.clock.bucket("edit.finish");
                // Preview still runs the same eager edit path to produce final
                // timing/report data; the preview change recording is
                // intentionally discarded.
                let _: () = txn.finish();
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
                output,
            })
        })();
        self.ctx.cache_dirty = cache_dirty_before;
        result
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

    fn plan_operator_mismatch_error(
        &self,
        plan_operator: &str,
        expected_operator: &str,
    ) -> OpError {
        OpError::new(
            OpErrorKind::PreconditionFailed,
            vec![Diagnostic::new(
                crate::DiagLevel::Error,
                crate::DiagCode::PreconditionFailed,
                alloc::format!(
                    "plan operator mismatch: plan is `{plan_operator}`, runner expected `{expected_operator}`"
                ),
            )],
            crate::Artifacts::new(
                self.ctx.policy.limits.max_artifact_items,
                self.ctx.policy.limits.max_artifact_bytes,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use exedra::EditSession;

    use super::OperatorRunner;
    use crate::{Artifacts, DirtyChannel, EditOperator, OpContext, OpError, OpReport};

    struct AddVertexOperator;

    impl EditOperator for AddVertexOperator {
        type Params = [f32; 3];
        type Plan = [f32; 3];
        type Output = ();

        fn name(&self) -> &'static str {
            "test.add_vertex"
        }

        fn apply<S: exedra::ChangeSink>(
            &self,
            txn: &mut EditSession<'_, S>,
            params: &Self::Params,
            ctx: &mut OpContext,
        ) -> Result<(OpReport, Self::Output), OpError> {
            assert!(ctx.scratch.u32s.is_empty());
            ctx.scratch.u32s.push(7);
            let _ = exedra::op::add_vertex(txn, *params);
            Ok((OpReport::new(self.name(), Artifacts::default()), ()))
        }

        fn compile(
            &self,
            _mesh: &exedra::Mesh,
            params: &Self::Params,
            _ctx: &mut OpContext,
        ) -> Result<Self::Plan, OpError> {
            Ok(*params)
        }

        fn apply_plan<S: exedra::ChangeSink>(
            &self,
            txn: &mut EditSession<'_, S>,
            plan: &Self::Plan,
            ctx: &mut OpContext,
        ) -> Result<(OpReport, Self::Output), OpError> {
            self.apply(txn, plan, ctx)
        }
    }

    #[test]
    fn apply_in_place_mutates_mesh_and_returns_change_set() {
        let mut mesh = exedra::Mesh::new();
        let op = AddVertexOperator;
        let mut runner = OperatorRunner::new();
        let plan = runner
            .compile(&mesh, &op, &[1.0, 2.0, 3.0])
            .expect("compile should succeed");
        let result = runner
            .apply_in_place(&mut mesh, &op, &plan)
            .expect("apply should succeed");

        assert_eq!(mesh.vertices().count(), 1);
        assert_eq!(result.change_set.created_vertices.len(), 1);
        assert_eq!(result.report.name, "test.add_vertex");
        assert_eq!(result.output, ());
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
                .any(|bucket| bucket.name == "edit.finish")
        );
        let mut adjacency = Vec::new();
        runner
            .ctx
            .cache_dirty
            .drain_into(DirtyChannel::Adjacency, &mut adjacency);
        assert!(adjacency.contains(&crate::DirtyKey::Global));
        let mut op_cache = Vec::new();
        runner
            .ctx
            .cache_dirty
            .drain_into(DirtyChannel::OperatorCache, &mut op_cache);
        assert_eq!(op_cache, vec![crate::DirtyKey::Global]);
    }

    #[test]
    fn preview_on_clone_does_not_mutate_base_mesh() {
        let base = exedra::Mesh::new();
        let op = AddVertexOperator;
        let mut runner = OperatorRunner::new();
        let plan = runner
            .compile(&base, &op, &[4.0, 5.0, 6.0])
            .expect("compile should succeed");
        let result = runner
            .preview_on_clone(&base, &op, &plan)
            .expect("preview should succeed");

        assert_eq!(base.vertices().count(), 0);
        assert_eq!(result.preview_mesh.vertices().count(), 1);
    }

    #[test]
    fn scratch_is_cleared_before_each_run() {
        let mut mesh = exedra::Mesh::new();
        let op = AddVertexOperator;
        let mut runner = OperatorRunner::new();

        let plan_a = runner
            .compile(&mesh, &op, &[0.0, 0.0, 0.0])
            .expect("compile should succeed");
        let _ = runner
            .apply_in_place(&mut mesh, &op, &plan_a)
            .expect("first run should succeed");
        let plan_b = runner
            .compile(&mesh, &op, &[1.0, 0.0, 0.0])
            .expect("compile should succeed");
        let _ = runner
            .apply_in_place(&mut mesh, &op, &plan_b)
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
    fn preview_on_clone_with_validation_enabled_still_succeeds_for_valid_mesh() {
        let base = exedra::Mesh::new();
        let op = AddVertexOperator;
        let mut runner = OperatorRunner::new();
        runner.ctx.policy.validate.validate_on_preview = true;
        runner.ctx.policy.validate.fail_on_error = true;

        let plan = runner
            .compile(&base, &op, &[0.0, 0.0, 0.0])
            .expect("compile should succeed");
        let result = runner
            .preview_on_clone(&base, &op, &plan)
            .expect("preview on valid mesh should succeed");
        assert!(
            result
                .report
                .timings
                .iter()
                .any(|bucket| bucket.name == "validate")
        );
    }

    struct MarksPreviewDirtyOperator;

    impl EditOperator for MarksPreviewDirtyOperator {
        type Params = ();
        type Plan = ();
        type Output = ();

        fn name(&self) -> &'static str {
            "test.marks_preview_dirty"
        }

        fn apply<S: exedra::ChangeSink>(
            &self,
            _txn: &mut EditSession<'_, S>,
            _params: &Self::Params,
            ctx: &mut OpContext,
        ) -> Result<(OpReport, Self::Output), OpError> {
            ctx.cache_dirty.mark_global(DirtyChannel::Selection);
            Ok((OpReport::new(self.name(), Artifacts::default()), ()))
        }

        fn compile(
            &self,
            _mesh: &exedra::Mesh,
            _params: &Self::Params,
            _ctx: &mut OpContext,
        ) -> Result<Self::Plan, OpError> {
            Ok(())
        }

        fn apply_plan<S: exedra::ChangeSink>(
            &self,
            txn: &mut EditSession<'_, S>,
            _plan: &Self::Plan,
            ctx: &mut OpContext,
        ) -> Result<(OpReport, Self::Output), OpError> {
            self.apply(txn, &(), ctx)
        }
    }

    #[test]
    fn preview_on_clone_discards_cache_dirty_mutations() {
        let base = exedra::Mesh::new();
        let op = MarksPreviewDirtyOperator;
        let mut runner = OperatorRunner::new();
        let plan = runner
            .compile(&base, &op, &())
            .expect("compile should succeed");

        let result = runner
            .preview_on_clone(&base, &op, &plan)
            .expect("preview should succeed");
        let _ = result;
        assert!(!runner.ctx.cache_dirty.has_any(DirtyChannel::Selection));
    }
}
