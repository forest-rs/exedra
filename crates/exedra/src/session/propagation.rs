// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared attribute propagation helpers for topology kernels.

use super::*;

pub(crate) fn capture_edge_tags(
    mesh: &Mesh,
    canonical_edge: HalfEdgeId,
) -> (Option<bool>, Option<f32>) {
    let seam = mesh
        .attrs()
        .sparse(attr::EDGE_SEAM)
        .and_then(|layer| layer.get(canonical_edge.as_id()).copied());
    let sharp = mesh
        .attrs()
        .sparse(attr::EDGE_SHARPNESS)
        .and_then(|layer| layer.get(canonical_edge.as_id()).copied());
    (seam, sharp)
}

pub(crate) fn clear_edge_tags(mesh: &mut Mesh, canonical_edge: HalfEdgeId) {
    if let Some(layer) = mesh.attrs_mut().sparse_mut(attr::EDGE_SEAM) {
        let _ = layer.remove(canonical_edge.as_id());
    }
    if let Some(layer) = mesh.attrs_mut().sparse_mut(attr::EDGE_SHARPNESS) {
        let _ = layer.remove(canonical_edge.as_id());
    }
}

pub(crate) fn propagate_split_edge_edge_attrs(
    session: &mut EditSession<'_, impl ChangeSink>,
    parent: HalfEdgeId,
    child: HalfEdgeId,
    edge_tags: (Option<bool>, Option<f32>),
    policy: &PropagatePolicy,
) {
    let (seam, sharp) = edge_tags;
    match policy.edge_attr {
        EdgeAttrPropagation::Inherit => {
            if let Some(seam) = seam {
                let _ = session.set_edge_seam_impl(parent, seam);
                let _ = session.set_edge_seam_impl(child, seam);
            }
            if let Some(sharp) = sharp {
                let _ = session.set_edge_sharpness_impl(parent, sharp);
                let _ = session.set_edge_sharpness_impl(child, sharp);
            }
        }
        EdgeAttrPropagation::DecayOnSplit => {
            if let Some(seam) = seam {
                let _ = session.set_edge_seam_impl(parent, seam);
                let _ = session.set_edge_seam_impl(child, seam);
            }
            if let Some(sharp) = sharp {
                let split_sharp = (sharp - 1.0).max(0.0);
                let _ = session.set_edge_sharpness_impl(parent, split_sharp);
                let _ = session.set_edge_sharpness_impl(child, split_sharp);
            }
        }
        EdgeAttrPropagation::Clear => {
            let _ = session.set_edge_seam_impl(parent, false);
            let _ = session.set_edge_seam_impl(child, false);
            let _ = session.set_edge_sharpness_impl(parent, 0.0);
            let _ = session.set_edge_sharpness_impl(child, 0.0);
        }
    }
}

pub(crate) struct SplitEdgeUvSources {
    pub(crate) old_uv_h: Option<[f32; 2]>,
    pub(crate) old_uv_t: Option<[f32; 2]>,
    pub(crate) uv_a_fh: [f32; 2],
    pub(crate) uv_b_ft: [f32; 2],
}

pub(crate) struct SplitEdgeNormalSources {
    pub(crate) old_normal_h: Option<[f32; 3]>,
    pub(crate) old_normal_t: Option<[f32; 3]>,
    pub(crate) normal_a_fh: Option<[f32; 3]>,
    pub(crate) normal_b_ft: Option<[f32; 3]>,
}

pub(crate) fn propagate_split_edge_corner_uvs(
    session: &mut EditSession<'_, impl ChangeSink>,
    half_edge: HalfEdgeId,
    twin: HalfEdgeId,
    child_h: HalfEdgeId,
    child_t: HalfEdgeId,
    sources: SplitEdgeUvSources,
    policy: &PropagatePolicy,
) {
    let SplitEdgeUvSources {
        old_uv_h,
        old_uv_t,
        uv_a_fh,
        uv_b_ft,
    } = sources;
    if let Some(uv) = old_uv_h {
        let _ = session.set_corner_uv_impl(child_h, uv);
    } else if let Some(layer) = session.mesh.attrs_mut().sparse_mut(attr::CORNER_UV) {
        let _ = layer.remove(child_h.as_id());
    }
    if let Some(uv) = old_uv_t {
        let _ = session.set_corner_uv_impl(child_t, uv);
    } else if let Some(layer) = session.mesh.attrs_mut().sparse_mut(attr::CORNER_UV) {
        let _ = layer.remove(child_t.as_id());
    }

    let h_uv_mid = match policy.uv {
        UvPropagation::Midpoint => {
            let uv_b_fh = old_uv_h.unwrap_or([0.0, 0.0]);
            [
                0.5 * (uv_a_fh[0] + uv_b_fh[0]),
                0.5 * (uv_a_fh[1] + uv_b_fh[1]),
            ]
        }
        UvPropagation::CopyFromSide => old_uv_h.unwrap_or([0.0, 0.0]),
    };
    let t_uv_mid = match policy.uv {
        UvPropagation::Midpoint => {
            let uv_a_ft = old_uv_t.unwrap_or([0.0, 0.0]);
            [
                0.5 * (uv_b_ft[0] + uv_a_ft[0]),
                0.5 * (uv_b_ft[1] + uv_a_ft[1]),
            ]
        }
        UvPropagation::CopyFromSide => old_uv_t.unwrap_or([0.0, 0.0]),
    };
    let _ = session.set_corner_uv_impl(half_edge, h_uv_mid);
    let _ = session.set_corner_uv_impl(twin, t_uv_mid);
}

