// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! SDK Surface Policy
//!
//! Exedra Ops is the public SDK surface. Exedra is the engine kernel.
//!
//! API tiers:
//! - `exedra_ops` (SDK tier): operator reference, selection/query ergonomics,
//!   deterministic plan lifecycle, reporting, and workflow-first examples.
//! - `exedra_mesh` (engine tier): topology/attribute storage, invariants, and
//!   deterministic kernel edits.
//!
//! # Where To Add New API
//!
//! Add to Exedra only when all are true:
//! - Required for kernel correctness, invariants, or performance-critical data
//!   access.
//! - Deterministic and explicit (no hidden mutable global behavior).
//! - Compatible with `no_std` and dependency policy.
//! - Covered by kernel-level tests and validation.
//! - Not a mesh-workflow/operator convenience API.
//!
//! Add mesh-workflow convenience APIs to Exedra Ops. Add operations over a
//! sibling representation to that representation's owning crate. Keep an
//! explicit adapter at the crossing when a workflow converts between them.
//!
//! # Exception Process
//!
//! If a change crosses tiers, open a ticket tagged `api-exception` with:
//! - rationale,
//! - alternatives considered,
//! - maintainer approval note before merge.
