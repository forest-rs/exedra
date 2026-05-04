// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::attr;
use crate::{ChangeSink, EditSession, FaceId, HalfEdgeId, VertexId, op};

use super::DissolveEdgesError;

#[derive(Copy, Clone, Debug, Default)]
struct BoundaryCornerData {
    uv: Option<[f32; 2]>,
    seam: Option<bool>,
    sharpness: Option<f32>,
}

/// Dissolves a canonical set of pairwise face-disjoint interior edges.
pub fn dissolve_edges<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    edges: &[HalfEdgeId],
) -> Result<Vec<FaceId>, DissolveEdgesError> {
    validate_edge_set(session, edges)?;
    let mut touched_faces = BTreeSet::<FaceId>::new();
    for &half_edge in edges {
        let twin = live_twin(session, half_edge)?;
        let face = live_face(session, half_edge)?;
        let twin_face = live_face(session, twin)?;
        if face == FaceId::OUTSIDE || twin_face == FaceId::OUTSIDE {
            return Err(DissolveEdgesError::BoundaryEdgeNotDissolvable {
                half_edge: half_edge.index(),
            });
        }
        if !touched_faces.insert(face) || !touched_faces.insert(twin_face) {
            return Err(DissolveEdgesError::OverlappingEdgeSet);
        }
    }

    let mut merged_faces = Vec::with_capacity(edges.len());
    for &half_edge in edges {
        let plan = build_merge_plan(session.mesh(), half_edge)?;
        let delete_faces = if plan.face_a < plan.face_b {
            [plan.face_a, plan.face_b]
        } else {
            [plan.face_b, plan.face_a]
        };
        op::delete_faces(session, &delete_faces, crate::DeletePolicy::KeepIsolated)
            .map_err(DissolveEdgesError::FaceDeleteFailed)?;
        let merged = op::add_face(session, &plan.loop_vertices)
            .map_err(DissolveEdgesError::FaceCreateFailed)?;
        if let Some(region) = plan.region {
            let _ = op::set_face_region(session, merged, region);
        }
        let new_corners = session.mesh().face_loop(merged).collect::<Vec<_>>();
        for (corner, attrs) in new_corners.into_iter().zip(plan.corner_data) {
            if let Some(uv) = attrs.uv {
                let _ = op::set_corner_uv(session, corner, uv);
            }
            if let Some(seam) = attrs.seam {
                let _ = op::set_edge_seam(session, corner, seam);
            }
            if let Some(sharpness) = attrs.sharpness {
                let _ = op::set_edge_sharpness(session, corner, sharpness);
            }
        }
        merged_faces.push(merged);
    }
    Ok(merged_faces)
}

fn validate_edge_set<S: ChangeSink>(
    session: &EditSession<'_, S>,
    edges: &[HalfEdgeId],
) -> Result<(), DissolveEdgesError> {
    let mut previous = None;
    for &half_edge in edges {
        if let Some(prev) = previous
            && prev >= half_edge
        {
            return Err(DissolveEdgesError::NonCanonicalEdgeSet);
        }
        previous = Some(half_edge);
        let twin = live_twin(session, half_edge)?;
        if core::cmp::min(half_edge, twin) != half_edge {
            return Err(DissolveEdgesError::NonCanonicalEdgeSet);
        }
    }
    Ok(())
}

fn live_twin<S: ChangeSink>(
    session: &EditSession<'_, S>,
    half_edge: HalfEdgeId,
) -> Result<HalfEdgeId, DissolveEdgesError> {
    session
        .mesh()
        .twin(half_edge)
        .ok_or(DissolveEdgesError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })
}

fn live_face<S: ChangeSink>(
    session: &EditSession<'_, S>,
    half_edge: HalfEdgeId,
) -> Result<FaceId, DissolveEdgesError> {
    session
        .mesh()
        .face(half_edge)
        .ok_or(DissolveEdgesError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })
}

struct MergePlan {
    face_a: FaceId,
    face_b: FaceId,
    loop_vertices: Vec<VertexId>,
    corner_data: Vec<BoundaryCornerData>,
    region: Option<u32>,
}

