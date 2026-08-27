// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Typed material reads, explanations, and guarded transactions.

use alloc::{boxed::Box, string::ToString, vec::Vec};

use addressable::{Endpoint, Explained, Guard, Opinion, Revision, Transaction, TransactionMode};
use addressable_tree::TreeReadError;

use crate::{InstanceAddress, InstanceId, SlotIndex};

use crate::{
    AddressableAssembly, AssemblyLocation, AssemblyOccurrence, AssemblyReferent, AssemblySpace,
    PartKey, ReadError,
};

/// Typed material facet for one named part slot.
///
/// Pair this with an [`AssemblyLocation`] through [`Endpoint::new`] and pass it
/// to [`AddressableAssembly::read_material`] or [`BindMaterial::new`]. Slot
/// existence is checked against the resolved instance's part at use time.
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

/// Durable subject of an effective material explanation or change.
///
/// Obtain one from [`Explained::subject`] on a [`MaterialExplanation`] or from
/// [`MaterialChange::subject`]. It identifies the placed part and named slot
/// without retaining a revision-scoped runtime handle.
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

/// Capability required by a guarded assembly edit.
///
/// Place [`Self::BindMaterial`] in the [`Guard`] carried by a
/// [`BindMaterial`] operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditCapability {
    /// Bind or rebind an instance material slot.
    BindMaterial,
}

/// Typed operation that binds one instance material slot.
///
/// Construct this after resolving the owner and calling
/// [`AddressableAssembly::read_material`]. The guard's expected value is the
/// returned explanation value wrapped in `Some`, or `None` when the slot had no
/// effective material. Submit one or more operations with
/// [`AddressableAssembly::transact`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindMaterial {
    endpoint: Endpoint<AssemblyLocation, MaterialSlot>,
    material: Box<str>,
    guard: Guard<AssemblySpace, AssemblyReferent, Option<Box<str>>, EditCapability>,
}

impl BindMaterial {
    /// Creates a guarded material-binding operation.
    #[must_use]
    pub fn new(
        endpoint: Endpoint<AssemblyLocation, MaterialSlot>,
        material: impl Into<Box<str>>,
        guard: Guard<AssemblySpace, AssemblyReferent, Option<Box<str>>, EditCapability>,
    ) -> Self {
        Self {
            endpoint,
            material: material.into(),
            guard,
        }
    }

    /// Returns the typed material endpoint.
    #[must_use]
    pub const fn endpoint(&self) -> &Endpoint<AssemblyLocation, MaterialSlot> {
        &self.endpoint
    }

    /// Returns the proposed material key.
    #[must_use]
    pub fn material(&self) -> &str {
        &self.material
    }

    /// Returns all mutation preconditions.
    #[must_use]
    pub const fn guard(
        &self,
    ) -> &Guard<AssemblySpace, AssemblyReferent, Option<Box<str>>, EditCapability> {
        &self.guard
    }
}

/// One authored binding change reported by a transaction.
///
/// A change may leave the effective key unchanged when it authors an instance
/// binding equal to the previous part default. It still changes provenance and
/// therefore advances the runtime revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialChange {
    subject: MaterialSubject,
    previous_effective: Option<Box<str>>,
    current_effective: Box<str>,
}

impl MaterialChange {
    /// Returns the stable placed-slot subject.
    #[must_use]
    pub const fn subject(&self) -> &MaterialSubject {
        &self.subject
    }

    /// Returns the effective value before the operation.
    #[must_use]
    pub fn previous_effective(&self) -> Option<&str> {
        self.previous_effective.as_deref()
    }

    /// Returns the effective value after the operation.
    #[must_use]
    pub fn current_effective(&self) -> &str {
        &self.current_effective
    }
}

/// Authored state needed to construct a separately guarded undo operation.
///
/// Exedra currently exposes binding but not binding removal. `None` therefore
/// records that exact restoration would require a future clear-binding
/// operation; this value is evidence, not an executable command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoMaterial {
    subject: MaterialSubject,
    previous_binding: Option<Box<str>>,
    expected_binding: Box<str>,
}

impl UndoMaterial {
    /// Returns the stable placed-slot subject.
    #[must_use]
    pub const fn subject(&self) -> &MaterialSubject {
        &self.subject
    }

    /// Returns the instance binding that existed before the transaction.
    #[must_use]
    pub fn previous_binding(&self) -> Option<&str> {
        self.previous_binding.as_deref()
    }

    /// Returns the binding an undo must still observe before restoring state.
    #[must_use]
    pub fn expected_binding(&self) -> &str {
        &self.expected_binding
    }
}

