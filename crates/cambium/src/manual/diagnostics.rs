// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Diagnostics and Errors
//!
//! Cambium separates machine-facing error classification from human-facing
//! diagnostics:
//! - classification: [`OpErrorKind`](crate::OpErrorKind)
//! - payload: [`OpError`](crate::OpError)
//! - structured messages: [`Diagnostic`](crate::Diagnostic)
//! - bounded retention: [`DiagnosticsSink`](crate::DiagnosticsSink)
//!
//! # Recommended Pattern
//!
//! - Use `OpErrorKind` for coarse programmatic handling.
//! - Attach one or more diagnostics with stable [`DiagCode`](crate::DiagCode).
//! - Include topology spans when possible via [`DiagSpan`](crate::DiagSpan).
//! - Prefer actionable, concrete messages.
//!
//! # Retention
//!
//! Diagnostics are bounded and deterministic:
//! - severity priority: Error > Warn > Note
//! - within level: earliest entries retained first
//! - capacity controlled by policy/runtime limits
//!
//! # Example
//!
//! ```rust
//! use cambium::{DiagCode, DiagLevel, Diagnostic, DiagnosticsSink};
//!
//! let mut sink = DiagnosticsSink::new(2);
//! sink.push(Diagnostic::new(DiagLevel::Warn, DiagCode::PreconditionFailed, "warn"));
//! sink.push(Diagnostic::new(DiagLevel::Error, DiagCode::InternalInvariantViolation, "error"));
//! sink.push(Diagnostic::new(DiagLevel::Note, DiagCode::Cancelled, "note"));
//!
//! let kept = sink.iter().collect::<Vec<_>>();
//! assert_eq!(kept.len(), 2);
//! assert_eq!(kept[0].level, DiagLevel::Error);
//! ```
