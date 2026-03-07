// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operator Authoring Guide
//!
//! Operators implement [`EditOperator`](crate::EditOperator) and mutate mesh
//! state through an in-flight [`exedra::EditSession`]. The runner owns
//! apply/preview orchestration.
//!
//! Lifecycle:
//! 1. Runner resets reusable context (`scratch`, diagnostics sink, clock).
//! 2. Runner compiles deterministic operator intent into an [`EditPlan`](crate::EditPlan).
//! 3. Runner applies the plan to a clone (`preview_on_clone`) or in place
//!    (`apply_in_place`).
//! 4. Runner attaches timings and performs optional post-run validation.
//!
//! # Reporting Discipline
//!
//! Keep reporting explicit and deterministic:
//! - Timings: use named phases through [`Clock::bucket`](crate::Clock::bucket).
//! - Stats: increment [`SmallCounters`](crate::SmallCounters) and element
//!   counts as work is performed.
//! - Artifacts: emit bounded debug payloads through
//!   [`OpReport::artifacts`](crate::OpReport::artifacts).
//! - Diagnostics: emit structured messages via
//!   [`DiagnosticsSink`](crate::DiagnosticsSink).
//!
//! Suggested timing bucket names:
//! - `select`: canonicalization and selection derivation
//! - `compute`: geometric/topological computation
//! - `attrs`: attribute writes
//! - `validate`: optional invariant checks
//!
//! # Error Semantics
//!
//! - Prefer structured [`OpError`](crate::OpError) with precise
//!   [`OpErrorKind`](crate::OpErrorKind).
//! - Use diagnostics for caller-actionable detail.
//! - Treat stale IDs and missing required layers as precondition failures.
//! - Exedra edit scopes are eager: mesh edits can already be applied before a
//!   runner post-apply validation error is returned.
//!
//! # Determinism Rules
//!
//! - Canonicalize face/edge selections before processing.
//! - Avoid nondeterministic iteration order in externally visible outputs.
//! - Keep artifact ordering stable.
//! - Keep timing bucket names stable across runs and versions.
//!
//! # Namespace Rules
//!
//! Stable operator IDs (`name()`) must use one of the v0.1 family prefixes:
//! - `inspect.*`
//! - `select.*`
//! - `tag.*`
//! - `mark.*`
//! - `uv.*`
//! - `edit.*`
//!
//! # Edge Sharpness Conventions
//!
//! Edge sharpness is numeric (`f32`) in Exedra/Cambium:
//! - `0.0` means smooth,
//! - values above `0.0` are authored sharpness.
//!
//! Operator-facing write path:
//! - [`MarkEdgeSharp`](crate::MarkEdgeSharp) and
//!   [`MarkEdgeSharpParams`](crate::MarkEdgeSharpParams).
//!
//! Transaction propagation behavior for split kernels is controlled by
//! [`exedra::EdgeAttrPropagation`]:
//! - `Inherit` preserves sharpness,
//! - `DecayOnSplit` applies subdivision-style decay,
//! - `Clear` resets to `0.0`.
//!
//! # Face-Edit Winding Contract
//!
//! Face-edit operators (`extrude`/`inset`) build frame quads against newly
//! created boundary rings after source-face deletion. Winding is resolved once
//! per operator invocation and then reused for all generated frame faces:
//! - try reverse-outer orientation first,
//! - if Exedra preflight reports `NonManifoldEdge`, switch to forward-outer
//!   orientation and keep that orientation for the rest of the run.
//!
//! This keeps orientation deterministic across open and closed manifold
//! contexts and avoids per-face heuristic flipping.
//!
//! # Skeleton
//!
//! ```rust
//! use cambium::{Artifacts, EditOperator, OpContext, OpError, OpReport};
//!
//! #[derive(Copy, Clone, Debug, Default)]
//! struct ExampleOp;
//!
//! #[derive(Copy, Clone, Debug, Default)]
//! struct ExampleParams;
//!
//! impl EditOperator for ExampleOp {
//!     type Params = ExampleParams;
//!     type Plan = ExampleParams;
//!     type Output = ();
//!
//!     fn name(&self) -> &'static str {
//!         "example.op"
//!     }
//!
//!     fn apply<S: exedra::ChangeSink>(
//!         &self,
//!         txn: &mut exedra::EditSession<'_, S>,
//!         _params: &Self::Params,
//!         ctx: &mut OpContext,
//!     ) -> Result<(OpReport, Self::Output), OpError> {
//!         let mut report = OpReport::new(
//!             self.name(),
//!             Artifacts::new(
//!                 ctx.policy.limits.max_artifact_items,
//!                 ctx.policy.limits.max_artifact_bytes,
//!             ),
//!         );
//!
//!         {
//!             let _bucket = ctx.clock.bucket("select");
//!             // Canonicalize / derive deterministic workset.
//!         }
//!
//!         {
//!             let _bucket = ctx.clock.bucket("compute");
//!             // Perform work; update stats as you go.
//!             report.stats.counters.faces_processed += 1;
//!         }
//!
//!         {
//!             let _bucket = ctx.clock.bucket("attrs");
//!             // Write through Exedra kernel ops.
//!             let _ = exedra::op::add_vertex(txn, [0.0, 0.0, 0.0]);
//!         }
//!
//!         Ok((report, ()))
//!     }
//!
//!     fn compile(
//!         &self,
//!         _mesh: &exedra::Mesh,
//!         params: &Self::Params,
//!         _ctx: &mut OpContext,
//!     ) -> Result<Self::Plan, OpError> {
//!         Ok(*params)
//!     }
//!
//!     fn apply_plan<S: exedra::ChangeSink>(
//!         &self,
//!         txn: &mut exedra::EditSession<'_, S>,
//!         plan: &Self::Plan,
//!         ctx: &mut OpContext,
//!     ) -> Result<(OpReport, Self::Output), OpError> {
//!         self.apply(txn, plan, ctx)
//!     }
//! }
//! ```
//!
//! # Review Checklist
//!
//! - Are all selection inputs canonicalized?
//! - Are timings bucketed for major phases?
//! - Do counters reflect actual performed work?
//! - Are artifact emissions bounded and deterministic?
//! - Are diagnostics actionable and classified correctly?
//! - Do tests cover success + failure + preview/commit expectations?
