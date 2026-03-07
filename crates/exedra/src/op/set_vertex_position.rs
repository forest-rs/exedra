// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

use crate::{EditSession, VertexId};

/// Structured failure from [`set_vertex_position`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SetVertexPositionError {
    /// Vertex ID is stale or missing.
    VertexNotLive {
        /// Numeric vertex slot index from the rejected ID.
        vertex: u32,
    },
}

impl fmt::Display for SetVertexPositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VertexNotLive { vertex } => write!(f, "vertex is not live: {vertex}"),
        }
    }
}

impl core::error::Error for SetVertexPositionError {}

/// Updates one live vertex position.
pub fn set_vertex_position(
    session: &mut EditSession<'_>,
    vertex: VertexId,
    position: [f32; 3],
) -> Result<(), SetVertexPositionError> {
    if session.set_vertex_position_impl(vertex, position) {
        Ok(())
    } else {
        Err(SetVertexPositionError::VertexNotLive {
            vertex: vertex.index(),
        })
    }
}