/// Successful dry-run or applied material transaction report.
///
/// Obtain this from [`AddressableAssembly::transact`]. Inspect [`Self::changes`]
/// for placed-slot effects and [`Self::undo`] for the prior authored bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionReport {
    mode: TransactionMode,
    revision_before: Revision<AssemblySpace>,
    revision_after: Revision<AssemblySpace>,
    changes: Vec<MaterialChange>,
    undo: Vec<UndoMaterial>,
}

impl TransactionReport {
    /// Returns whether state was previewed or applied.
    #[must_use]
    pub const fn mode(&self) -> TransactionMode {
        self.mode
    }

    /// Returns the revision validated by the transaction.
    #[must_use]
    pub const fn revision_before(&self) -> Revision<AssemblySpace> {
        self.revision_before
    }

    /// Returns the resulting revision; a dry run retains the prior revision.
    #[must_use]
    pub const fn revision_after(&self) -> Revision<AssemblySpace> {
        self.revision_after
    }

    /// Returns authored binding changes in operation order.
    #[must_use]
    pub fn changes(&self) -> &[MaterialChange] {
        &self.changes
    }

    /// Returns pre-transaction authored state in operation order.
    #[must_use]
    pub fn undo(&self) -> &[UndoMaterial] {
        &self.undo
    }
}

/// Atomic transaction conflict returned by [`AddressableAssembly::transact`].
///
/// No operation is observable when any variant is returned. Operation indices
/// refer to the caller's transaction order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransactionConflict {
    /// The selected target set is no longer current.
    SelectionRevision {
        /// Transaction selection revision.
        expected: Revision<AssemblySpace>,
        /// Current runtime revision.
        actual: Revision<AssemblySpace>,
    },
    /// An endpoint could not be consumed at the current revision.
    Endpoint {
        /// Operation index.
        operation: usize,
        /// Read-side validation failure.
        error: ReadError,
    },
    /// Endpoint and guard name different semantic referents.
    ReferentMismatch {
        /// Operation index.
        operation: usize,
    },
    /// The guard was established at another revision.
    GuardRevision {
        /// Operation index.
        operation: usize,
        /// Guard revision.
        expected: Revision<AssemblySpace>,
        /// Current runtime revision.
        actual: Revision<AssemblySpace>,
    },
    /// Effective material changed since the guard was established.
    ValueMismatch {
        /// Operation index.
        operation: usize,
        /// Guarded material key.
        expected: Option<Box<str>>,
        /// Current effective material key.
        actual: Option<Box<str>>,
    },
    /// The operation did not carry the binding capability.
    CapabilityUnavailable {
        /// Operation index.
        operation: usize,
    },
    /// Several operations target the same placed slot in one atomic batch.
    DuplicateTarget {
        /// Later conflicting operation index.
        operation: usize,
    },
}

#[derive(Clone, Debug)]
struct PreparedMaterial {
    instance: InstanceId,
    slot_index: SlotIndex,
    slot: Box<str>,
    subject: MaterialSubject,
    previous_effective: Option<Box<str>>,
    previous_binding: Option<Box<str>>,
    current: Box<str>,
}

impl AddressableAssembly {
    /// Reads and explains the effective material at a placed slot.
    ///
    /// `Ok(None)` means the slot exists but has neither an instance binding nor
    /// a part default. Otherwise opinions are ordered by strength, with an
    /// instance binding before the part default it overrides. Use the returned
    /// value together with the endpoint owner's referent and revision to form a
    /// coherent [`Guard`].
    pub fn read_material(
        &self,
        endpoint: &Endpoint<AssemblyLocation, MaterialSlot>,
    ) -> Result<Option<MaterialExplanation>, ReadError> {
        let observation = self.material_observation(endpoint)?;
        let subject = observation.subject;
        match (observation.binding, observation.default) {
            (Some(binding), Some(default)) => Explained::new(
                subject.clone(),
                [
                    Opinion::new(
                        binding,
                        MaterialProvenance::InstanceBinding {
                            occurrence: subject.occurrence.clone(),
                        },
                    ),
                    Opinion::new(
                        default,
                        MaterialProvenance::PartDefault {
                            part: subject.part.clone(),
                        },
                    ),
                ],
                0,
                MaterialReason::InstanceBindingOverridesPartDefault,
            )
            .map(Some)
            .map_err(|_| ReadError::InvalidExplanation),
            (Some(binding), None) => Explained::new(
                subject.clone(),
                [Opinion::new(
                    binding,
                    MaterialProvenance::InstanceBinding {
                        occurrence: subject.occurrence.clone(),
                    },
                )],
                0,
                MaterialReason::InstanceBindingUsed,
            )
            .map(Some)
            .map_err(|_| ReadError::InvalidExplanation),
            (None, Some(default)) => Explained::new(
                subject.clone(),
                [Opinion::new(
                    default,
                    MaterialProvenance::PartDefault {
                        part: subject.part.clone(),
                    },
                )],
                0,
                MaterialReason::PartDefaultUsed,
            )
            .map(Some)
            .map_err(|_| ReadError::InvalidExplanation),
            (None, None) => Ok(None),
        }
    }

