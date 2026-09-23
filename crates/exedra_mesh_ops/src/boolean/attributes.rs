// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Corner UVs and caller-defined layers carried onto a Boolean result.
//!
//! Every output face records its operand and original pre-split face, and a
//! Boolean moves no points, so each output corner lies on its original face.
//! Corner UVs and caller-defined values are sampled there by barycentric
//! weights in the face's robust triangulation (see [`crate::layers`]): a
//! corner at an operand vertex keeps that corner's value exactly, a corner on
//! a split edge interpolates it. Cut-surface faces come from the cutting
//! operand and so carry its values. An output vertex that is an original
//! operand vertex (by the stitch's identity maps, never by position) takes
//! that vertex's values for the operand's layers. Each operand also samples
//! every other output vertex its faces reach, at the point on its first such
//! face, for its own layers. Identity samples are written after point
//! samples, so a vertex of one operand lying on a face of the other keeps its
//! own values; within each pass operand A writes last, so for a layer name
//! both operands carry A's value (or its absence) wins. Layer names only one
//! operand carries always take that operand's value.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use exedra_math::promote;
use exedra_mesh::{FaceId, HalfEdgeId, Mesh, VertexId, attr, op};

use super::split::MeshSide;
use crate::layers::{Chart, CornerSample, Transfer, VertexSample};

/// What carrying attributes onto a Boolean result could not do.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Carried {
    /// Output faces whose source face has an incomplete or non-finite UV
    /// chart, so their corners have no UV.
    pub(super) uv_unmapped_faces: u64,
    /// Caller-defined values whose layer rule is `Unspecified`.
    pub(super) unpropagated_attribute_values: u64,
}

/// The two operands' layers disagree on storage, value type, default or
/// propagation rule for one name, so the result cannot hold both.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(super) struct LayerConflict;

pub(super) fn carry(
    output: &mut Mesh,
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    provenance: &[(FaceId, MeshSide, FaceId)],
    vertex_provenance: &[(VertexId, MeshSide, VertexId)],
) -> Result<Carried, LayerConflict> {
    let mut carried = Carried {
        uv_unmapped_faces: carry_uvs(output, mesh_a, mesh_b, provenance),
        unpropagated_attribute_values: 0,
    };
    let mut transfer_a = Transfer::new(mesh_a);
    let mut transfer_b = Transfer::new(mesh_b);
    if transfer_a.is_none() && transfer_b.is_none() {
        return Ok(carried);
    }
    // Each operand samples every output vertex it reaches, for its own
    // layers: by identity where the vertex is one of its original vertices
    // (written by a separate identity pass), else at the point on its first
    // output face that reaches it. An operand without caller layers records
    // nothing.
    let mut identity_a = Transfer::new(mesh_a);
    let mut identity_b = Transfer::new(mesh_b);
    let mut originals = [
        BTreeMap::<VertexId, VertexId>::new(),
        BTreeMap::<VertexId, VertexId>::new(),
    ];
    let slot = |side: MeshSide| match side {
        MeshSide::A => 0,
        MeshSide::B => 1,
    };
    for &(output_vertex, side, source_vertex) in vertex_provenance {
        originals[slot(side)]
            .entry(output_vertex)
            .or_insert(source_vertex);
    }
    let mut points = [
        BTreeMap::<VertexId, (FaceId, [f64; 3])>::new(),
        BTreeMap::<VertexId, (FaceId, [f64; 3])>::new(),
    ];
    for &(face, side, source_face) in provenance {
        let transfer = match side {
            MeshSide::A => transfer_a.as_mut(),
            MeshSide::B => transfer_b.as_mut(),
        };
        let Some(transfer) = transfer else {
            continue;
        };
        let mut corners = Vec::new();
        for corner in output.face_loop(face) {
            let vertex = output.to_vertex(corner).expect("live output corner");
            let point = promote(*output.vertex_position(vertex).expect("live output vertex"));
            corners.push((corner, CornerSample::Point(point)));
            if !originals[slot(side)].contains_key(&vertex) {
                points[slot(side)]
                    .entry(vertex)
                    .or_insert((source_face, point));
            }
        }
        transfer.face(face, source_face, corners);
    }
    let [originals_a, originals_b] = originals;
    let [points_a, points_b] = points;
    for (transfer, samples) in [
        (transfer_a.as_mut(), points_a),
        (transfer_b.as_mut(), points_b),
    ] {
        if let Some(transfer) = transfer {
            for (vertex, (face, point)) in samples {
                transfer.vertex(vertex, VertexSample::Point { face, point });
            }
        }
    }
    for (transfer, samples) in [
        (identity_a.as_mut(), originals_a),
        (identity_b.as_mut(), originals_b),
    ] {
        if let Some(transfer) = transfer {
            for (vertex, source) in samples {
                transfer.vertex(vertex, VertexSample::Vertex(source));
            }
        }
    }
    // Adopt both operands' layers before writing either. A conflict fails the
    // whole Boolean, which then returns no mesh.
    for source in [mesh_a, mesh_b] {
        output
            .adopt_attribute_layers(source)
            .map_err(|_| LayerConflict)?;
    }
    // A restore writes only the layers its operand carries. Point samples go
    // first and identity samples last, so an original vertex keeps its own
    // operand's value; within each pass B writes before A, so for a layer
    // name both operands carry A's value (or its absence) wins.
    for transfer in [transfer_b, transfer_a, identity_b, identity_a]
        .into_iter()
        .flatten()
    {
        carried.unpropagated_attribute_values +=
            transfer.apply(output).map_err(|_| LayerConflict)?;
    }
    Ok(carried)
}

