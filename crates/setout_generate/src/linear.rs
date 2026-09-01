// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact labeled station generation.

mod bay;
mod error;
mod generate;
mod item_override;
mod types;

pub use bay::{
    LinearBay, LinearBayDistribution, LinearBayFragment, LinearBayGenerator, distribute_linear_bays,
};
pub use error::GenerationError;
pub use generate::distribute_linear;
pub use item_override::ItemOverride;
pub use types::{LinearDistribution, LinearFragment, LinearGenerator, LinearStation};

/// Canonical schema version for generated fragment fingerprints.
pub const GENERATION_SCHEMA_VERSION: u32 = 1;

/// Maximum active and omitted slots accepted by a linear invocation.
///
/// The bound makes work and allocation predictable. Larger structures should
/// be split into independently named fragments with meaningful boundaries.
pub const MAX_LINEAR_STATIONS: u32 = 65_536;

#[cfg(test)]
mod tests;
