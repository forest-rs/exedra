// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use crate::session::{
    clear_deleted_corner_attrs, collect_half_edge_vertices, contains_face, find_outgoing_half_edge,
    preflight_boundary_continuation, sort_dedup, stitch_outside_loops_for_vertices,
};
use crate::{ChangeSink, DeletePolicy, EditSession, FaceId, HalfEdgeId, VertexId};

use super::DeleteFacesError;

/// Deletes a canonical set of interior faces.
pub fn delete_faces<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    faces: &[FaceId],
    policy: DeletePolicy,
) -> Result<(), DeleteFacesError> {
    if !crate::session::is_canonical_face_set(faces) {
        return Err(DeleteFacesError::NonCanonicalFaceSet);
    }
    for &face in faces {
        if face == FaceId::OUTSIDE {
            return Err(DeleteFacesError::OutsideFaceNotAllowed);
        }
        if session.mesh().faces.get(face.as_id()).is_none() {
            return Err(DeleteFacesError::FaceNotLive { face: face.index() });
        }
    }
    if faces.is_empty() {
        return Ok(());
    }

    let mut dirty_faces = Vec::<FaceId>::new();
    let mut dirty_vertices = Vec::<VertexId>::new();
    let mut boundary_replacements = Vec::<HalfEdgeId>::new();
    let mut deleted_half_edges = Vec::<HalfEdgeId>::new();

    for &face in faces {
        let loop_edges = session.mesh().face_loop(face).collect::<Vec<_>>();
        for half_edge in loop_edges {
            let twin = session
                .mesh()
                .twin(half_edge)
                .expect("valid mesh must provide a twin for each half-edge");
            let twin_face = session
                .mesh()
                .face(twin)
                .expect("valid mesh must provide an owning face for each half-edge");
            deleted_half_edges.push(half_edge);
            if twin_face == FaceId::OUTSIDE {
                deleted_half_edges.push(twin);
            } else if !contains_face(faces, twin_face) {
                boundary_replacements.push(twin);
                dirty_faces.push(twin_face);
            }
            collect_half_edge_vertices(session.mesh(), half_edge, &mut dirty_vertices);
        }
    }
    sort_dedup(&mut deleted_half_edges);
    sort_dedup(&mut boundary_replacements);
    sort_dedup(&mut dirty_faces);
    sort_dedup(&mut dirty_vertices);
    preflight_boundary_continuation(session.mesh(), &deleted_half_edges, &boundary_replacements)?;

    for &face in faces {
        let removed = session.mesh_mut().faces.remove(face.as_id());
        debug_assert!(removed.is_some(), "validated face should remove");
        session.record_deleted_face(face);
    }
    for half_edge in deleted_half_edges {
        let removed = session.mesh_mut().half_edges.remove(half_edge.as_id());
        debug_assert!(removed.is_some(), "validated half-edge should remove");
        session.record_deleted_half_edge(half_edge);
        clear_deleted_corner_attrs(session.mesh_mut(), half_edge);
    }

    for twin in boundary_replacements {
        let from = session
            .mesh()
            .from_vertex(twin)
            .expect("surviving interior half-edge must have an origin");
        let boundary = HalfEdgeId::from(session.mesh_mut().half_edges.insert(crate::HalfEdge {
            to: from,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin,
        }));
        session
            .mesh_mut()
            .half_edges
            .get_mut(twin.as_id())
            .expect("surviving interior half-edge must be live")
            .twin = boundary;
        session.record_created_half_edge(boundary);
        collect_half_edge_vertices(session.mesh(), boundary, &mut dirty_vertices);
    }
    sort_dedup(&mut dirty_vertices);

    // Deleting and replacing half-edges makes any index inherited from an
    // earlier operation in this edit session stale. Rebuild it once, then use
    // its directional boundary caches to repair only continuations at changed
    // vertices. An OUTSIDE `next` link is determined entirely by the incoming
    // and outgoing boundary edges at its shared vertex, so untouched vertices
    // cannot need repair. This replaces the former whole-mesh collect, sort,
    // and stitch pass and leaves the rebuilt index available for `out` repair.
    session.invalidate_outgoing_index();
    let _ = session.ensure_outgoing_index();
    stitch_outside_loops_for_vertices(session, &dirty_vertices);

    for face in dirty_faces {
        session.mark_face_dirty(face);
        let corners = session.mesh().face_loop(face).collect::<Vec<_>>();
        for corner in corners {
            session.mark_corner_dirty(corner);
        }
    }
    let mut isolated = Vec::<VertexId>::new();
    for &vertex in &dirty_vertices {
        let new_out = find_outgoing_half_edge(session.ensure_outgoing_index(), vertex);
        let Some(record) = session.mesh_mut().vertices.get_mut(vertex.as_id()) else {
            continue;
        };
        let previous = record.out;
        record.out = new_out.unwrap_or(HalfEdgeId::INVALID);
        if record.out != previous {
            session.mark_vertex_dirty(vertex);
        }
        if new_out.is_none() {
            isolated.push(vertex);
        }
    }
    for vertex in dirty_vertices {
        session.mark_vertex_dirty(vertex);
    }

    if policy == DeletePolicy::CleanupIsolated {
        for vertex in isolated {
            if session.mesh_mut().vertices.remove(vertex.as_id()).is_some() {
                session.record_deleted_vertex(vertex);
            }
        }
    }
    Ok(())
}
