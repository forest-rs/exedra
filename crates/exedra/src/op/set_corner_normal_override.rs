// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

use crate::{ChangeSink, CornerId, EditSession};

/// Structured failure from [`set_corner_normal_override`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SetCornerNormalOverrideError {
    /// Corner ID is stale or missing.
    CornerNotLive {
        /// Numeric corner slot index from the rejected ID.
        corner: u32,
    },
}

impl fmt::Display for SetCornerNormalOverrideError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CornerNotLive { corner } => write!(f, "corner is not live: {corner}"),
        }
    }
}

impl core::error::Error for SetCornerNormalOverrideError {}

/// Writes or clears one authored corner-normal override.
pub fn set_corner_normal_override<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    corner: CornerId,
    normal: Option<[f32; 3]>,
) -> Result<(), SetCornerNormalOverrideError> {
    if session.set_corner_normal_override_impl(corner, normal) {
        Ok(())
    } else {
        Err(SetCornerNormalOverrideError::CornerNotLive {
            corner: corner.index(),
        })
    }
}
