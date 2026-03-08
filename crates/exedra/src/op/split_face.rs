// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use crate::session::propagation::{
    propagate_split_face_diagonal_normals, propagate_split_face_diagonal_uvs,
    split_face_diagonal_sharpness,
};
use crate::{
    ChangeSink, CornerId, EditSession, FaceId, HalfEdge, HalfEdgeId, PropagatePolicy, attr,
};

use super::SplitFaceError;

/// Splits one interior face with a new diagonal.
pub fn split_face<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    corner_a: CornerId,
    corner_b: CornerId,
    policy: &PropagatePolicy,
) -> Result<FaceId, SplitFaceError> {
    if corner_a == corner_b {
        return Err(SplitFaceError::IdenticalCorners);
    }
    if session.mesh().half_edges.get(corner_a.as_id()).is_none() {
        return Err(SplitFaceError::CornerNotLive {
            corner: corner_a.index(),
        });
    }
    if session.mesh().half_edges.get(corner_b.as_id()).is_none() {
        return Err(SplitFaceError::CornerNotLive {
            corner: corner_b.index(),
        });
    }
    let face = session
        .mesh()
        .face(corner_a)
        .ok_or(SplitFaceError::CornerNotLive {
            corner: corner_a.index(),
        })?;
    let face_b = session
        .mesh()
        .face(corner_b)
        .ok_or(SplitFaceError::CornerNotLive {
            corner: corner_b.index(),
        })?;
    if face != face_b {
        return Err(SplitFaceError::CornersNotOnSameFace);
    }
    if face == FaceId::OUTSIDE {
        return Err(SplitFaceError::OutsideFaceNotAllowed);
    }

    let loop_edges = session.mesh().face_loop(face).collect::<Vec<_>>();
    let degree = loop_edges.len();
    let ia = loop_edges
        .iter()
        .position(|&corner| corner == corner_a)
        .ok_or(SplitFaceError::CornersNotOnSameFace)?;
    let ib = loop_edges
        .iter()
        .position(|&corner| corner == corner_b)
        .ok_or(SplitFaceError::CornersNotOnSameFace)?;
    let a_next_index = (ia + 1) % degree;
    let b_next_index = (ib + 1) % degree;
    if a_next_index == ib || b_next_index == ia {
        return Err(SplitFaceError::AdjacentCorners);
    }

    let corner_a_next = loop_edges[a_next_index];
    let corner_b_next = loop_edges[b_next_index];
    let vertex_a = session
        .mesh()
        .to_vertex(corner_a)
        .expect("validated corner has destination vertex");
    let vertex_b = session
        .mesh()
        .to_vertex(corner_b)
        .expect("validated corner has destination vertex");

    let mut new_face_path = Vec::<CornerId>::with_capacity(degree);
    let mut cursor = corner_a_next;
    for _ in 0..degree {
        new_face_path.push(cursor);
        if cursor == corner_b {
            break;
        }
        cursor = session
            .mesh()
            .next(cursor)
            .expect("face loop should have valid next pointers");
    }
    debug_assert_eq!(
        new_face_path.last().copied(),
        Some(corner_b),
        "face loop path should reach target corner"
    );

    let diagonal_a_b = HalfEdgeId::from(session.mesh_mut().half_edges.insert(HalfEdge {
        to: vertex_b,
        face,
        next: corner_b_next,
        twin: HalfEdgeId::INVALID,
    }));
    let new_face = FaceId::from(session.mesh_mut().faces.insert(crate::Face {
        edge: corner_b,
        degree: 0,
    }));
    let diagonal_b_a = HalfEdgeId::from(session.mesh_mut().half_edges.insert(HalfEdge {
        to: vertex_a,
        face: new_face,
        next: corner_a_next,
        twin: diagonal_a_b,
    }));
    session
        .mesh_mut()
        .half_edges
        .get_mut(diagonal_a_b.as_id())
        .expect("new diagonal half-edge must be live")
        .twin = diagonal_b_a;

    session
        .mesh_mut()
        .half_edges
        .get_mut(corner_a.as_id())
        .expect("validated corner must be live")
        .next = diagonal_a_b;
    session
        .mesh_mut()
        .half_edges
        .get_mut(corner_b.as_id())
        .expect("validated corner must be live")
        .next = diagonal_b_a;

    for &corner in &new_face_path {
        session
            .mesh_mut()
            .half_edges
            .get_mut(corner.as_id())
            .expect("validated loop corner must be live")
            .face = new_face;
    }
    let new_face_path_len =
        u32::try_from(new_face_path.len()).expect("face path length should fit u32");
    let face_degree = u32::try_from(degree).expect("face degree should fit u32");
    let new_face_degree = new_face_path_len + 1;
    let old_face_degree = face_degree - new_face_path_len + 1;
    {
        let source_face = session
            .mesh_mut()
            .faces
            .get_mut(face.as_id())
            .expect("source face must be live");
        source_face.edge = corner_a;
        source_face.degree = old_face_degree;
    }
    session
        .mesh_mut()
        .faces
        .get_mut(new_face.as_id())
        .expect("new face must be live")
        .degree = new_face_degree;

    if let Some(region_layer) = session.mesh_mut().attrs_mut().dense_mut(attr::FACE_REGION) {
        let region = region_layer.get(face.as_id()).copied().unwrap_or(0);
        let _ = region_layer.set(new_face.as_id(), region);
    }

    if session.mesh().attrs().sparse(attr::CORNER_UV).is_some() {
        let uv_a = session.corner_uv(corner_a);
        let uv_b = session.corner_uv(corner_b);
        propagate_split_face_diagonal_uvs(session, diagonal_a_b, diagonal_b_a, uv_a, uv_b, policy);
    }
    if session
        .mesh()
        .attrs()
        .sparse(attr::CORNER_NORMAL_OVERRIDE)
        .is_some()
    {
        let normal_a = session.corner_normal_override(corner_a);
        let normal_b = session.corner_normal_override(corner_b);
        propagate_split_face_diagonal_normals(
            session,
            diagonal_a_b,
            diagonal_b_a,
            normal_a,
            normal_b,
            policy,
        );
    }

    let source_sharp = session
        .edge_sharpness(corner_a)
        .unwrap_or(0.0)
        .max(session.edge_sharpness(corner_b).unwrap_or(0.0));
    let diagonal_sharp = split_face_diagonal_sharpness(source_sharp, policy);
    if diagonal_sharp > 0.0 {
        let _ = session.set_edge_sharpness_impl(diagonal_a_b, diagonal_sharp);
    }

    let vertex_slots = session.mesh().vertices.slot_count();
    let face_slots = session.mesh().faces.slot_count();
    let half_edge_slots = session.mesh().half_edges.slot_count();
    session
        .mesh_mut()
        .attrs
        .sync_capacities(vertex_slots, face_slots, half_edge_slots);

    session.record_created_face(new_face);
    session.record_created_half_edge(diagonal_a_b);
    session.record_created_half_edge(diagonal_b_a);
    session.mark_face_dirty(face);
    session.mark_face_dirty(new_face);
    session.mark_corner_dirty(corner_a);
    session.mark_corner_dirty(corner_b);
    session.mark_corner_dirty(diagonal_a_b);
    session.mark_corner_dirty(diagonal_b_a);
    for corner in loop_edges {
        if let Some(to) = session.mesh().to_vertex(corner) {
            session.mark_vertex_dirty(to);
        }
        session.mark_corner_dirty(corner);
    }

    session.invalidate_outgoing_index();
    Ok(new_face)
}
