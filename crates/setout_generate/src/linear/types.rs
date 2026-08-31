// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Linear invocation and immutable result types.

use alloc::boxed::Box;

use setout::{Count, Fingerprint, Offset, Rational};

use crate::delta::{DeltaError, FragmentDelta};
use crate::key::{InvocationKey, ItemKey, ItemLabel, KeyError};

use super::error::GenerationError;
use super::generate::distribute_linear;

/// One explicit change to a generated item.
///
/// The first slice supports omission only. Keeping the target as a semantic
/// label, rather than a numeric vector index, lets an unavailable target remain
/// visible as an orphan instead of affecting a neighboring station.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ItemOverride {
    target: ItemLabel,
}

impl ItemOverride {
    /// Creates an omission targeting an invocation-local item label.
    pub fn omit(target: impl Into<Box<str>>) -> Result<Self, KeyError> {
        Ok(Self {
            target: ItemLabel::new(target)?,
        })
    }

    /// Returns the semantic target of this omission.
    #[must_use]
    pub const fn target(&self) -> &ItemLabel {
        &self.target
    }
}

/// Exact inputs for an endpoint-inclusive linear station invocation.
#[derive(Copy, Clone, Debug)]
pub struct LinearDistribution<'a> {
    /// Stable identity shared by all re-expansions of this invocation.
    pub invocation: &'a InvocationKey,
    /// Exact first station coordinate.
    pub start: Offset,
    /// Exact final station coordinate.
    pub end: Offset,
    /// Number of spaces between the endpoint stations.
    pub intervals: Count,
    /// Explicit item omissions, in any order.
    pub overrides: &'a [ItemOverride],
}

/// One generated station with stable semantic identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinearStation {
    pub(super) key: ItemKey,
    pub(super) label: ItemLabel,
    pub(super) ordinal: u32,
    pub(super) position: Rational,
}

impl LinearStation {
    /// Returns the globally unique semantic item key.
    #[must_use]
    pub const fn key(&self) -> &ItemKey {
        &self.key
    }

    /// Returns the invocation-local semantic label.
    #[must_use]
    pub const fn label(&self) -> &ItemLabel {
        &self.label
    }

    /// Returns the spatial sequence position.
    ///
    /// This ordinal is convenient for lowering and presentation, but is not
    /// the station's durable identity.
    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    /// Returns the exact rational coordinate measured in joto iotas.
    #[must_use]
    pub const fn position(&self) -> Rational {
        self.position
    }
}

/// Immutable result of one exact linear invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinearFragment {
    pub(super) invocation: InvocationKey,
    pub(super) start: Offset,
    pub(super) end: Offset,
    pub(super) intervals: u32,
    pub(super) items: Box<[LinearStation]>,
    pub(super) orphaned_overrides: Box<[ItemOverride]>,
    pub(super) fingerprint: Fingerprint,
}

impl LinearFragment {
    /// Returns the invocation identity.
    #[must_use]
    pub const fn invocation(&self) -> &InvocationKey {
        &self.invocation
    }

    /// Returns the exact first anchor.
    #[must_use]
    pub const fn start(&self) -> Offset {
        self.start
    }

    /// Returns the exact final anchor.
    #[must_use]
    pub const fn end(&self) -> Offset {
        self.end
    }

    /// Returns the number of spaces between endpoint stations.
    #[must_use]
    pub const fn intervals(&self) -> u32 {
        self.intervals
    }

    /// Returns generated, non-omitted items in spatial order.
    #[must_use]
    pub const fn items(&self) -> &[LinearStation] {
        &self.items
    }

    /// Returns overrides whose semantic target is absent from this expansion.
    #[must_use]
    pub const fn orphaned_overrides(&self) -> &[ItemOverride] {
        &self.orphaned_overrides
    }

    /// Returns the deterministic fingerprint of inputs, overrides, and output.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Compares this fragment with a re-expansion of the same invocation.
    ///
    /// # Errors
    ///
    /// Returns [`DeltaError`] if `next` belongs to another invocation.
    pub fn delta_to(&self, next: &Self) -> Result<FragmentDelta, DeltaError> {
        FragmentDelta::between(self, next)
    }
}

/// Stateful convenience wrapper for warm re-expansion.
///
/// Generation itself remains a pure function. This wrapper only retains the
/// previous immutable fragment so callers receive a key-based delta and the
/// same fresh result without maintaining two sources of truth.
#[derive(Clone, Debug)]
pub struct LinearGenerator {
    fragment: LinearFragment,
}

impl LinearGenerator {
    /// Evaluates the initial fragment.
    ///
    /// # Errors
    ///
    /// Returns [`GenerationError`] when the distribution is invalid.
    pub fn new(spec: &LinearDistribution<'_>) -> Result<Self, GenerationError> {
        Ok(Self {
            fragment: distribute_linear(spec)?,
        })
    }

    /// Returns the current immutable fragment.
    #[must_use]
    pub const fn fragment(&self) -> &LinearFragment {
        &self.fragment
    }

    /// Re-expands the same invocation and returns its semantic delta.
    ///
    /// The current fragment changes only after both generation and comparison
    /// succeed.
    ///
    /// # Errors
    ///
    /// Returns [`GenerationError`] for an invalid distribution or a changed
    /// invocation identity.
    pub fn reexpand(
        &mut self,
        spec: &LinearDistribution<'_>,
    ) -> Result<FragmentDelta, GenerationError> {
        let next = distribute_linear(spec)?;
        let delta = self
            .fragment
            .delta_to(&next)
            .map_err(|_| GenerationError::InvocationChanged)?;
        self.fragment = next;
        Ok(delta)
    }
}
