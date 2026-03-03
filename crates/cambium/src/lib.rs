// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operator and growth layer for the exedra mesh kernel.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod artifact;
pub mod context;
pub mod diag;
pub mod error;
pub mod report;
mod timing;

pub use artifact::{Artifact, Artifacts};
pub use context::{Clock, ClockBucket, ContextPolicy, OpContext, Scratch};
pub use diag::{DiagCode, DiagLevel, DiagSpan, Diagnostic, DiagnosticsSink};
pub use error::{OpError, OpErrorKind};
pub use report::{ElementCounts, OpReport, SmallCounters, Stats, TimeBucket, Timings};

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {}
}
