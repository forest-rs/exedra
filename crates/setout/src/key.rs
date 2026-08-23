// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Stable semantic keys.

use alloc::boxed::Box;
use core::fmt;

/// Why a semantic key was rejected.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum KeyError {
    /// The key is empty.
    Empty,
    /// The key starts or ends with `/` or contains an empty path segment.
    EmptySegment,
    /// The key contains a byte outside the conservative ASCII grammar.
    InvalidByte,
}

impl fmt::Display for KeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("key is empty"),
            Self::EmptySegment => formatter.write_str("key contains an empty path segment"),
            Self::InvalidByte => {
                formatter.write_str("key contains a byte outside [A-Za-z0-9._:/-]")
            }
        }
    }
}

impl core::error::Error for KeyError {}

fn validate(value: &str) -> Result<(), KeyError> {
    if value.is_empty() {
        return Err(KeyError::Empty);
    }
    if value.starts_with('/')
        || value.ends_with('/')
        || value.as_bytes().windows(2).any(|w| w == b"//")
    {
        return Err(KeyError::EmptySegment);
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
    }) {
        return Err(KeyError::InvalidByte);
    }
    Ok(())
}

macro_rules! semantic_key {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(Box<str>);

        impl $name {
            /// Validates and owns a semantic key.
            pub fn new(value: impl Into<Box<str>>) -> Result<Self, KeyError> {
                let value = value.into();
                validate(&value)?;
                Ok(Self(value))
            }

            /// Returns the canonical key text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&self.0)
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = KeyError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
    };
}

semantic_key!(QuantityKey, "Stable external identity of a typed quantity.");
semantic_key!(RootClaimKey, "Stable identity of an authored root claim.");
semantic_key!(RelationKey, "Stable identity of a relation definition.");
semantic_key!(MethodId, "Stable identity of one directed relation method.");
semantic_key!(ScenarioKey, "Stable identity of an evaluation scenario.");
semantic_key!(
    DecisionKey,
    "Stable identity of an explicit human or policy decision."
);
semantic_key!(
    ChoiceDomainKey,
    "Stable identity of a discrete-choice domain."
);
semantic_key!(
    ChoiceOptionKey,
    "Stable identity of one discrete-choice option."
);
