// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dropping islands left by sampling features thinner than a cell.
//!
//! Where a feature tapers below the grid spacing, as toward the rim of an
//! acute wedge, a lone grid node can land inside it while every neighbor
//! lands outside. The cells around that node close a tiny surface around it,
//! cut off from the body the feature belongs to. The field may also hold a
//! genuinely small component, so size alone does not decide.
//!
//! The field decides. From inside a small closed component, descending the
//! field moves away from the thin tip and into the body that holds it. Each
//! step is taken only when the field's interval bounds show the whole
//! segment inside, so the descent never jumps a gap between disjoint solids.
//! When the rest of the mesh encloses the point the descent reaches, the
//! component's inside is connected to a region already meshed: the
//! component is a stray sample of it and is dropped. Otherwise, including
//! when the bounds cannot show a path, it is kept.
//!
//! Dropping a stray sample loses at most its own extent, a cell or two at
//! the finest depth, of a tip the grid already truncates. It does not
//! reconstruct the tip's connection to the body.

use alloc::vec;
use alloc::vec::Vec;
use exedra_math::Real;
use exedra_mesh::{DeletePolicy, FaceId, Mesh, VertexId, op};
use hashbrown::{HashMap, HashSet};

use super::DualContourError;
use super::control::RunContext;
use crate::{Aabb, ScalarField};

/// Faces in the largest component considered: the surface around one lone
/// node is twelve triangles.
const ISLAND_FACES: usize = 24;

/// Descent steps before the descent stops where it is.
const DESCENT_STEPS: usize = 24;

/// Halvings of a rejected descent step before the descent stalls.
const BACKTRACKS: usize = 8;

/// Interval queries spent showing one descent step stays inside.
const SEGMENT_QUERIES: usize = 32;

/// Deletes small closed components whose inside the rest of the mesh
/// already encloses. `cell` is the edge of a finest cell. Returns the
/// number of components deleted.
pub(super) fn drop_sampling_islands<F: ScalarField>(
    field: &F,
    mesh: &mut Mesh,
    cell: f32,
    run: &RunContext,
) -> Result<usize, DualContourError> {
    let small = small_components(mesh);
    if small.is_empty() {
        return Ok(0);
    }
    // One cell deep is past where the body's own surface sits.
    let depth = field.capabilities().metric.lipschitz().unwrap_or(1.0) * cell;
    let mut doomed = Vec::new();
    let mut dropped = 0;
    for faces in small {
        if !closed(mesh, &faces) {
            continue;
        }
        let vertices = component_vertices(mesh, &faces);
        let mut start = [0.0_f32; 3];
        for &vertex in &vertices {
            let position = mesh.vertex_position(vertex).copied().unwrap_or_default();
            for axis in 0..3 {
                start[axis] += position[axis];
            }
        }
        #[expect(clippy::cast_precision_loss, reason = "a handful of vertices")]
        let start = start.map(|sum| sum / vertices.len() as f32);
        let Some(reached) = descend(field, start, depth, 0.5 * cell, run)? else {
            continue;
        };
        let own = faces.iter().copied().collect::<HashSet<_>>();
        if winding_number(mesh, reached, &own) > 0.5 {
            doomed.extend(faces);
            dropped += 1;
        }
    }
    if dropped > 0 {
        doomed.sort_unstable();
        let mut session = mesh.edit();
        op::delete_faces(&mut session, &doomed, DeletePolicy::CleanupIsolated)
            .expect("whole closed components are live interior faces");
        let _: () = session.finish();
    }
    Ok(dropped)
}

