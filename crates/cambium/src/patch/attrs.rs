// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use exedra::{EdgeAttrPropagation, FaceId, HalfEdgeId, VertexId, op};

#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct SourceEdgeAttrs {
    pub(crate) seam: Option<bool>,
    pub(crate) sharpness: Option<f32>,
}

pub(crate) fn propagate_face_corner_uvs<S: exedra::ChangeSink>(
    txn: &mut exedra::EditSession<'_, S>,
    face: FaceId,
    uv_map: &[(VertexId, Option<[f32; 2]>)],
) {
    let corners = txn.mesh().face_loop(face).collect::<Vec<_>>();
    for corner in corners {
        let Some(to_vertex) = txn.mesh().to_vertex(corner) else {
            continue;
        };
        let uv = uv_map
            .iter()
            .find_map(|(vertex, uv)| (*vertex == to_vertex).then_some(*uv))
            .flatten();
        if let Some(uv) = uv {
            let _ = op::set_corner_uv(txn, corner, uv);
        }
    }
}

pub(crate) fn propagate_edge_attrs_for_vertices<S: exedra::ChangeSink>(
    txn: &mut exedra::EditSession<'_, S>,
    face: FaceId,
    a: VertexId,
    b: VertexId,
    source: SourceEdgeAttrs,
    policy: &exedra::PropagatePolicy,
) {
    let Some(corner) = find_face_edge_for_vertices(txn.mesh(), face, a, b) else {
        return;
    };
    match policy.edge_attr {
        EdgeAttrPropagation::Clear => {
            let _ = op::set_edge_seam(txn, corner, false);
            let _ = op::set_edge_sharpness(txn, corner, 0.0);
        }
        EdgeAttrPropagation::Inherit => {
            let seam = source.seam.unwrap_or(false);
            let sharpness = source.sharpness.unwrap_or(0.0);
            let _ = op::set_edge_seam(txn, corner, seam);
            let _ = op::set_edge_sharpness(txn, corner, sharpness);
        }
        EdgeAttrPropagation::DecayOnSplit => {
            let seam = source.seam.unwrap_or(false);
            let sharpness = source.sharpness.map_or(0.0, |value| (value - 1.0).max(0.0));
            let _ = op::set_edge_seam(txn, corner, seam);
            let _ = op::set_edge_sharpness(txn, corner, sharpness);
        }
    }
}

pub(crate) fn propagate_frame_edge_attrs<S: exedra::ChangeSink>(
    txn: &mut exedra::EditSession<'_, S>,
    face: FaceId,
    current: VertexId,
    next: VertexId,
    current_inner: VertexId,
    next_inner: VertexId,
    source: SourceEdgeAttrs,
    policy: &exedra::PropagatePolicy,
) {
    propagate_edge_attrs_for_vertices(txn, face, current, next, source, policy);
    propagate_edge_attrs_for_vertices(txn, face, current_inner, next_inner, source, policy);
    propagate_edge_attrs_for_vertices(
        txn,
        face,
        current,
        current_inner,
        SourceEdgeAttrs::default(),
        policy,
    );
    propagate_edge_attrs_for_vertices(
        txn,
        face,
        next,
        next_inner,
        SourceEdgeAttrs::default(),
        policy,
    );
}

fn find_face_edge_for_vertices(
    mesh: &exedra::Mesh,
    face: FaceId,
    a: VertexId,
    b: VertexId,
) -> Option<HalfEdgeId> {
    mesh.face_loop(face).find(|&corner| {
        let Some(from) = mesh.from_vertex(corner) else {
            return false;
        };
        let Some(to) = mesh.to_vertex(corner) else {
            return false;
        };
        (from == a && to == b) || (from == b && to == a)
    })
}
