// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::attr;
use crate::attributes::{CallerValues, Domain};
use crate::{ChangeSink, EditSession, FaceId, VertexId, op};

use super::DissolveVerticesError;

#[derive(Copy, Clone, Debug, Default)]
struct EdgeAttrs {
    seam: Option<bool>,
    sharpness: Option<f32>,
}

#[derive(Clone, Debug)]
struct RebuiltFacePlan {
    loop_vertices: Vec<VertexId>,
    corner_uvs: Vec<(VertexId, [f32; 2])>,
    perimeter_edges: Vec<(VertexId, VertexId, EdgeAttrs)>,
    region: Option<u32>,
    /// Caller-defined values of each kept corner, keyed by its vertex.
    corner_values: Vec<(VertexId, CallerValues)>,
    /// Caller-defined values of the rebuilt face.
    face_values: CallerValues,
}

#[derive(Clone, Debug)]
struct VertexDissolvePlan {
    faces: [FaceId; 2],
    rebuilt_faces: Vec<RebuiltFacePlan>,
}

/// Dissolves a canonical set of pairwise face-disjoint interior valence-2 vertices.
pub fn dissolve_vertices<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    vertices: &[VertexId],
) -> Result<Vec<FaceId>, DissolveVerticesError> {
    let (deleted_faces, plans) = build_plans(session.mesh(), vertices)?;
    op::delete_faces(session, &deleted_faces, crate::DeletePolicy::KeepIsolated)
        .map_err(DissolveVerticesError::FaceDeleteFailed)?;
    op::delete_vertices(session, vertices).map_err(DissolveVerticesError::VertexDeleteFailed)?;

    let mut rebuilt = Vec::<FaceId>::new();
    for plan in plans {
        for face_plan in plan.rebuilt_faces {
            let face = op::add_face(session, &face_plan.loop_vertices)
                .map_err(DissolveVerticesError::FaceCreateFailed)?;
            if let Some(region) = face_plan.region {
                let _ = op::set_face_region(session, face, region);
            }
            session.restore_caller_layers(Domain::Face, face.as_id(), &face_plan.face_values);
            let new_corners = session.mesh().face_loop(face).collect::<Vec<_>>();
            for corner in new_corners {
                let Some(to) = session.mesh().to_vertex(corner) else {
                    continue;
                };
                if let Some((_, values)) = face_plan
                    .corner_values
                    .iter()
                    .find(|(vertex, _)| *vertex == to)
                {
                    session.restore_caller_layers(Domain::HalfEdge, corner.as_id(), values);
                }
                if let Some((_, uv)) = face_plan
                    .corner_uvs
                    .iter()
                    .find(|(vertex, _)| *vertex == to)
                {
                    let _ = op::set_corner_uv(session, corner, *uv);
                }
                let Some(from) = session.mesh().from_vertex(corner) else {
                    continue;
                };
                if let Some((_, _, attrs)) = face_plan
                    .perimeter_edges
                    .iter()
                    .find(|(a, b, _)| *a == from && *b == to)
                {
                    if let Some(seam) = attrs.seam {
                        let _ = op::set_edge_seam(session, corner, seam);
                    }
                    if let Some(sharpness) = attrs.sharpness {
                        let _ = op::set_edge_sharpness(session, corner, sharpness);
                    }
                }
            }
            rebuilt.push(face);
        }
    }
    Ok(rebuilt)
}

/// Checks the canonical, live, face-disjoint interior valence-2 selection.
///
/// Uses the same topology and attribute preparation as [`dissolve_vertices`],
/// without editing. Kernel mutation failures remain possible during execution.
pub fn validate_dissolve_vertex_selection(
    mesh: &crate::Mesh,
    vertices: &[VertexId],
) -> Result<(), DissolveVerticesError> {
    build_plans(mesh, vertices).map(|_| ())
}

fn build_plans(
    mesh: &crate::Mesh,
    vertices: &[VertexId],
) -> Result<(Vec<FaceId>, Vec<VertexDissolvePlan>), DissolveVerticesError> {
    if !crate::session::is_canonical_vertex_set(vertices) {
        return Err(DissolveVerticesError::NonCanonicalVertexSet);
    }

    let mut touched_faces = BTreeSet::<FaceId>::new();
    let mut plans = Vec::<VertexDissolvePlan>::with_capacity(vertices.len());
    for &vertex in vertices {
        let plan = build_vertex_plan(mesh, vertex)?;
        for &face in &plan.faces {
            if !touched_faces.insert(face) {
                return Err(DissolveVerticesError::OverlappingVertexSet);
            }
        }
        plans.push(plan);
    }

    let mut deleted_faces = touched_faces.into_iter().collect::<Vec<_>>();
    deleted_faces.sort_unstable();
    Ok((deleted_faces, plans))
}

