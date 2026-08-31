// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Key-based comparison of immutable generated fragments.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::fmt;

use crate::{InvocationKey, ItemKey, ItemLabel, LinearFragment, LinearStation};

/// Semantic changes between two expansions of one invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FragmentDelta {
    retained: Box<[ItemKey]>,
    added: Box<[ItemKey]>,
    removed: Box<[ItemKey]>,
    changed: Box<[ItemKey]>,
    orphaned_overrides: Box<[ItemLabel]>,
}

impl FragmentDelta {
    pub(crate) fn between(
        previous: &LinearFragment,
        next: &LinearFragment,
    ) -> Result<Self, DeltaError> {
        if previous.invocation() != next.invocation() {
            return Err(DeltaError::InvocationMismatch {
                previous: previous.invocation().clone(),
                next: next.invocation().clone(),
            });
        }
        let previous_by_key: BTreeMap<_, _> = previous
            .items()
            .iter()
            .map(|item| (item.key(), item))
            .collect();
        let next_by_key: BTreeMap<_, _> =
            next.items().iter().map(|item| (item.key(), item)).collect();

        let retained: Vec<_> = next
            .items()
            .iter()
            .filter(|item| previous_by_key.contains_key(item.key()))
            .map(|item| item.key().clone())
            .collect();
        let added: Vec<_> = next
            .items()
            .iter()
            .filter(|item| !previous_by_key.contains_key(item.key()))
            .map(|item| item.key().clone())
            .collect();
        let removed: Vec<_> = previous
            .items()
            .iter()
            .filter(|item| !next_by_key.contains_key(item.key()))
            .map(|item| item.key().clone())
            .collect();
        let changed: Vec<_> = next
            .items()
            .iter()
            .filter(|item| {
                previous_by_key
                    .get(item.key())
                    .is_some_and(|previous| payload_changed(previous, item))
            })
            .map(|item| item.key().clone())
            .collect();
        let orphaned_overrides = next
            .orphaned_overrides()
            .iter()
            .map(|item_override| item_override.target().clone())
            .collect::<Vec<_>>();

        Ok(Self {
            retained: retained.into_boxed_slice(),
            added: added.into_boxed_slice(),
            removed: removed.into_boxed_slice(),
            changed: changed.into_boxed_slice(),
            orphaned_overrides: orphaned_overrides.into_boxed_slice(),
        })
    }

    /// Returns keys present in both expansions, in new spatial order.
    #[must_use]
    pub const fn retained(&self) -> &[ItemKey] {
        &self.retained
    }

    /// Returns keys present only in the new expansion, in spatial order.
    #[must_use]
    pub const fn added(&self) -> &[ItemKey] {
        &self.added
    }

    /// Returns keys present only in the previous expansion, in old spatial order.
    #[must_use]
    pub const fn removed(&self) -> &[ItemKey] {
        &self.removed
    }

    /// Returns retained keys whose exact payload changed.
    #[must_use]
    pub const fn changed(&self) -> &[ItemKey] {
        &self.changed
    }

    /// Returns current override targets that do not exist in the new topology.
    #[must_use]
    pub const fn orphaned_overrides(&self) -> &[ItemLabel] {
        &self.orphaned_overrides
    }

    /// Returns whether there is no generated work and no unresolved override.
    ///
    /// A still-orphaned override keeps this false even when two expansions are
    /// otherwise identical, so callers do not accidentally treat unresolved
    /// intent as a clean result.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && self.orphaned_overrides.is_empty()
    }
}

fn payload_changed(previous: &LinearStation, next: &LinearStation) -> bool {
    // Ordinal is current presentation order, not durable payload. In
    // particular, an endpoint can move from ordinal 7 to 8 while retaining
    // both its semantic identity and exact coordinate.
    previous.position() != next.position()
}

/// Failure to compare fragments that do not share an invocation identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeltaError {
    /// The fragments belong to different generation invocations.
    InvocationMismatch {
        /// Previous invocation identity.
        previous: InvocationKey,
        /// New invocation identity.
        next: InvocationKey,
    },
}

impl fmt::Display for DeltaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvocationMismatch { previous, next } => {
                write!(
                    formatter,
                    "cannot compare invocation {previous} with {next}"
                )
            }
        }
    }
}

impl core::error::Error for DeltaError {}

#[cfg(test)]
mod tests {
    use setout::{Count, Offset};

    use super::*;
    use crate::{LinearDistribution, distribute_linear};

    #[test]
    fn count_growth_preserves_endpoint_identity_and_reports_payload_changes() {
        // The old end remains `end`; the added rank receives a new interior
        // key. Shared interior ranks are retained but reported changed because
        // uniform redistribution moved their exact coordinates.
        let invocation = InvocationKey::new("fixture/grid").unwrap();
        let four = distribute_linear(&LinearDistribution {
            invocation: &invocation,
            start: Offset::ZERO,
            end: Offset::from_iota(12),
            intervals: Count::new(3),
            overrides: &[],
        })
        .unwrap();
        let five = distribute_linear(&LinearDistribution {
            invocation: &invocation,
            start: Offset::ZERO,
            end: Offset::from_iota(12),
            intervals: Count::new(4),
            overrides: &[],
        })
        .unwrap();
        let delta = four.delta_to(&five).unwrap();

        assert!(
            delta
                .retained()
                .iter()
                .any(|key| key.as_str() == "fixture/grid/end")
        );
        assert_eq!(
            delta.added(),
            &[ItemKey::new("fixture/grid/interior/000003").unwrap()]
        );
        assert_eq!(delta.changed().len(), 2);
        assert!(delta.removed().is_empty());
    }

    #[test]
    fn different_invocations_cannot_be_compared() {
        // A caller must not accidentally interpret unrelated fragments as one
        // incremental history merely because their payload shapes match.
        let first_key = InvocationKey::new("fixture/first").unwrap();
        let second_key = InvocationKey::new("fixture/second").unwrap();
        let spec = |invocation| LinearDistribution {
            invocation,
            start: Offset::ZERO,
            end: Offset::from_iota(1),
            intervals: Count::new(1),
            overrides: &[],
        };
        let first = distribute_linear(&spec(&first_key)).unwrap();
        let second = distribute_linear(&spec(&second_key)).unwrap();

        assert!(matches!(
            first.delta_to(&second),
            Err(DeltaError::InvocationMismatch { .. })
        ));
    }
}
