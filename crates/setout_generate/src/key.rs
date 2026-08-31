// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Stable semantic identity for invocations and generated items.

use alloc::boxed::Box;
use alloc::format;
use core::fmt;

pub use setout::KeyError;
use setout::validate_key;

macro_rules! semantic_key {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(Box<str>);

        impl $name {
            /// Validates and owns a semantic key.
            pub fn new(value: impl Into<Box<str>>) -> Result<Self, KeyError> {
                let value = value.into();
                validate_key(&value)?;
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

semantic_key!(
    InvocationKey,
    "Stable identity of one generation invocation."
);
semantic_key!(
    ItemLabel,
    "Stable invocation-local identity of one generated item."
);
semantic_key!(
    ItemKey,
    "Stable fully-qualified identity of one generated item."
);

impl ItemKey {
    pub(crate) fn within(invocation: &InvocationKey, label: &ItemLabel) -> Self {
        Self::new(format!("{invocation}/{label}")).expect("validated key components compose")
    }
}
