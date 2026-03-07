// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

use crate::{ChangeSink, EditSession, HalfEdgeId};

/// Structured failure from [`set_edge_seam`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SetEdgeSeamError {
    /// Half-edge ID is stale or missing a live twin.
    HalfEdgeNotLive {
        /// Numeric half-edge slot index from the rejected ID.
        half_edge: u32,
    },
}

impl fmt::Display for SetEdgeSeamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HalfEdgeNotLive { half_edge } => {
                write!(f, "half-edge is not live: {half_edge}")
            }
        }
    }
}

impl core::error::Error for SetEdgeSeamError {}

/// Writes one explicit edge seam tag.
pub fn set_edge_seam<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    half_edge: HalfEdgeId,
    seam: bool,
) -> Result<(), SetEdgeSeamError> {
    if session.set_edge_seam_impl(half_edge, seam) {
        Ok(())
    } else {
        Err(SetEdgeSeamError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })
    }
}