/// Descends the field from `start` while staying inside, until `-depth`, a
/// stall or the step limit; `None` when `start` is not inside. Every step
/// is a segment the field's interval bounds show inside.
///
/// Inside a thin wedge the gradient flips from one side to the other at each
/// step; averaging two opposing steps follows the wedge into the body
/// instead of across it.
fn descend<F: ScalarField>(
    field: &F,
    start: [f32; 3],
    depth: f32,
    step: f32,
    run: &RunContext,
) -> Result<Option<[f32; 3]>, DualContourError> {
    let sample = |point: [f32; 3]| -> Result<_, DualContourError> {
        let mut row = [[0.0_f32; 4]];
        field.eval_gradients(&[point], &mut row);
        run.check()?;
        let [value, gx, gy, gz] = row[0];
        Ok((value, unit([gx, gy, gz])))
    };
    let (mut value, mut gradient) = sample(start)?;
    if value.is_nan() || value >= 0.0 {
        return Ok(None);
    }
    let mut point = start;
    let mut previous: Option<[f32; 3]> = None;
    for _ in 0..DESCENT_STEPS {
        if value <= -depth {
            break;
        }
        let Some(down) = gradient else {
            break;
        };
        let direction = match previous {
            Some(last) if dot(down, last) < 0.0 => {
                unit(core::array::from_fn(|axis| down[axis] + last[axis])).unwrap_or(down)
            }
            _ => down,
        };
        let mut length = step;
        let mut accepted = None;
        for _ in 0..BACKTRACKS {
            let next = core::array::from_fn(|axis| point[axis] - length * direction[axis]);
            let (next_value, next_gradient) = sample(next)?;
            if next_value < value && stays_inside(field, point, next, run)? {
                accepted = Some((next, next_value, next_gradient));
                break;
            }
            length *= 0.5;
        }
        let Some((next, next_value, next_gradient)) = accepted else {
            break;
        };
        previous = Some(down);
        (point, value, gradient) = (next, next_value, next_gradient);
    }
    Ok(Some(point))
}

/// Whether the field's interval bounds show the segment `from`-`to` inside,
/// bisecting it where one box is too coarse.
fn stays_inside<F: ScalarField>(
    field: &F,
    from: [f32; 3],
    to: [f32; 3],
    run: &RunContext,
) -> Result<bool, DualContourError> {
    let mut pending = vec![(from, to)];
    for _ in 0..SEGMENT_QUERIES {
        let Some((a, b)) = pending.pop() else {
            return Ok(true);
        };
        let bounds = Aabb {
            min: core::array::from_fn(|axis| a[axis].min(b[axis])),
            max: core::array::from_fn(|axis| a[axis].max(b[axis])),
        };
        let interval = field.eval_interval(&bounds);
        run.check()?;
        match interval {
            Some([_, high]) if high < 0.0 => {}
            Some(_) => {
                let middle = core::array::from_fn(|axis| 0.5 * (a[axis] + b[axis]));
                pending.extend([(a, middle), (middle, b)]);
            }
            None => return Ok(false),
        }
    }
    Ok(pending.is_empty())
}

/// The generalized winding number of the mesh, without `skip`, around
/// `point`: about one inside a closed surface and zero outside.
fn winding_number(mesh: &Mesh, point: [f32; 3], skip: &HashSet<FaceId>) -> f64 {
    let point = point.map(f64::from);
    let mut total = 0.0;
    for face in mesh.faces().filter(|face| !skip.contains(face)) {
        let corners = mesh
            .face_loop(face)
            .filter_map(|edge| mesh.vertex_position(mesh.to_vertex(edge)?))
            .map(|position| core::array::from_fn(|axis| f64::from(position[axis]) - point[axis]))
            .collect::<Vec<[f64; 3]>>();
        for pair in corners.windows(2).skip(1) {
            total += solid_angle(corners[0], pair[0], pair[1]);
        }
    }
    total / (4.0 * core::f64::consts::PI)
}

