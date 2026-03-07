// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

use crate::{ChangeSink, EditSession, HalfEdgeId};

/// Structured failure from [`set_edge_sharpness`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SetEdgeSharpnessError {
    /// Half-edge ID is stale or missing a live twin.
    HalfEdgeNotLive {
        /// Numeric half-edge slot index from the rejected ID.
        half_edge: u32,
    },
}

impl fmt::Display for SetEdgeSharpnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HalfEdgeNotLive { half_edge } => {
                write!(f, "half-edge is not live: {half_edge}")
            }
        }
    }
}

impl core::error::Error for SetEdgeSharpnessError {}

/// Writes one explicit edge sharpness value.
pub fn set_edge_sharpness<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    half_edge: HalfEdgeId,
    sharpness: f32,
) -> Result<(), SetEdgeSharpnessError> {
    if session.set_edge_sharpness_impl(half_edge, sharpness) {
        Ok(())
    } else {
        Err(SetEdgeSharpnessError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })
    }
}
