// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operation ownership
//!
//! - `exedra_mesh`: structural topology, attributes, traversal and primitive edits.
//! - `exedra_mesh_ops`: direct modeling and geometric queries, with typed errors,
//!   work counters, geometric evidence and source correspondence.
//! - `exedra_edit`: command preparation and execution, previews, timings,
//!   diagnostics, artifacts and application change reports.
//! - Native domains own their representations, conversions and operations.
//!
//! Add geometric algorithms to mesh ops and call them from command adapters.
//! Do not require an operation runner, clock or application context to compute
//! geometry. Constructive binds authored meaning to geometric evidence; its
//! recipes and semantic selections are not inputs to mesh ops.
//!
//! Kernel edit sessions are eager. Prepared geometry validates its own consumed
//! source state; command plans additionally bind application execution to the
//! captured mesh state. Neither in-place path implies rollback on failure.