    /// Atomically validates and previews or applies material bindings.
    ///
    /// Validation resolves every target and slot before mutation. Apply then
    /// commits the already-validated binding indexes in place. A batch with one
    /// or more authored changes advances the runtime revision exactly once.
    pub fn transact(
        &mut self,
        transaction: Transaction<AssemblySpace, BindMaterial>,
    ) -> Result<TransactionReport, TransactionConflict> {
        if transaction.selection_revision() != self.revision() {
            return Err(TransactionConflict::SelectionRevision {
                expected: transaction.selection_revision(),
                actual: self.revision(),
            });
        }

        let mut prepared = Vec::new();
        for (operation, edit) in transaction.operations().iter().enumerate() {
            self.runtime
                .validate_location(edit.endpoint.owner())
                .map_err(ReadError::from)
                .map_err(|error| TransactionConflict::Endpoint { operation, error })?;
            if edit.endpoint.owner().referent() != edit.guard.expected_referent() {
                return Err(TransactionConflict::ReferentMismatch { operation });
            }
            if edit.guard.expected_revision() != self.revision() {
                return Err(TransactionConflict::GuardRevision {
                    operation,
                    expected: edit.guard.expected_revision(),
                    actual: self.revision(),
                });
            }
            if *edit.guard.required_capability() != EditCapability::BindMaterial {
                return Err(TransactionConflict::CapabilityUnavailable { operation });
            }
            let observation = self
                .material_observation(&edit.endpoint)
                .map_err(|error| TransactionConflict::Endpoint { operation, error })?;
            let current = observation.effective();
            if current != *edit.guard.expected_value() {
                return Err(TransactionConflict::ValueMismatch {
                    operation,
                    expected: edit.guard.expected_value().clone(),
                    actual: current,
                });
            }
            if prepared.iter().any(|prior: &PreparedMaterial| {
                prior.subject.occurrence == observation.subject.occurrence
                    && prior.slot.as_ref() == edit.endpoint.facet().name()
            }) {
                return Err(TransactionConflict::DuplicateTarget { operation });
            }
            prepared.push(PreparedMaterial {
                instance: observation.instance,
                slot_index: observation.slot_index,
                slot: Box::from(edit.endpoint.facet().name()),
                subject: observation.subject,
                previous_effective: current,
                previous_binding: observation.binding,
                current: edit.material.clone(),
            });
        }

        let changed = prepared
            .iter()
            .filter(|change| change.previous_binding.as_deref() != Some(change.current.as_ref()))
            .cloned()
            .collect::<Vec<_>>();
        let changes = changed
            .iter()
            .map(|change| MaterialChange {
                subject: change.subject.clone(),
                previous_effective: change.previous_effective.clone(),
                current_effective: change.current.clone(),
            })
            .collect();
        let undo = changed
            .iter()
            .map(|change| UndoMaterial {
                subject: change.subject.clone(),
                previous_binding: change.previous_binding.clone(),
                expected_binding: change.current.clone(),
            })
            .collect();

        let revision_before = self.revision();
        if transaction.mode() == TransactionMode::Apply && !changed.is_empty() {
            self.runtime.commit(|assembly| {
                for change in &changed {
                    assembly.set_material_binding(
                        change.instance,
                        change.slot_index,
                        change.current.to_string(),
                    );
                }
            });
        }

        Ok(TransactionReport {
            mode: transaction.mode(),
            revision_before,
            revision_after: self.revision(),
            changes,
            undo,
        })
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
            instance: instance_id,
            slot_index: slot,
            subject,
            binding: instance.binding(slot).map(Box::from),
            default: part.default_material(slot).map(Box::from),
        })
    }
}

#[derive(Clone, Debug)]
struct MaterialObservation {
    instance: InstanceId,
    slot_index: SlotIndex,
    subject: MaterialSubject,
    binding: Option<Box<str>>,
    default: Option<Box<str>>,
}

impl MaterialObservation {
    fn effective(&self) -> Option<Box<str>> {
        self.binding.clone().or_else(|| self.default.clone())
    }
}
