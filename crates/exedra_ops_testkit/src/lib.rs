// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test helpers for Exedra Ops golden snapshots and debug dumps.

#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

pub mod golden;
pub mod regions;
mod smoke;

pub use golden::{GoldenSnapshot, GoldenStep, ParseError, parse_snapshot, render_snapshot};
