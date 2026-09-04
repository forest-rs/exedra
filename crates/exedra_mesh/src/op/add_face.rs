// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use crate::session::{FaceEdgeUse, stitch_outside_loops_for_vertices};
use crate::{ChangeSink, EditSession, FaceId, HalfEdge, HalfEdgeId, VertexId};

use super::AddFaceError;

/// Adds one interior face from an ordered loop of live vertices.
pub fn add_face<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    loop_vertices: &[VertexId],
) -> Result<FaceId, AddFaceError> {
    if loop_vertices.len() < 3 {
        return Err(AddFaceError::LoopTooShort);
    }
    for &vertex in loop_vertices {
        if session.mesh().vertices.get(vertex.as_id()).is_none() {
            return Err(AddFaceError::VertexNotLive {
                vertex: vertex.index(),
            });
        }
    }
    for pair in loop_vertices.windows(2) {
        if pair[0] == pair[1] {
            return Err(AddFaceError::ZeroLengthEdge {
                from: pair[0].index(),
                to: pair[1].index(),
            });
        }
    }
    if loop_vertices.first() == loop_vertices.last() {
        let first = loop_vertices[0];
        return Err(AddFaceError::ZeroLengthEdge {
            from: first.index(),
            to: first.index(),
        });
    }
    for i in 0..loop_vertices.len() {
        let a = loop_vertices[i];
        for &b in &loop_vertices[(i + 1)..] {
            if a == b {
                return Err(AddFaceError::RepeatedVertex { vertex: a.index() });
            }
        }
    }

    let mut reuse_boundary = Vec::<Option<HalfEdgeId>>::with_capacity(loop_vertices.len());
    for i in 0..loop_vertices.len() {
        let from = loop_vertices[i];
        let to = loop_vertices[(i + 1) % loop_vertices.len()];
        let reuse = match session.face_edge_use(from, to) {
            FaceEdgeUse::Boundary(half_edge) => Some(half_edge),
            FaceEdgeUse::Occupied => {
                return Err(AddFaceError::NonManifoldEdge {
                    a: u32::min(from.index(), to.index()),
                    b: u32::max(from.index(), to.index()),
                });
            }
            FaceEdgeUse::Vacant => None,
        };
        reuse_boundary.push(reuse);
    }
    if let Some(vertex) =
        session.first_non_manifold_vertex_after_face(loop_vertices, &reuse_boundary)
    {
        return Err(AddFaceError::NonManifoldVertex {
            vertex: vertex.index(),
        });
    }

    let degree = u32::try_from(loop_vertices.len()).expect("face loop length should fit into u32");
    let face = FaceId::from(session.mesh_mut().faces.insert(crate::Face {
        edge: HalfEdgeId::INVALID,
        degree,
    }));
    let mut loop_half_edges = Vec::<HalfEdgeId>::with_capacity(loop_vertices.len());
    let mut changed_half_edges = Vec::<HalfEdgeId>::with_capacity(loop_vertices.len() * 2);
    for i in 0..loop_vertices.len() {
        let from = loop_vertices[i];
        let to = loop_vertices[(i + 1) % loop_vertices.len()];
        if let Some(boundary) = reuse_boundary[i] {
            session
                .mesh_mut()
                .half_edges
                .get_mut(boundary.as_id())
                .expect("preflight-validated boundary half-edge must be live")
                .face = face;
            loop_half_edges.push(boundary);
            changed_half_edges.push(boundary);
            session.mark_corner_dirty(boundary);
            continue;
        }

        let interior = HalfEdgeId::from(session.mesh_mut().half_edges.insert(HalfEdge {
            to,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let boundary = HalfEdgeId::from(session.mesh_mut().half_edges.insert(HalfEdge {
            to: from,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: interior,
        }));
        session
            .mesh_mut()
            .half_edges
            .get_mut(interior.as_id())
            .expect("new interior half-edge must be live")
            .twin = boundary;
        loop_half_edges.push(interior);
        changed_half_edges.push(interior);
        changed_half_edges.push(boundary);
        session.record_created_half_edge(interior);
        session.record_created_half_edge(boundary);
        session.mark_corner_dirty(interior);
        session.mark_corner_dirty(boundary);
    }

    for i in 0..loop_half_edges.len() {
        let current = loop_half_edges[i];
        let next = loop_half_edges[(i + 1) % loop_half_edges.len()];
        session
            .mesh_mut()
            .half_edges
            .get_mut(current.as_id())
            .expect("new face loop half-edge must be live")
            .next = next;
    }
    session
        .mesh_mut()
        .faces
        .get_mut(face.as_id())
        .expect("new face must be live")
        .edge = loop_half_edges[0];

    // Preflight built a valid outgoing index. Patch only the reused/new edges
    // before stitching so a batch of face additions does not repeatedly sort
    // the complete mesh and boundary topology.
    session.refresh_outgoing_index(&changed_half_edges);
    stitch_outside_loops_for_vertices(session, loop_vertices);
    let vertex_slots = session.mesh().vertices.slot_count();
    let face_slots = session.mesh().faces.slot_count();
    let half_edge_slots = session.mesh().half_edges.slot_count();
    session
        .mesh_mut()
        .attrs
        .sync_capacities(vertex_slots, face_slots, half_edge_slots);

    session.record_created_face(face);
    session.mark_face_dirty(face);
    for &vertex in loop_vertices {
        session.mark_vertex_dirty(vertex);
    }
    for &corner in &loop_half_edges {
        session.mark_corner_dirty(corner);
        if let Some(twin) = session.mesh().twin(corner) {
            session.mark_corner_dirty(twin);
        }
    }

    Ok(face)
}
