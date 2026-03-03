// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test helpers for Cambium golden snapshots and debug dumps.

#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod golden;
pub mod regions;

pub use golden::{GoldenSnapshot, GoldenStep, ParseError, parse_snapshot, render_snapshot};
