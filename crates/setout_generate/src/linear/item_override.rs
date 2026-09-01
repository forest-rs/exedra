// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit edits shared by linear generator shapes.

use alloc::boxed::Box;

use crate::key::{ItemLabel, KeyError};

/// One explicit change to a generated item.
///
/// The current generator supports omission only. Keeping the target as a
/// semantic label, rather than a numeric vector index, lets an unavailable
/// target remain visible as an orphan instead of affecting a neighbor.
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
