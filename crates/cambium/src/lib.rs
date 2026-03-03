// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operator and growth layer for the exedra mesh kernel.

#![no_std]
extern crate alloc;

pub mod diag;

pub use diag::{DiagCode, DiagLevel, DiagSpan, Diagnostic, DiagnosticsSink};

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {}
}
