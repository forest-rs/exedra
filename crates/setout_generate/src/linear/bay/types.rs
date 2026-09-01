// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Linear-bay invocation and immutable result types.

use alloc::boxed::Box;

use setout::{Count, Fingerprint, Offset, Rational};

use crate::delta::{DeltaError, FragmentDelta};
use crate::key::{InvocationKey, ItemKey, ItemLabel};

use super::super::error::GenerationError;
use super::super::item_override::ItemOverride;
use super::generate::distribute_linear_bays;

/// Exact inputs for a linear sequence of edge-to-edge bays.
#[derive(Copy, Clone, Debug)]
pub struct LinearBayDistribution<'a> {
    /// Stable identity shared by all re-expansions of this invocation.
    pub invocation: &'a InvocationKey,
    /// Exact outer edge of the first bay.
    pub start: Offset,
    /// Exact outer edge of the final bay.
    pub end: Offset,
    /// Number of equal bays between the two outer edges.
    pub bays: Count,
    /// Explicit bay omissions, in any order.
    pub overrides: &'a [ItemOverride],
}

/// One generated bay with exact edges, center, and stable rank identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinearBay {
    pub(super) key: ItemKey,
    pub(super) label: ItemLabel,
    pub(super) ordinal: u32,
    pub(super) start: Rational,
    pub(super) end: Rational,
    pub(super) center: Rational,
}

impl LinearBay {
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

    /// Returns the zero-based spatial sequence position.
    ///
    /// This ordinal is convenient for presentation, but is not the bay's
    /// durable identity.
    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    /// Returns the exact first edge measured in rational joto iotas.
    #[must_use]
    pub const fn start(&self) -> Rational {
        self.start
    }

    /// Returns the exact final edge measured in rational joto iotas.
    #[must_use]
    pub const fn end(&self) -> Rational {
        self.end
    }

    /// Returns the exact midpoint measured in rational joto iotas.
    #[must_use]
    pub const fn center(&self) -> Rational {
        self.center
    }
}

/// Immutable result of one exact linear-bay invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinearBayFragment {
    pub(super) invocation: InvocationKey,
    pub(super) start: Offset,
    pub(super) end: Offset,
    pub(super) bays: u32,
    pub(super) items: Box<[LinearBay]>,
    pub(super) orphaned_overrides: Box<[ItemOverride]>,
    pub(super) fingerprint: Fingerprint,
}

impl LinearBayFragment {
    /// Returns the invocation identity.
    #[must_use]
    pub const fn invocation(&self) -> &InvocationKey {
        &self.invocation
    }

    /// Returns the exact first outer edge.
    #[must_use]
    pub const fn start(&self) -> Offset {
        self.start
    }

    /// Returns the exact final outer edge.
    #[must_use]
    pub const fn end(&self) -> Offset {
        self.end
    }

    /// Returns the number of equal bays in the unedited invocation.
    #[must_use]
    pub const fn bays(&self) -> u32 {
        self.bays
    }

    /// Returns generated, non-omitted bays in spatial order.
    #[must_use]
    pub const fn items(&self) -> &[LinearBay] {
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
        FragmentDelta::between_bays(self, next)
    }
}

/// Stateful convenience wrapper for warm linear-bay re-expansion.
#[derive(Clone, Debug)]
pub struct LinearBayGenerator {
    fragment: LinearBayFragment,
}

impl LinearBayGenerator {
    /// Evaluates the initial fragment.
    ///
    /// # Errors
    ///
    /// Returns [`GenerationError`] when the distribution is invalid.
    pub fn new(spec: &LinearBayDistribution<'_>) -> Result<Self, GenerationError> {
        Ok(Self {
            fragment: distribute_linear_bays(spec)?,
        })
    }

    /// Returns the current immutable fragment.
    #[must_use]
    pub const fn fragment(&self) -> &LinearBayFragment {
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
        spec: &LinearBayDistribution<'_>,
    ) -> Result<FragmentDelta, GenerationError> {
        let next = distribute_linear_bays(spec)?;
        let delta = self
            .fragment
            .delta_to(&next)
            .map_err(|_| GenerationError::InvocationChanged)?;
        self.fragment = next;
        Ok(delta)
    }
}
