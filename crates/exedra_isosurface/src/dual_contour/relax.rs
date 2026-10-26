// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Unfolding faces that turn against the field.
//!
//! Across a crease, a quad split along the wrong diagonal bends the wrong
//! way; flipping the edge, when both new triangles then agree with the
//! gradient, fixes it without moving anything. Near corners, several cells
//! can each solve their vertex onto one feature line or plane, and the faces
//! between them turn edge-on: fins that no flip can fix, because the vertices
//! themselves are badly placed. Moving such a vertex to the middle of its
//! neighbors, then back onto the surface along the gradient, unfolds them,
//! at the cost of rounding the feature locally. A move is kept only when
//! every face around the vertex then agrees with the field.

use alloc::vec::Vec;
use exedra_math::dot;
use exedra_mesh::{FaceId, HalfEdgeId, Mesh, VertexId, op};
use hashbrown::{HashMap, HashSet};

use super::control::RunContext;
use super::{DualContourError, newell_normal};
use crate::ScalarField;

/// Sweeps over the inverted faces before giving up on the rest.
const PASSES: usize = 3;

/// Moves vertices of `inverted` faces where that unfolds every face around
/// them. Returns the moved vertices.
pub(super) fn relax_folds<F: ScalarField>(
    field: &F,
    mesh: &mut Mesh,
    inverted: &[FaceId],
    run: &RunContext,
) -> Result<Vec<VertexId>, DualContourError> {
    if inverted.is_empty() {
        return Ok(Vec::new());
    }
    let mut faces_of = HashMap::<VertexId, Vec<FaceId>>::new();
    for face in mesh.faces() {
        for corner in mesh.face_loop(face) {
            if let Some(vertex) = mesh.to_vertex(corner) {
                faces_of.entry(vertex).or_default().push(face);
            }
        }
    }
    let mut moved = Vec::new();
    let mut moved_set = HashSet::new();
    let mut pending = inverted.to_vec();
    for _ in 0..PASSES {
        let mut candidates = pending
            .iter()
            .flat_map(|&face| loop_vertices(mesh, face))
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        candidates.dedup();
        let mut changed = false;
        for vertex in candidates {
            let Some(faces) = faces_of.get(&vertex) else {
                continue;
            };
            let Some(target) = relaxed_position(field, mesh, vertex, faces, run)? else {
                continue;
            };
            if unfolds(field, mesh, vertex, target, faces, run)? {
                let mut session = mesh.edit();
                op::set_vertex_position(&mut session, vertex, target)
                    .expect("a vertex of a live face is live");
                let _: () = session.finish();
                if moved_set.insert(vertex) {
                    moved.push(vertex);
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
        // Faces still folded after this sweep are tried again.
        pending = still_inverted(field, mesh, &pending, run)?;
        if pending.is_empty() {
            break;
        }
    }
    Ok(moved)
}

/// Flips edges of `inverted` triangles where both new triangles agree with
/// the gradient at their centers. Returns the faces the flips rewired and
/// the faces still inverted.
pub(super) fn flip_folds<F: ScalarField>(
    field: &F,
    mesh: &mut Mesh,
    inverted: &[FaceId],
    run: &RunContext,
) -> Result<(Vec<FaceId>, Vec<FaceId>), DualContourError> {
    let mut rewired = Vec::new();
    let mut remaining = Vec::new();
    // One session for every flip, so the mesh's adjacency index is built
    // once and kept up to date, not rebuilt for each flip.
    let mut session = mesh.edit();
    for &face in inverted {
        if rewired.contains(&face) {
            continue;
        }
        let edges = session.mesh().face_loop(face).collect::<Vec<_>>();
        if edges.len() != 3 {
            remaining.push(face);
            continue;
        }
        let mut fixed = false;
        for edge in edges {
            let Some(neighbor) = flippable(session.mesh(), edge) else {
                continue;
            };
            if rewired.contains(&neighbor) {
                continue;
            }
            let Some([first, second]) = flipped_triangles(session.mesh(), edge) else {
                continue;
            };
            let points = [first, second];
            let centers = points.map(|triangle| center(&triangle));
            let mut gradients = [[0.0_f32; 4]; 2];
            field.eval_gradients(&centers, &mut gradients);
            run.check()?;
            let agrees = points.iter().zip(&gradients).all(|(triangle, gradient)| {
                let gradient = [gradient[1], gradient[2], gradient[3]].map(f64::from);
                super::triangle_is_nondegenerate(*triangle)
                    && dot(newell_normal(triangle), gradient) > 0.0
            });
            if agrees && op::flip_edge(&mut session, edge).is_ok() {
                rewired.extend([face, neighbor]);
                fixed = true;
                break;
            }
        }
        if !fixed {
            remaining.push(face);
        }
    }
    let _: () = session.finish();
    remaining.retain(|face| !rewired.contains(face));
    Ok((rewired, remaining))
}

/// The triangle across `edge`, when there is one.
fn flippable(mesh: &Mesh, edge: HalfEdgeId) -> Option<FaceId> {
    let twin = mesh.twin(edge)?;
    let neighbor = mesh.face(twin)?;
    (neighbor != FaceId::OUTSIDE && mesh.face_loop(neighbor).count() == 3).then_some(neighbor)
}

/// The two triangles a flip of `edge` would make: for `(a, b, c)` and
/// `(b, a, d)`, `(c, d, b)` and `(d, c, a)`.
fn flipped_triangles(mesh: &Mesh, edge: HalfEdgeId) -> Option<[[[f32; 3]; 3]; 2]> {
    let at = |vertex| mesh.vertex_position(vertex).copied();
    let a = at(mesh.from_vertex(edge)?)?;
    let b = at(mesh.to_vertex(edge)?)?;
    let c = at(mesh.to_vertex(mesh.next(edge)?)?)?;
    let d = at(mesh.to_vertex(mesh.next(mesh.twin(edge)?)?)?)?;
    Some([[c, d, b], [d, c, a]])
}

/// The middle of the vertex's neighbors, moved onto the surface by a Newton
/// step along the gradient.
fn relaxed_position<F: ScalarField>(
    field: &F,
    mesh: &Mesh,
    vertex: VertexId,
    faces: &[FaceId],
    run: &RunContext,
) -> Result<Option<[f32; 3]>, DualContourError> {
    let mut neighbors = faces
        .iter()
        .flat_map(|&face| loop_vertices(mesh, face))
        .filter(|&other| other != vertex)
        .collect::<Vec<_>>();
    neighbors.sort_unstable();
    neighbors.dedup();
    if neighbors.is_empty() {
        return Ok(None);
    }
    let mut middle = [0.0_f64; 3];
    for &neighbor in &neighbors {
        let position = position(mesh, neighbor);
        for axis in 0..3 {
            middle[axis] += f64::from(position[axis]);
        }
    }
    #[expect(clippy::cast_possible_truncation, reason = "positions are f32")]
    let middle = middle.map(|v| (v / neighbors.len() as f64) as f32);
    let mut row = [[0.0_f32; 4]];
    field.eval_gradients(&[middle], &mut row);
    run.check()?;
    let [value, gx, gy, gz] = row[0];
    let length2 = gx * gx + gy * gy + gz * gz;
    if !(value.is_finite() && length2.is_finite() && length2 > 0.0) {
        return Ok(None);
    }
    let step = value / length2;
    Ok(Some([
        middle[0] - step * gx,
        middle[1] - step * gy,
        middle[2] - step * gz,
    ]))
}

/// Whether every face around `vertex` agrees with the field once the
/// vertex sits at `target`.
fn unfolds<F: ScalarField>(
    field: &F,
    mesh: &Mesh,
    vertex: VertexId,
    target: [f32; 3],
    faces: &[FaceId],
    run: &RunContext,
) -> Result<bool, DualContourError> {
    let mut normals = Vec::with_capacity(faces.len());
    let mut centers = Vec::with_capacity(faces.len());
    for &face in faces {
        let points = loop_vertices(mesh, face)
            .map(|other| {
                if other == vertex {
                    target
                } else {
                    position(mesh, other)
                }
            })
            .collect::<Vec<_>>();
        normals.push(newell_normal(&points));
        centers.push(center(&points));
    }
    let mut gradients = alloc::vec![[0.0_f32; 4]; centers.len()];
    field.eval_gradients(&centers, &mut gradients);
    run.check()?;
    Ok(normals.iter().zip(&gradients).all(|(normal, gradient)| {
        let gradient = [gradient[1], gradient[2], gradient[3]].map(f64::from);
        dot(*normal, gradient) > 0.0
    }))
}

/// The faces of `faces` whose winding still opposes the gradient at their
/// centers.
fn still_inverted<F: ScalarField>(
    field: &F,
    mesh: &Mesh,
    faces: &[FaceId],
    run: &RunContext,
) -> Result<Vec<FaceId>, DualContourError> {
    let points = faces
        .iter()
        .map(|&face| {
            loop_vertices(mesh, face)
                .map(|v| position(mesh, v))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let centers = points
        .iter()
        .map(|points| center(points))
        .collect::<Vec<_>>();
    let mut gradients = alloc::vec![[0.0_f32; 4]; centers.len()];
    field.eval_gradients(&centers, &mut gradients);
    run.check()?;
    Ok(faces
        .iter()
        .zip(&points)
        .zip(&gradients)
        .filter(|((_, points), gradient)| {
            let gradient = [gradient[1], gradient[2], gradient[3]].map(f64::from);
            dot(newell_normal(points), gradient) <= 0.0
        })
        .map(|((&face, _), _)| face)
        .collect())
}

fn loop_vertices(mesh: &Mesh, face: FaceId) -> impl Iterator<Item = VertexId> + '_ {
    mesh.face_loop(face)
        .filter_map(|corner| mesh.to_vertex(corner))
}

fn position(mesh: &Mesh, vertex: VertexId) -> [f32; 3] {
    *mesh
        .vertex_position(vertex)
        .expect("a vertex of a live face is live")
}

fn center(points: &[[f32; 3]]) -> [f32; 3] {
    #[expect(clippy::cast_precision_loss, reason = "faces have a few corners")]
    let count = points.len() as f32;
    core::array::from_fn(|axis| points.iter().map(|p| p[axis]).sum::<f32>() / count)
}