fn build_merge_plan(
    mesh: &crate::Mesh,
    half_edge: HalfEdgeId,
) -> Result<MergePlan, DissolveEdgesError> {
    let twin = mesh
        .twin(half_edge)
        .ok_or(DissolveEdgesError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })?;
    let face_a = mesh
        .face(half_edge)
        .ok_or(DissolveEdgesError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })?;
    let face_b = mesh.face(twin).ok_or(DissolveEdgesError::HalfEdgeNotLive {
        half_edge: half_edge.index(),
    })?;
    if face_a == FaceId::OUTSIDE || face_b == FaceId::OUTSIDE {
        return Err(DissolveEdgesError::BoundaryEdgeNotDissolvable {
            half_edge: half_edge.index(),
        });
    }
    let start = mesh
        .from_vertex(half_edge)
        .ok_or(DissolveEdgesError::HalfEdgeNotLive {
            half_edge: half_edge.index(),
        })?;
    let path_b = path_excluding_corner(mesh, twin)?;
    let path_a = path_excluding_corner(mesh, half_edge)?;
    if path_b.is_empty() || path_a.is_empty() {
        return Err(DissolveEdgesError::MergedLoopTooShort {
            half_edge: half_edge.index(),
        });
    }

    let mut corner_data = Vec::<BoundaryCornerData>::with_capacity(path_a.len() + path_b.len());
    corner_data.extend(
        path_b
            .iter()
            .map(|&corner| boundary_corner_data(mesh, corner)),
    );
    corner_data.extend(
        path_a
            .iter()
            .map(|&corner| boundary_corner_data(mesh, corner)),
    );

    let mut loop_vertices = Vec::<VertexId>::with_capacity(corner_data.len());
    loop_vertices.push(start);
    for corner in path_b
        .iter()
        .chain(path_a.iter())
        .take(corner_data.len() - 1)
    {
        let vertex = mesh
            .to_vertex(*corner)
            .ok_or(DissolveEdgesError::HalfEdgeNotLive {
                half_edge: half_edge.index(),
            })?;
        loop_vertices.push(vertex);
    }
    if loop_vertices.len() < 3 {
        return Err(DissolveEdgesError::MergedLoopTooShort {
            half_edge: half_edge.index(),
        });
    }
    let mut seen = BTreeSet::<VertexId>::new();
    for &vertex in &loop_vertices {
        if !seen.insert(vertex) {
            return Err(DissolveEdgesError::MergedLoopRepeatedVertex {
                half_edge: half_edge.index(),
                vertex: vertex.index(),
            });
        }
    }

    let region_a = mesh
        .attrs()
        .dense(attr::FACE_REGION)
        .and_then(|layer| layer.get(face_a.as_id()).copied());
    let region_b = mesh
        .attrs()
        .dense(attr::FACE_REGION)
        .and_then(|layer| layer.get(face_b.as_id()).copied());
    let region = match (region_a, region_b) {
        (Some(a), Some(b)) if a == b => Some(a),
        _ => None,
    };

    Ok(MergePlan {
        face_a,
        face_b,
        loop_vertices,
        corner_data,
        region,
    })
}

fn path_excluding_corner(
    mesh: &crate::Mesh,
    corner: HalfEdgeId,
) -> Result<Vec<HalfEdgeId>, DissolveEdgesError> {
    let mut path = Vec::<HalfEdgeId>::new();
    let mut current = mesh
        .next(corner)
        .ok_or(DissolveEdgesError::HalfEdgeNotLive {
            half_edge: corner.index(),
        })?;
    let face = mesh
        .face(corner)
        .ok_or(DissolveEdgesError::HalfEdgeNotLive {
            half_edge: corner.index(),
        })?;
    let max_steps = mesh
        .face_edge(face)
        .map(|_| mesh.face_loop(face).count())
        .unwrap_or(0);
    let mut steps = 0_usize;
    while current != corner {
        path.push(current);
        current = mesh
            .next(current)
            .ok_or(DissolveEdgesError::HalfEdgeNotLive {
                half_edge: corner.index(),
            })?;
        steps += 1;
        if steps > max_steps {
            return Err(DissolveEdgesError::HalfEdgeNotLive {
                half_edge: corner.index(),
            });
        }
    }
    Ok(path)
}

fn boundary_corner_data(mesh: &crate::Mesh, corner: HalfEdgeId) -> BoundaryCornerData {
    BoundaryCornerData {
        uv: mesh
            .attrs()
            .sparse(attr::CORNER_UV)
            .and_then(|layer| layer.get(corner.as_id()).copied()),
        seam: mesh.edge_seam(corner),
        sharpness: mesh.edge_sharpness(corner),
    }
}
