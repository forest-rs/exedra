// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Typed validation failures for linear generation.

use core::fmt;

use setout::ArithmeticError;

use crate::ItemLabel;

/// Failure to expand an exact linear invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GenerationError {
    /// At least one interval is required to distinguish the two endpoints.
    NoIntervals,
    /// At least one bay is required between the two outer edges.
    NoBays,
    /// The requested station count exceeds [`crate::MAX_LINEAR_STATIONS`].
    TooManyStations {
        /// Requested endpoint-inclusive station count.
        requested: u64,
        /// Maximum accepted station count.
        limit: u32,
    },
    /// Start and end are identical, so the invocation has no linear extent.
    CoincidentAnchors,
    /// More than one override targets the same semantic label.
    DuplicateOverride(ItemLabel),
    /// The override list exceeds the invocation work budget.
    TooManyOverrides {
        /// Requested override count.
        requested: usize,
        /// Maximum accepted override count.
        limit: u32,
    },
    /// Exact coordinate construction failed.
    Arithmetic(ArithmeticError),
    /// Warm re-expansion changed the invocation's stable identity.
    InvocationChanged,
}

impl fmt::Display for GenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoIntervals => formatter.write_str("linear distribution needs one interval"),
            Self::NoBays => formatter.write_str("linear bay distribution needs one bay"),
            Self::TooManyStations { requested, limit } => {
                write!(
                    formatter,
                    "{requested} stations exceed the limit of {limit}"
                )
            }
            Self::CoincidentAnchors => formatter.write_str("linear anchors are coincident"),
            Self::DuplicateOverride(target) => {
                write!(formatter, "duplicate override for {target}")
            }
            Self::TooManyOverrides { requested, limit } => {
                write!(
                    formatter,
                    "{requested} overrides exceed the limit of {limit}"
                )
            }
            Self::Arithmetic(error) => {
                write!(formatter, "exact station arithmetic failed: {error}")
            }
            Self::InvocationChanged => {
                formatter.write_str("warm re-expansion changed invocation identity")
            }
        }
    }
}

impl core::error::Error for GenerationError {}

impl From<ArithmeticError> for GenerationError {
    fn from(value: ArithmeticError) -> Self {
        Self::Arithmetic(value)
    }
}
