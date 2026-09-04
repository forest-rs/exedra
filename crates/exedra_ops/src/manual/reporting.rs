// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reporting Patterns
//!
//! Operator reporting uses [`OpReport`](crate::OpReport), which contains:
//! - deterministic counters (`stats`: [`Stats`](crate::Stats)),
//! - best-effort timings (`timings`: [`Timings`](crate::Timings)),
//! - bounded artifacts (`artifacts`: [`Artifacts`](crate::Artifacts)).
//!
//! # Timing Guidance
//!
//! Use stable phase names so downstream tools can compare runs:
//! - `select`
//! - `compute`
//! - `attrs`
//! - `validate`
//!
//! Timings are additive by bucket name and bounded by configured bucket count.
//!
//! # Counter Guidance
//!
//! Update counters based on actual work performed:
//! - faces touched/processed,
//! - corners/edges written,
//! - canonicalization passes.
//!
//! Keep semantics stable so consumers can trust trends across versions.
//!
//! # Artifacts
//!
//! Artifacts are debug aids, not primary data flow:
//! - retain only bounded samples/snapshots,
//! - prefer deterministic insertion order,
//! - avoid large payloads when not necessary.
//!
//! # Example
//!
//! ```rust
//! use exedra_ops::{Artifacts, OpReport};
//!
//! let mut report = OpReport::new("example.op", Artifacts::default());
//! report.stats.counters.faces_processed = 2;
//! report.timings.add("compute", 100);
//! report.timings.add("compute", 50);
//! assert_eq!(report.timings.iter().next().map(|b| b.nanos), Some(150));
//! ```
