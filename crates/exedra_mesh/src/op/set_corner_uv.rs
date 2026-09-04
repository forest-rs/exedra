// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

use crate::{ChangeSink, CornerId, EditSession};

/// Structured failure from [`set_corner_uv`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SetCornerUvError {
    /// Corner ID is stale or missing.
    CornerNotLive {
        /// Numeric corner slot index from the rejected ID.
        corner: u32,
    },
}

impl fmt::Display for SetCornerUvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CornerNotLive { corner } => write!(f, "corner is not live: {corner}"),
        }
    }
}

impl core::error::Error for SetCornerUvError {}

/// Writes one corner UV value.
pub fn set_corner_uv<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    corner: CornerId,
    uv: [f32; 2],
) -> Result<(), SetCornerUvError> {
    if session.set_corner_uv_impl(corner, uv) {
        Ok(())
    } else {
        Err(SetCornerUvError::CornerNotLive {
            corner: corner.index(),
        })
    }
}
