// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{ChangeSink, EditSession, HalfEdgeId, VertexId};

use super::DeleteVerticesError;

/// Deletes a canonical set of isolated vertices.
pub fn delete_vertices<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    vertices: &[VertexId],
) -> Result<(), DeleteVerticesError> {
    if !crate::session::is_canonical_vertex_set(vertices) {
        return Err(DeleteVerticesError::NonCanonicalVertexSet);
    }
    for &vertex in vertices {
        let Some(record) = session.mesh().vertices.get(vertex.as_id()) else {
            return Err(DeleteVerticesError::VertexNotLive {
                vertex: vertex.index(),
            });
        };
        if record.out != HalfEdgeId::INVALID || session.vertex_has_incident_half_edge(vertex) {
            return Err(DeleteVerticesError::VertexNotIsolated {
                vertex: vertex.index(),
            });
        }
    }
    for &vertex in vertices {
        let removed = session.mesh_mut().vertices.remove(vertex.as_id());
        debug_assert!(removed.is_some(), "validated vertex should remove");
        session.record_deleted_vertex(vertex);
    }
    session.invalidate_outgoing_index();
    Ok(())
}