fn build_vertex_plan(
    mesh: &crate::Mesh,
    vertex: VertexId,
) -> Result<VertexDissolvePlan, DissolveVerticesError> {
    if mesh.vertices.get(vertex.as_id()).is_none() {
        return Err(DissolveVerticesError::VertexNotLive {
            vertex: vertex.index(),
        });
    }

    let mut star = mesh.vertex_star(vertex).collect::<Vec<_>>();
    star.sort_unstable();
    if star.len() != 2 {
        return Err(DissolveVerticesError::UnsupportedVertexDegree {
            vertex: vertex.index(),
            degree: star.len(),
        });
    }

    let mut faces = Vec::<FaceId>::with_capacity(2);
    let mut rebuilt_faces = Vec::<RebuiltFacePlan>::with_capacity(2);
    for half_edge in star {
        let face = mesh
            .face(half_edge)
            .ok_or(DissolveVerticesError::VertexNotLive {
                vertex: vertex.index(),
            })?;
        if face == FaceId::OUTSIDE {
            return Err(DissolveVerticesError::BoundaryVertexNotDissolvable {
                vertex: vertex.index(),
            });
        }
        if faces.contains(&face) {
            continue;
        }
        let degree = mesh.face_loop(face).count();
        if degree < 4 {
            return Err(DissolveVerticesError::IncidentFaceTooSmall {
                vertex: vertex.index(),
                face: face.index(),
                degree,
            });
        }
        rebuilt_faces.push(rebuild_face_without_vertex(mesh, face, vertex)?);
        faces.push(face);
    }

    if faces.len() != 2 {
        return Err(DissolveVerticesError::UnsupportedVertexTopology {
            vertex: vertex.index(),
        });
    }

    Ok(VertexDissolvePlan {
        faces: [faces[0], faces[1]],
        rebuilt_faces,
    })
}

fn rebuild_face_without_vertex(
    mesh: &crate::Mesh,
    face: FaceId,
    removed_vertex: VertexId,
) -> Result<RebuiltFacePlan, DissolveVerticesError> {
    let mut loop_vertices = Vec::<VertexId>::new();
    let mut corner_uvs = Vec::<(VertexId, [f32; 2])>::new();
    let mut corner_values = Vec::<(VertexId, CallerValues)>::new();
    let mut perimeter_edges = Vec::<(VertexId, VertexId, EdgeAttrs)>::new();

    for corner in mesh.face_loop(face) {
        let to =
            mesh.to_vertex(corner)
                .ok_or(DissolveVerticesError::UnsupportedVertexTopology {
                    vertex: removed_vertex.index(),
                })?;
        if to == removed_vertex {
            continue;
        }
        let from =
            mesh.from_vertex(corner)
                .ok_or(DissolveVerticesError::UnsupportedVertexTopology {
                    vertex: removed_vertex.index(),
                })?;
        loop_vertices.push(to);
        corner_values.push((
            to,
            mesh.attrs()
                .capture_caller(Domain::HalfEdge, corner.as_id()),
        ));
        if let Some(uv) = mesh
            .attrs()
            .sparse(attr::CORNER_UV)
            .and_then(|layer| layer.get(corner.as_id()).copied())
        {
            corner_uvs.push((to, uv));
        }
        if from != removed_vertex {
            perimeter_edges.push((
                from,
                to,
                EdgeAttrs {
                    seam: mesh.edge_seam(corner),
                    sharpness: mesh.edge_sharpness(corner),
                },
            ));
        }
    }

    let mut unique = BTreeSet::<VertexId>::new();
    for &vertex in &loop_vertices {
        if !unique.insert(vertex) {
            return Err(DissolveVerticesError::UnsupportedVertexTopology {
                vertex: removed_vertex.index(),
            });
        }
    }

    let region = mesh
        .attrs()
        .dense(attr::FACE_REGION)
        .and_then(|layer| layer.get(face.as_id()).copied());

    Ok(RebuiltFacePlan {
        loop_vertices,
        corner_uvs,
        perimeter_edges,
        region,
        corner_values,
        face_values: mesh.attrs().capture_caller(Domain::Face, face.as_id()),
    })
}
