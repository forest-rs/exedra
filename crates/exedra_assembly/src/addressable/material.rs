// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Typed material reads and explanations.

use alloc::{boxed::Box, vec::Vec};

use addressable::{Endpoint, Explained, Opinion};
use addressable_tree::TreeReadError;

use crate::InstanceAddress;

use crate::{AddressableAssembly, AssemblyLocation, AssemblyOccurrence, PartKey, ReadError};

/// Typed material facet for one named part slot.
///
/// Pair this with an [`AssemblyLocation`] through [`Endpoint::new`] and pass it
/// to [`AddressableAssembly::read_material`]. Slot existence is checked against
/// the resolved instance's part at use time.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MaterialSlot(Box<str>);

impl MaterialSlot {
    /// Selects a material slot by its Exedra declaration name.
    #[must_use]
    pub fn new(name: impl Into<Box<str>>) -> Self {
        Self(name.into())
    }

    /// Returns the declared slot name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.0
    }
}

/// Durable subject of an effective material explanation.
///
/// Obtain one from [`Explained::subject`] on a [`MaterialExplanation`]. It
/// identifies the placed part and named slot without retaining a
/// revision-scoped runtime handle.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MaterialSubject {
    occurrence: InstanceAddress,
    part: PartKey,
    slot: Box<str>,
}

impl MaterialSubject {
    /// Returns the stable placed-part address.
    #[must_use]
    pub const fn occurrence(&self) -> &InstanceAddress {
        &self.occurrence
    }

    /// Returns the stable part referent.
    #[must_use]
    pub const fn part(&self) -> &PartKey {
        &self.part
    }

    /// Returns the declared material slot name.
    #[must_use]
    pub fn slot(&self) -> &str {
        &self.slot
    }
}

/// Domain provenance for one candidate material key.
///
/// Inspect these through the opinions returned by
/// [`AddressableAssembly::read_material`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MaterialProvenance {
    /// Binding authored on this placed instance.
    InstanceBinding {
        /// Stable occurrence carrying the binding.
        occurrence: InstanceAddress,
    },
    /// Default authored on the referenced part definition.
    PartDefault {
        /// Stable part carrying the default.
        part: PartKey,
    },
}

/// Why one material opinion became effective.
///
/// Obtain this through [`Explained::reason`] on a [`MaterialExplanation`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialReason {
    /// An instance binding outranked an available part default.
    InstanceBindingOverridesPartDefault,
    /// An instance binding was the only authored opinion.
    InstanceBindingUsed,
    /// No instance binding existed, so the part default was used.
    PartDefaultUsed,
}

/// Explained effective material key for one placed slot.
///
/// Obtain one from [`AddressableAssembly::read_material`]. Read
/// [`Explained::value`] for the winner and [`Explained::opinions`] for the
/// instance-binding and part-default evidence.
pub type MaterialExplanation =
    Explained<MaterialSubject, Box<str>, MaterialProvenance, MaterialReason>;

impl AddressableAssembly {
    /// Reads and explains the effective material at a placed slot.
    ///
    /// `Ok(None)` means the slot exists but has neither an instance binding nor
    /// a part default. Otherwise opinions are ordered by strength, with an
    /// instance binding before the part default it overrides.
    pub fn read_material(
        &self,
        endpoint: &Endpoint<AssemblyLocation, MaterialSlot>,
    ) -> Result<Option<MaterialExplanation>, ReadError> {
        let observation = self.material_observation(endpoint)?;
        let subject = observation.subject;
        let reason = match (&observation.binding, &observation.default) {
            (Some(_), Some(_)) => MaterialReason::InstanceBindingOverridesPartDefault,
            (Some(_), None) => MaterialReason::InstanceBindingUsed,
            (None, Some(_)) => MaterialReason::PartDefaultUsed,
            (None, None) => return Ok(None),
        };
        let mut opinions = Vec::with_capacity(2);
        if let Some(binding) = observation.binding {
            opinions.push(Opinion::new(
                binding,
                MaterialProvenance::InstanceBinding {
                    occurrence: subject.occurrence.clone(),
                },
            ));
        }
        if let Some(default) = observation.default {
            opinions.push(Opinion::new(
                default,
                MaterialProvenance::PartDefault {
                    part: subject.part.clone(),
                },
            ));
        }
        Explained::new(subject, opinions, 0, reason)
            .map(Some)
            .map_err(|_| ReadError::InvalidExplanation)
    }

    fn material_observation(
        &self,
        endpoint: &Endpoint<AssemblyLocation, MaterialSlot>,
    ) -> Result<MaterialObservation, ReadError> {
        let instance_id = self.validate_instance_location(endpoint.owner())?;
        let instance = self
            .assembly()
            .instance(instance_id)
            .ok_or_else(|| ReadError::from(TreeReadError::MissingOccurrence))?;
        let part = self
            .assembly()
            .part(instance.part())
            .ok_or(ReadError::MissingReferent)?;
        let slot = part
            .slot_index(endpoint.facet().name())
            .ok_or(ReadError::UnknownMaterialSlot)?;
        let occurrence = match endpoint.owner().occurrence() {
            AssemblyOccurrence::Instance(address) => address.clone(),
            AssemblyOccurrence::Assembly => return Err(ReadError::RootHasNoInstance),
        };
        let subject = MaterialSubject {
            occurrence,
            part: PartKey::from(part.key()),
            slot: Box::from(endpoint.facet().name()),
        };
        Ok(MaterialObservation {
            subject,
            binding: instance.binding(slot).map(Box::from),
            default: part.default_material(slot).map(Box::from),
        })
    }
}

#[derive(Clone, Debug)]
struct MaterialObservation {
    subject: MaterialSubject,
    binding: Option<Box<str>>,
    default: Option<Box<str>>,
}
