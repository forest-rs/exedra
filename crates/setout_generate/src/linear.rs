// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact labeled station generation.

mod error;
mod generate;
mod types;

pub use error::GenerationError;
pub use generate::distribute_linear;
pub use types::{ItemOverride, LinearDistribution, LinearFragment, LinearGenerator, LinearStation};

/// Canonical schema version for generated fragment fingerprints.
pub const GENERATION_SCHEMA_VERSION: u32 = 1;

/// Maximum active and omitted slots accepted by a linear invocation.
///
/// The bound makes work and allocation predictable. Larger structures should
/// be split into independently named fragments with meaningful boundaries.
pub const MAX_LINEAR_STATIONS: u32 = 65_536;

#[cfg(test)]
mod tests;
