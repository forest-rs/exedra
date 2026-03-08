// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::PositionPropagation;
use crate::session::propagation::{
    SplitEdgeNormalSources, SplitEdgeUvSources, capture_edge_tags, clear_edge_tags,
    propagate_split_edge_corner_normals, propagate_split_edge_corner_uvs,
    propagate_split_edge_edge_attrs,
};
use crate::session::{corner_normal_override_for_face_to_vertex, corner_uv_for_face_to_vertex};
use crate::{
    ChangeSink, EditSession, FaceId, HalfEdge, HalfEdgeId, PropagatePolicy, VertexId, attr,
};

use super::SplitEdgeError;

/// Splits one undirected edge by inserting a new vertex.
pub fn split_edge<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    half_edge: HalfEdgeId,
    policy: &PropagatePolicy,
) -> Result<VertexId, SplitEdgeError> {
    let twin = session
        .mesh()
        .twin(half_edge)
        .ok_or(SplitEdgeError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })?;
    if session.mesh().half_edges.get(half_edge.as_id()).is_none()
        || session.mesh().half_edges.get(twin.as_id()).is_none()
    {
        return Err(SplitEdgeError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        });
    }

    let from = session
        .mesh()
        .from_vertex(half_edge)
        .ok_or(SplitEdgeError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })?;
    let to = session
        .mesh()
        .to_vertex(half_edge)
        .ok_or(SplitEdgeError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })?;
    let h_next = session
        .mesh()
        .next(half_edge)
        .ok_or(SplitEdgeError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })?;
    let t_next = session
        .mesh()
        .next(twin)
        .ok_or(SplitEdgeError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })?;

    let from_pos =
        *session
            .mesh()
            .vertex_position(from)
            .ok_or(SplitEdgeError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;
    let to_pos = *session
        .mesh()
        .vertex_position(to)
        .ok_or(SplitEdgeError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })?;
    let midpoint = [
        0.5 * (from_pos[0] + to_pos[0]),
        0.5 * (from_pos[1] + to_pos[1]),
        0.5 * (from_pos[2] + to_pos[2]),
    ];

    let new_vertex = session.add_vertex_impl(match policy.position {
        PositionPropagation::Midpoint | PositionPropagation::WeightedMidpoint => midpoint,
    });

    let h_face = session.mesh().face(half_edge).expect("validated edge face");
    let t_face = session.mesh().face(twin).expect("validated twin face");
    let old_canonical_edge = core::cmp::min(half_edge, twin);

    let old_uv_h = session.corner_uv(half_edge);
    let old_uv_t = session.corner_uv(twin);
    let uv_a_fh = corner_uv_for_face_to_vertex(session.mesh(), h_face, from).unwrap_or([0.0, 0.0]);
    let uv_b_ft = corner_uv_for_face_to_vertex(session.mesh(), t_face, to).unwrap_or([0.0, 0.0]);
    let old_normal_h = session.corner_normal_override(half_edge);
    let old_normal_t = session.corner_normal_override(twin);
    let normal_a_fh = corner_normal_override_for_face_to_vertex(session.mesh(), h_face, from);
    let normal_b_ft = corner_normal_override_for_face_to_vertex(session.mesh(), t_face, to);

    let child_h = HalfEdgeId::from(session.mesh_mut().half_edges.insert(HalfEdge {
        to,
        face: h_face,
        next: h_next,
        twin,
    }));
    let child_t = HalfEdgeId::from(session.mesh_mut().half_edges.insert(HalfEdge {
        to: from,
        face: t_face,
        next: t_next,
        twin: half_edge,
    }));

    {
        let edge = session
            .mesh_mut()
            .half_edges
            .get_mut(half_edge.as_id())
            .expect("validated half-edge");
        edge.to = new_vertex;
        edge.next = child_h;
        edge.twin = child_t;
    }
    {
        let edge = session
            .mesh_mut()
            .half_edges
            .get_mut(twin.as_id())
            .expect("validated twin");
        edge.to = new_vertex;
        edge.next = child_t;
        edge.twin = child_h;
    }
    session
        .mesh_mut()
        .vertices
        .get_mut(new_vertex.as_id())
        .expect("new vertex must be live")
        .out = child_h;
    let vertex_slots = session.mesh().vertices.slot_count();
    let face_slots = session.mesh().faces.slot_count();
    let half_edge_slots = session.mesh().half_edges.slot_count();
    session
        .mesh_mut()
        .attrs
        .sync_capacities(vertex_slots, face_slots, half_edge_slots);

    session.record_created_half_edge(child_h);
    session.record_created_half_edge(child_t);
    session.mark_corner_dirty(half_edge);
    session.mark_corner_dirty(twin);
    session.mark_corner_dirty(child_h);
    session.mark_corner_dirty(child_t);
    session.mark_vertex_dirty(from);
    session.mark_vertex_dirty(to);
    session.mark_vertex_dirty(new_vertex);

    let edge_tags = capture_edge_tags(session.mesh_mut(), old_canonical_edge);
    clear_edge_tags(session.mesh_mut(), old_canonical_edge);
    propagate_split_edge_edge_attrs(session, half_edge, child_h, edge_tags, policy);

    if session.mesh().attrs().sparse(attr::CORNER_UV).is_some() {
        propagate_split_edge_corner_uvs(
            session,
            half_edge,
            twin,
            child_h,
            child_t,
            SplitEdgeUvSources {
                old_uv_h,
                old_uv_t,
                uv_a_fh,
                uv_b_ft,
            },
            policy,
        );
    }
    if session
        .mesh()
        .attrs()
        .sparse(attr::CORNER_NORMAL_OVERRIDE)
        .is_some()
    {
        propagate_split_edge_corner_normals(
            session,
            half_edge,
            twin,
            child_h,
            child_t,
            SplitEdgeNormalSources {
                old_normal_h,
                old_normal_t,
                normal_a_fh,
                normal_b_ft,
            },
            policy,
        );
    }

    if h_face != FaceId::OUTSIDE {
        if let Some(face) = session.mesh_mut().faces.get_mut(h_face.as_id()) {
            face.degree = face.degree.saturating_add(1);
        }
        session.mark_face_dirty(h_face);
    }
    if t_face != FaceId::OUTSIDE && t_face != h_face {
        if let Some(face) = session.mesh_mut().faces.get_mut(t_face.as_id()) {
            face.degree = face.degree.saturating_add(1);
        }
        session.mark_face_dirty(t_face);
    }

    session.invalidate_outgoing_index();
    Ok(new_vertex)
}
