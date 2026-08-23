// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Stable claim, candidate, decision, and support identity.

use alloc::boxed::Box;
use core::fmt;

use crate::fingerprint::{CanonicalEncoder, Fingerprint};
use crate::key::{DecisionKey, KeyError, MethodId, QuantityKey, RelationKey, RootClaimKey};
use crate::value::Domain;

macro_rules! fingerprint_key {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(Fingerprint);

        impl $name {
            /// Returns the underlying canonical fingerprint.
            #[must_use]
            pub const fn fingerprint(self) -> Fingerprint {
                self.0
            }

            /// Reconstitutes an identity from a persisted canonical fingerprint.
            #[must_use]
            pub const fn from_fingerprint(fingerprint: Fingerprint) -> Self {
                Self(fingerprint)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({})", stringify!($name), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

fingerprint_key!(
    ClaimKey,
    "Fixed-width structural identity of a realized claim."
);
fingerprint_key!(ClaimFingerprint, "Content fingerprint of a realized claim.");
fingerprint_key!(
    CandidateKey,
    "Stable identity of one candidate within a discrete claim."
);
fingerprint_key!(
    SupportKey,
    "Content identity of an interned structural-support set."
);

impl CandidateKey {
    /// Derives a stable candidate identity from a semantic name and value.
    ///
    /// The name, rather than container position, is the durable selection
    /// target. Including the value prevents a changed option from silently
    /// inheriting a decision intended for its earlier meaning.
    #[must_use]
    pub fn named<T: Domain>(name: &str, value: &T) -> Self {
        let mut encoder = CanonicalEncoder::new("setout/candidate");
        encoder.str(name);
        value.encode(&mut encoder);
        Self(encoder.finish())
    }
}

/// One stable candidate in an unresolved discrete claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate<T> {
    /// Stable semantic candidate identity.
    pub key: CandidateKey,
    /// Candidate value.
    pub value: T,
}

impl<T: Domain> Candidate<T> {
    /// Creates a named candidate whose identity is independent of list order.
    #[must_use]
    pub fn named(name: &str, value: T) -> Self {
        Self {
            key: CandidateKey::named(name, &value),
            value,
        }
    }
}

/// Knowledge asserted by a root or derived claim.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Knowledge<T> {
    /// One exact value.
    Exact(T),
    /// A closed set of stable candidates requiring an explicit selection.
    Discrete {
        /// Candidates are canonicalized by key when the root set is built.
        candidates: Box<[Candidate<T>]>,
    },
}

impl<T> Knowledge<T> {
    /// Creates exact knowledge.
    #[must_use]
    pub const fn exact(value: T) -> Self {
        Self::Exact(value)
    }
}

/// Compact evaluation-local claim handle.
///
/// Public persistence and cross-evaluation comparison use [`ClaimKey`]. The
/// generation detects stale handles when a structural slot is replaced within
/// an incremental lineage.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ClaimId {
    index: u32,
    generation: u32,
}

impl ClaimId {
    pub(crate) const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    pub(crate) const fn index(self) -> usize {
        self.index as usize
    }

    pub(crate) const fn generation(self) -> u32 {
        self.generation
    }
}

/// Why a claim exists, without reconstruction-policy labels.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ClaimOrigin {
    /// Authored or imported root claim.
    Root {
        /// Stable root identity.
        key: RootClaimKey,
        /// How the root entered this definition.
        source: RootSource,
    },
    /// Result of one directed relation method.
    Relation {
        /// Relation identity.
        relation: RelationKey,
        /// Directed method identity.
        method: MethodId,
        /// Ordered structural inputs.
        inputs: Box<[ClaimKey]>,
    },
    /// Explicit selection from another claim.
    Resolution {
        /// Decision that made the selection operative.
        decision: DecisionKey,
        /// Structural claim that was selected.
        selected: ClaimKey,
    },
}

/// Structural source of a root claim.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum RootSource {
    /// Value was authored directly by the frontend.
    Authored,
    /// Value came from a named importer and external item.
    Imported {
        /// Stable importer identity.
        importer: Box<str>,
        /// Stable source item within that importer.
        item: Box<str>,
    },
}

/// One atom in structural support.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum SupportAtom {
    /// An authored root supports the claim.
    Root(RootClaimKey),
    /// An explicit decision supports the operative claim.
    Decision(DecisionKey),
}

/// Constant-size reference into an evaluation's interned support arena.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct SupportRef {
    /// Stable content identity of the support set.
    pub key: SupportKey,
    pub(crate) slot: u32,
}

/// Durable instruction for finding a claim producer in a fresh plan.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum ClaimProducer {
    /// An externally supplied root.
    ExternalRoot(RootClaimKey),
    /// One directed method of a relation.
    Relation {
        /// Stable relation identity.
        relation: RelationKey,
        /// Stable method identity.
        method: MethodId,
    },
}

/// A producer directive paired with the structural claim observed at authoring time.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ClaimSelection {
    /// Producer that plan compilation must reproduce.
    pub producer: ClaimProducer,
    /// Structural identity that must still match before selection is applied.
    pub expected: ClaimKey,
}

/// Explicit action that changes which knowledge may propagate.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum DecisionAction {
    /// Selects one complete claim among alternatives.
    SelectClaim {
        /// Quantity whose operative claim changes.
        quantity: QuantityKey,
        /// Non-rebinding producer selection.
        selection: ClaimSelection,
    },
    /// Selects one stable candidate from a discrete claim.
    SelectCandidate {
        /// Quantity whose candidate becomes operative.
        quantity: QuantityKey,
        /// Non-rebinding identity of the producing discrete claim.
        claim: ClaimSelection,
        /// Stable candidate identity; never a display ordinal.
        candidate: CandidateKey,
    },
}

/// One stable decision in an evaluation scenario.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Decision {
    /// Stable decision identity.
    pub key: DecisionKey,
    /// Selection action.
    pub action: DecisionAction,
}

impl Decision {
    /// Creates a decision after validating its key.
    pub fn new(key: &str, action: DecisionAction) -> Result<Self, KeyError> {
        Ok(Self {
            key: DecisionKey::new(key)?,
            action,
        })
    }
}