/// The signed solid angle a triangle subtends at the origin (Van Oosterom
/// and Strackee).
fn solid_angle(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let length = |v: [f64; 3]| Real::sqrt(dot(v, v));
    let (la, lb, lc) = (length(a), length(b), length(c));
    let triple = dot(a, exedra_math::cross(b, c));
    let denominator = la * lb * lc + dot(a, b) * lc + dot(b, c) * la + dot(c, a) * lb;
    // atan2 from acos, which `Real` provides.
    let radius = Real::sqrt(triple * triple + denominator * denominator);
    if radius == 0.0 {
        return 0.0;
    }
    let angle = Real::acos((denominator / radius).clamp(-1.0, 1.0));
    2.0 * if triple < 0.0 { -angle } else { angle }
}

fn dot<T: Real>(a: [T; 3], b: [T; 3]) -> T {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let length = Real::sqrt(dot(v, v));
    (length.is_finite() && length > 0.0).then(|| v.map(|c| c / length))
}

/// Faces of each component with at most [`ISLAND_FACES`] faces.
fn small_components(mesh: &Mesh) -> Vec<Vec<FaceId>> {
    let mut parent = HashMap::<VertexId, VertexId>::new();
    for face in mesh.faces() {
        let mut corners = mesh.face_loop(face).filter_map(|edge| mesh.to_vertex(edge));
        let Some(first) = corners.next() else {
            continue;
        };
        parent.entry(first).or_insert(first);
        for vertex in corners {
            parent.entry(vertex).or_insert(vertex);
            let (a, b) = (root(&mut parent, first), root(&mut parent, vertex));
            if a != b {
                parent.insert(a, b);
            }
        }
    }
    let mut components = HashMap::<VertexId, Vec<FaceId>>::new();
    for face in mesh.faces() {
        if let Some(vertex) = mesh.face_loop(face).find_map(|edge| mesh.to_vertex(edge)) {
            components
                .entry(root(&mut parent, vertex))
                .or_default()
                .push(face);
        }
    }
    let mut small = components
        .into_values()
        .filter(|faces| faces.len() <= ISLAND_FACES)
        .collect::<Vec<_>>();
    small.sort_unstable();
    small
}

/// The root of `vertex` in a union-find forest, halving paths on the way.
fn root(parent: &mut HashMap<VertexId, VertexId>, mut vertex: VertexId) -> VertexId {
    loop {
        let up = parent[&vertex];
        if up == vertex {
            return vertex;
        }
        let grand = parent[&up];
        parent.insert(vertex, grand);
        vertex = grand;
    }
}

fn closed(mesh: &Mesh, faces: &[FaceId]) -> bool {
    faces.iter().all(|&face| {
        mesh.face_loop(face).all(|edge| {
            mesh.twin(edge)
                .and_then(|twin| mesh.face(twin))
                .is_some_and(|other| other != FaceId::OUTSIDE)
        })
    })
}

fn component_vertices(mesh: &Mesh, faces: &[FaceId]) -> Vec<VertexId> {
    let mut vertices = faces
        .iter()
        .flat_map(|&face| mesh.face_loop(face).filter_map(|edge| mesh.to_vertex(edge)))
        .collect::<Vec<_>>();
    vertices.sort_unstable();
    vertices.dedup();
    vertices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Metric;
    use crate::dual_contour::control::ExtractionLimits;
    use crate::function::FnField;

    #[test]
    fn descent_does_not_jump_between_disjoint_solids() {
        // A long step from the small ball lands deeper inside the large one,
        // across outside space between them.
        let distance = |p: [f64; 3], x: f64, r: f64| {
            Real::sqrt((p[0] - x) * (p[0] - x) + p[1] * p[1] + p[2] * p[2]) - r
        };
        let field = FnField::new(
            move |p: [f64; 3]| distance(p, 0.0, 0.05).min(distance(p, -0.45, 0.15)),
            Metric::Lipschitz(1.0),
            Aabb::new([-1.0; 3], [1.0; 3]),
        )
        .unwrap();
        let run = RunContext::new(ExtractionLimits::default());
        let reached = descend(&field, [0.01, 0.0, 0.0], 1.0, 0.5, &run)
            .unwrap()
            .unwrap();
        assert!(dot(reached, reached) < 0.05 * 0.05, "{reached:?}");
    }
}
