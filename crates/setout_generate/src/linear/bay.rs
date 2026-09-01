// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact labeled bay generation.

mod generate;
mod types;

pub use generate::distribute_linear_bays;
pub use types::{LinearBay, LinearBayDistribution, LinearBayFragment, LinearBayGenerator};

#[cfg(test)]
mod tests;
