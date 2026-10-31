// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Filled-region operations on analytic line and circular-arc boundaries.

use crate::profile::{LoopError, Profile, ProfileError, ProfilePolicy, SegmentRef};
use alloc::vec::Vec;

mod arrangement;
#[cfg(test)]
mod tests;

/// A set operation on two filled profiles.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RegionOp {
    /// Material in either input.
    Union,
    /// Material in both inputs.
    Intersection,
    /// Material in the left input and outside the right input.
    Difference,
}

impl RegionOp {
    fn contains(self, a: bool, b: bool) -> bool {
        match self {
            Self::Union => a || b,
            Self::Intersection => a && b,
            Self::Difference => a && !b,
        }
    }
}

/// Which operand owns an input segment.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    /// The profile receiving `combine`.
    Left,
    /// The other profile.
    Right,
}

/// A boundary segment of one input.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct InputSegment {
    /// Owning operand.
    pub operand: Operand,
    /// Loop and segment within that profile.
    pub segment: SegmentRef,
}

/// Explicit arrangement and operation work limits.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RegionPolicy {
    /// Validation applied to both inputs and each output profile.
    pub profile: ProfilePolicy,
    /// Maximum cross-operand segment-pair intersection tests.
    pub max_pair_tests: u64,
    /// Maximum fragment-versus-input boundary classification tests.
    pub max_classification_tests: u64,
    /// Maximum intersection/contact events, including authored endpoints.
    pub max_events: u32,
    /// Maximum split boundary fragments before classification.
    pub max_fragments: u32,
}

impl RegionPolicy {
    /// A profile policy and bounded defaults for pair tests, events and fragments.
    #[must_use]
    pub const fn new(profile: ProfilePolicy) -> Self {
        Self {
            profile,
            max_pair_tests: 1_000_000,
            max_classification_tests: 1_000_000,
            max_events: 16_384,
            max_fragments: 16_384,
        }
    }
}

/// The exhausted operation resource.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RegionBudget {
    /// Cross-input segment-pair tests.
    PairTests,
    /// Fragment-versus-boundary classification tests.
    ClassificationTests,
    /// Intersection/contact events.
    Events,
    /// Split fragments.
    Fragments,
}

/// Why an analytic region operation could not produce validated simple profiles.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RegionError {
    /// An operation budget is zero.
    InvalidBudget {
        /// Offending resource.
        resource: RegionBudget,
        /// Supplied limit.
        maximum: u64,
    },
    /// An input fails the requested profile validation policy.
    Input {
        /// Offending operand.
        operand: Operand,
        /// Arrangement violation.
        error: ProfileError,
    },
    /// The next operation charge would exceed its budget.
    Budget {
        /// Exhausted resource.
        resource: RegionBudget,
        /// Caller limit.
        maximum: u64,
        /// Required charge including the next item.
        required: u64,
    },
    /// Near-coincident curves cannot be classified unambiguously in f64.
    Ambiguous {
        /// Fragment's source.
        first: InputSegment,
        /// Conflicting boundary.
        second: InputSegment,
    },
    /// The boundary branches or ends at a point; simple profiles cannot encode it.
    NonManifold {
        /// The conflicting point in model units.
        point: [f64; 2],
        /// Incoming retained fragments.
        incoming: usize,
        /// Outgoing retained fragments.
        outgoing: usize,
    },
    /// Constructed segment geometry is not representable.
    Loop(LoopError),
    /// An output violates the requested separation or arrangement contract.
    Output(ProfileError),
    /// A result expected to contain one connected profile contains another count.
    ExpectedOne {
        /// Number of connected profiles, including zero for an empty result.
        count: usize,
    },
}

impl core::fmt::Display for RegionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidBudget { resource, maximum } => write!(
                f,
                "region {resource:?} budget must be positive, got {maximum}"
            ),
            Self::Input { operand, error } => write!(f, "{operand:?} region: {error}"),
            Self::Budget {
                resource,
                maximum,
                required,
            } => write!(
                f,
                "region {resource:?} requires {required}, exceeding {maximum}"
            ),
            Self::Ambiguous { first, second } => write!(
                f,
                "region boundary is numerically ambiguous between {first:?} and {second:?}"
            ),
            Self::NonManifold {
                point,
                incoming,
                outgoing,
            } => write!(
                f,
                "region boundary at {point:?} has {incoming} incoming and {outgoing} outgoing fragments"
            ),
            Self::Loop(error) => write!(f, "region loop: {error}"),
            Self::Output(error) => write!(f, "region output: {error}"),
            Self::ExpectedOne { count } => write!(f, "expected one connected region, got {count}"),
        }
    }
}
impl core::error::Error for RegionError {}

/// The oriented fraction of a source segment used by an output segment.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SegmentSource {
    /// Segment in the corresponding input profile.
    pub segment: SegmentRef,
    /// Start and end fractions; descending fractions mean reversed orientation.
    pub parameters: [f64; 2],
}

/// Source correspondence for one retained analytic output segment.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RegionSegment {
    /// Index in [`RegionResult::profiles`].
    pub profile: usize,
    /// Loop and segment in that output profile.
    pub segment: SegmentRef,
    /// Left and right input correspondence. Coincident retained boundaries
    /// carry both sources; a split keeps its oriented parameter interval.
    pub sources: [Option<SegmentSource>; 2],
}

/// Work and numerical evidence from an analytic region operation.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct RegionReport {
    /// Cross-operand pair tests performed.
    pub pair_tests: u64,
    /// Fragment-versus-boundary classifications performed.
    pub classification_tests: u64,
    /// Contact/intersection events found.
    pub events: u32,
    /// Split fragments classified before coincident duplicates are removed.
    pub fragments: u32,
    /// Retained analytic segments.
    pub retained_segments: usize,
    /// Closed output boundary loops.
    pub loops: usize,
    /// Largest distance of an event's shared coordinate from either source
    /// piece. This measures f64 intersection residual, not mesh deviation.
    pub max_intersection_residual: f64,
}

/// Zero or more connected profiles retaining analytic line and arc boundaries.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionResult {
    /// Connected profiles, each with its clockwise holes and validated separately.
    pub profiles: Vec<Profile>,
    /// Correspondence in profile, outer-then-hole segment order.
    pub segments: Vec<RegionSegment>,
    /// Bounded work and numerical evidence.
    pub report: RegionReport,
}

impl RegionResult {
    /// Takes the sole connected profile.
    ///
    /// # Errors
    ///
    /// Returns [`RegionError::ExpectedOne`] for empty or disconnected results.
    pub fn into_single(mut self) -> Result<Profile, RegionError> {
        if self.profiles.len() != 1 {
            return Err(RegionError::ExpectedOne {
                count: self.profiles.len(),
            });
        }
        Ok(self.profiles.remove(0))
    }
}
