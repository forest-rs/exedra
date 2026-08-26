// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Domain identities and typed assembly query vocabulary.

use alloc::{boxed::Box, string::String};
use core::{convert::Infallible, fmt, str::FromStr};

use addressable_tree::{
    Measured as TreeMeasured, TreeAxis, TreeLocation, TreeLocator, TreeQuery, TreeResolution,
};

use crate::{Assembly, InstanceAddress};

/// Durable semantic identity of an Exedra part definition.
///
/// Obtain one from [`AssemblyReferent::part`] while resolving an occurrence, or
/// construct one from the stable key accepted by [`Assembly::part_by_key`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartKey(Box<str>);

impl PartKey {
    /// Owns a stable Exedra part key.
    #[must_use]
    pub fn new(key: impl Into<Box<str>>) -> Self {
        Self(key.into())
    }

    /// Returns the Exedra part key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PartKey {
    fn from(key: &str) -> Self {
        Self::new(key)
    }
}

impl From<String> for PartKey {
    fn from(key: String) -> Self {
        Self::new(key.into_boxed_str())
    }
}

impl fmt::Display for PartKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PartKey {
    type Err = Infallible;

    fn from_str(key: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(key))
    }
}

/// Durable semantic identity resolved at an assembly address.
///
/// [`Self::Assembly`] belongs to the synthetic `/` occurrence. Every concrete
/// instance refers to a [`Self::Part`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssemblyReferent {
    /// The complete assembly.
    Assembly,
    /// A registered part definition, which may have several occurrences.
    Part(PartKey),
}

impl AssemblyReferent {
    /// Constructs a part referent from its stable registration key.
    #[must_use]
    pub fn part(key: impl Into<PartKey>) -> Self {
        Self::Part(key.into())
    }

    /// Returns the part key, or `None` for the assembly referent.
    #[must_use]
    pub const fn as_part(&self) -> Option<&PartKey> {
        match self {
            Self::Assembly => None,
            Self::Part(key) => Some(key),
        }
    }
}

impl fmt::Display for AssemblyReferent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Assembly => formatter.write_str("assembly"),
            Self::Part(key) => write!(formatter, "part:{key}"),
        }
    }
}

/// Failure to parse a persisted [`AssemblyReferent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssemblyReferentParseError;

impl FromStr for AssemblyReferent {
    type Err = AssemblyReferentParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == "assembly" {
            return Ok(Self::Assembly);
        }
        text.strip_prefix("part:")
            .map(|key| Self::Part(PartKey::from(key)))
            .ok_or(AssemblyReferentParseError)
    }
}

/// Durable identity of one contextual occurrence in the instance view.
///
/// Obtain this through [`AssemblyLocation::occurrence`]. Concrete occurrences
/// carry the same structured address stored by [`Instance`](crate::Instance);
/// the assembly occurrence is the query root.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssemblyOccurrence {
    /// Synthetic occurrence representing the forest of root instances.
    Assembly,
    /// Stable address of one placed part.
    Instance(InstanceAddress),
}

impl AssemblyOccurrence {
    /// Returns the stable instance address, or `None` for the synthetic root.
    #[must_use]
    pub const fn as_instance(&self) -> Option<&InstanceAddress> {
        match self {
            Self::Assembly => None,
            Self::Instance(address) => Some(address),
        }
    }
}

impl fmt::Display for AssemblyOccurrence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Assembly => formatter.write_str("/"),
            Self::Instance(address) => address.fmt(formatter),
        }
    }
}

/// Named view exposed by [`AddressableAssembly`](crate::AddressableAssembly).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssemblyView {
    /// Synthetic root plus the Exedra instance forest.
    Instances,
}

impl fmt::Display for AssemblyView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("instances")
    }
}

/// A dynamic view name was not part of the assembly schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssemblyViewParseError;

impl FromStr for AssemblyView {
    type Err = AssemblyViewParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "instances" => Ok(Self::Instances),
            _ => Err(AssemblyViewParseError),
        }
    }
}

/// Typed instance predicates evaluated by [`AddressableAssembly`](crate::AddressableAssembly).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssemblyPredicate {
    /// Match the synthetic root or any concrete instance.
    Any,
    /// Match occurrences of one stable part definition.
    Part(PartKey),
    /// Match an exact instance metadata key/value pair.
    MetadataEquals {
        /// Opaque metadata key.
        key: Box<str>,
        /// Required metadata value.
        value: Box<str>,
    },
}

impl AssemblyPredicate {
    /// Matches every occurrence of the registered part key.
    #[must_use]
    pub fn part(key: impl Into<PartKey>) -> Self {
        Self::Part(key.into())
    }

    /// Matches instances carrying the exact metadata pair.
    #[must_use]
    pub fn metadata_equals(key: impl Into<Box<str>>, value: impl Into<Box<str>>) -> Self {
        Self::MetadataEquals {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// Rooted-tree navigation implemented by `addressable_tree`.
pub type AssemblyAxis = TreeAxis;

/// Resolved occurrence produced by assembly resolution and queries.
pub type AssemblyLocation = TreeLocation<Assembly>;

/// View-qualified locator accepted by assembly resolution and queries.
pub type AssemblyLocator = TreeLocator<Assembly>;

/// Rich resolution outcome returned by `resolve` and `resolve_pinned`.
pub type AssemblyResolution = TreeResolution<Assembly>;

/// Typed assembly query, defaulting to many-result cardinality.
pub type AssemblyQuery<C = addressable::Many> = TreeQuery<Assembly, C>;

/// A cardinality-shaped query value paired with measured work.
pub type Measured<T> = TreeMeasured<T>;