pub(crate) fn propagate_split_edge_corner_normals(
    session: &mut EditSession<'_, impl ChangeSink>,
    half_edge: HalfEdgeId,
    twin: HalfEdgeId,
    child_h: HalfEdgeId,
    child_t: HalfEdgeId,
    sources: SplitEdgeNormalSources,
    policy: &PropagatePolicy,
) {
    let SplitEdgeNormalSources {
        old_normal_h,
        old_normal_t,
        normal_a_fh,
        normal_b_ft,
    } = sources;
    let _ = session.set_corner_normal_override_impl(child_h, old_normal_h);
    let _ = session.set_corner_normal_override_impl(child_t, old_normal_t);

    let h_mid = match policy.normal_override {
        NormalOverridePropagation::Clear => None,
        NormalOverridePropagation::CopyFromSide => old_normal_h,
        NormalOverridePropagation::Average => average_optional_vec3(normal_a_fh, old_normal_h),
    };
    let t_mid = match policy.normal_override {
        NormalOverridePropagation::Clear => None,
        NormalOverridePropagation::CopyFromSide => old_normal_t,
        NormalOverridePropagation::Average => average_optional_vec3(normal_b_ft, old_normal_t),
    };
    let _ = session.set_corner_normal_override_impl(half_edge, h_mid);
    let _ = session.set_corner_normal_override_impl(twin, t_mid);
}

pub(crate) fn propagate_split_face_diagonal_uvs(
    session: &mut EditSession<'_, impl ChangeSink>,
    diagonal_a_b: HalfEdgeId,
    diagonal_b_a: HalfEdgeId,
    uv_a: Option<[f32; 2]>,
    uv_b: Option<[f32; 2]>,
    policy: &PropagatePolicy,
) {
    let uv_diag = match policy.uv {
        UvPropagation::Midpoint => match (uv_a, uv_b) {
            (Some(a), Some(b)) => Some([0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])]),
            _ => None,
        },
        UvPropagation::CopyFromSide => uv_b,
    };
    let uv_twin = match policy.uv {
        UvPropagation::Midpoint => uv_diag,
        UvPropagation::CopyFromSide => uv_a,
    };
    if let Some(uv) = uv_diag {
        let _ = session.set_corner_uv_impl(diagonal_a_b, uv);
    } else if let Some(layer) = session.mesh.attrs_mut().sparse_mut(attr::CORNER_UV) {
        let _ = layer.remove(diagonal_a_b.as_id());
    }
    if let Some(uv) = uv_twin {
        let _ = session.set_corner_uv_impl(diagonal_b_a, uv);
    } else if let Some(layer) = session.mesh.attrs_mut().sparse_mut(attr::CORNER_UV) {
        let _ = layer.remove(diagonal_b_a.as_id());
    }
}

pub(crate) fn propagate_split_face_diagonal_normals(
    session: &mut EditSession<'_, impl ChangeSink>,
    diagonal_a_b: HalfEdgeId,
    diagonal_b_a: HalfEdgeId,
    normal_a: Option<[f32; 3]>,
    normal_b: Option<[f32; 3]>,
    policy: &PropagatePolicy,
) {
    let normal_diag = match policy.normal_override {
        NormalOverridePropagation::Clear => None,
        NormalOverridePropagation::CopyFromSide => normal_b,
        NormalOverridePropagation::Average => average_optional_vec3(normal_a, normal_b),
    };
    let normal_twin = match policy.normal_override {
        NormalOverridePropagation::Clear => None,
        NormalOverridePropagation::CopyFromSide => normal_a,
        NormalOverridePropagation::Average => normal_diag,
    };
    let _ = session.set_corner_normal_override_impl(diagonal_a_b, normal_diag);
    let _ = session.set_corner_normal_override_impl(diagonal_b_a, normal_twin);
}

pub(crate) fn split_face_diagonal_sharpness(source_sharp: f32, policy: &PropagatePolicy) -> f32 {
    match policy.edge_attr {
        EdgeAttrPropagation::Clear => 0.0,
        EdgeAttrPropagation::Inherit => 0.0,
        EdgeAttrPropagation::DecayOnSplit => (source_sharp - 1.0).max(0.0),
    }
}

fn average_optional_vec3(a: Option<[f32; 3]>, b: Option<[f32; 3]>) -> Option<[f32; 3]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([
            0.5 * (a[0] + b[0]),
            0.5 * (a[1] + b[1]),
            0.5 * (a[2] + b[2]),
        ]),
        _ => None,
    }
}
