// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! SDK Surface Policy
//!
//! Cambium is the public SDK surface. Exedra is the engine kernel.
//!
//! API tiers:
//! - `cambium` (SDK tier): operator catalog, selection/query ergonomics,
//!   deterministic plan lifecycle, reporting, and workflow-first examples.
//! - `exedra` (engine tier): topology/attribute storage, invariants, and
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
//! - Not a workflow/operator convenience API.
//!
//! Otherwise add to Cambium.
//!
//! # Exception Process
//!
//! If a change crosses tiers, open a ticket tagged `api-exception` with:
//! - rationale,
//! - alternatives considered,
//! - maintainer approval note before merge.
