// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operator and growth layer for the exedra mesh kernel.

#![no_std]
extern crate alloc;

pub mod artifact;
pub mod diag;
pub mod report;

pub use artifact::{Artifact, Artifacts};
pub use diag::{DiagCode, DiagLevel, DiagSpan, Diagnostic, DiagnosticsSink};
pub use report::{ElementCounts, OpReport, SmallCounters, Stats, TimeBucket, Timings};

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {}
}
