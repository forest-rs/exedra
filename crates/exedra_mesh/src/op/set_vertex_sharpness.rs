// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

use crate::{ChangeSink, EditSession, VertexId};

/// Structured failure from [`set_vertex_sharpness`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SetVertexSharpnessError {
    /// Vertex ID is stale or missing.
    VertexNotLive {
        /// Numeric vertex slot index from the rejected ID.
        vertex: u32,
    },
}

impl fmt::Display for SetVertexSharpnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VertexNotLive { vertex } => write!(f, "vertex is not live: {vertex}"),
        }
    }
}

impl core::error::Error for SetVertexSharpnessError {}

/// Writes one explicit vertex sharpness override.
///
/// Absence means downstream subdivision classifiers should derive vertex class
/// from incident edge sharpness. Writing `0.0` is an explicit smooth override,
/// while positive values represent increasingly sharp authored corners.
pub fn set_vertex_sharpness<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    vertex: VertexId,
    sharpness: f32,
) -> Result<(), SetVertexSharpnessError> {
    if session.set_vertex_sharpness_impl(vertex, sharpness) {
        Ok(())
    } else {
        Err(SetVertexSharpnessError::VertexNotLive {
            vertex: vertex.index(),
        })
    }
}