/// Interpolates operand corner UVs onto every output corner. Returns the
/// output faces whose source chart is incomplete or non-finite.
fn carry_uvs(
    output: &mut Mesh,
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    provenance: &[(FaceId, MeshSide, FaceId)],
) -> u64 {
    let uvs_a = mesh_a.attrs().sparse(attr::CORNER_UV);
    let uvs_b = mesh_b.attrs().sparse(attr::CORNER_UV);
    if uvs_a.is_none() && uvs_b.is_none() {
        return 0;
    }
    // Keyed by (operand B?, source face); `None` marks an incomplete chart,
    // decided once per source face.
    let mut charts = BTreeMap::<(bool, FaceId), Option<Chart>>::new();
    let mut writes = Vec::<(HalfEdgeId, [f32; 2])>::new();
    let mut unmapped = 0;
    for &(face, side, source_face) in provenance {
        let (mesh, uvs) = match side {
            MeshSide::A => (mesh_a, uvs_a),
            MeshSide::B => (mesh_b, uvs_b),
        };
        let Some(uvs) = uvs else {
            continue;
        };
        let chart = charts
            .entry((side == MeshSide::B, source_face))
            .or_insert_with(|| {
                // A partial chart has no defined interpolation.
                let complete = mesh.face_loop(source_face).all(|corner| {
                    uvs.get(corner.as_id())
                        .is_some_and(|uv| uv.iter().all(|v| v.is_finite()))
                });
                complete.then(|| Chart::new(mesh, source_face))
            });
        let Some(chart) = chart else {
            unmapped += 1;
            continue;
        };
        for corner in output.face_loop(face) {
            let vertex = output.to_vertex(corner).expect("live output corner");
            let point = promote(*output.vertex_position(vertex).expect("live output vertex"));
            let mut uv = [0.0_f32; 2];
            let mut total = 0.0_f32;
            for (source_corner, weight) in chart.weights(point) {
                let value = uvs.get(source_corner.as_id()).expect("complete chart");
                uv[0] += weight * value[0];
                uv[1] += weight * value[1];
                total += weight;
            }
            if total > 0.0 && uv.iter().all(|v| v.is_finite()) {
                writes.push((corner, uv));
            }
        }
    }
    if !writes.is_empty() {
        let mut edit = output.edit();
        for (corner, uv) in writes {
            let _ = op::set_corner_uv(&mut edit, corner, uv);
        }
        let _: () = edit.finish();
    }
    unmapped
}
