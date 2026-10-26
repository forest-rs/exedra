// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Separate vertices for faces that one component crosses twice.
//!
//! A cell keeps one vertex per surface component, and a component may meet a
//! face of its cell in two arcs, as where a thin wedge or two sheets pass
//! through an ambiguous face. When the cell across that face also joins both
//! arcs into one component, the patches of both arcs run between the same two
//! vertices: the dual edge between them is doubled, four faces share it, and
//! the mesh is not manifold.
//!
//! Each arc gets a vertex of its own, at the middle of the arc on the face,
//! and its patches pass through it instead of along the shared edge. The
//! patches stay closed and every edge keeps two faces.

use alloc::vec::Vec;
use exedra_mesh::MeshBuilder;
use hashbrown::HashMap;

use super::VertexEntry;
use super::control::{ExtractionResource, RunContext};
use crate::ScalarField;
use crate::adaptive_transition::{AdaptiveGrid, CornerKey, EdgeSegmentKey, segment_end};
use crate::dual_contour::DualContourError;

/// A patch: its vertex cycle and region.
pub(super) type Patch = (Vec<VertexEntry>, u32);

/// Routes the arcs of every doubled dual edge through their own vertices.
///
/// `segments[i]` is the primal edge that generated `patches[i]`. Returns the
/// patches with each rerouted one fanned into triangles from an arc vertex,
/// and the number of arc vertices added.
pub(super) fn separate_doubled_edges<F: ScalarField>(
    patches: Vec<Patch>,
    segments: &[EdgeSegmentKey],
    grid: &AdaptiveGrid<'_, F>,
    builder: &mut MeshBuilder,
    run: &RunContext,
    counts: (&mut usize, &mut usize),
) -> Result<(Vec<Patch>, usize), DualContourError> {
    let (vertex_count, face_count) = counts;
    let mut uses = HashMap::<(u32, u32), Vec<usize>>::new();
    for (index, (face, _)) in patches.iter().enumerate() {
        for (i, vertex) in face.iter().enumerate() {
            let next = face[(i + 1) % face.len()].builder_index;
            let edge = ordered(vertex.builder_index, next);
            uses.entry(edge).or_default().push(index);
        }
    }
    let mut doubled = uses
        .into_iter()
        .filter(|(_, users)| users.len() == 4)
        .collect::<Vec<_>>();
    if doubled.is_empty() {
        return Ok((patches, 0));
    }
    doubled.sort_unstable();

    // A primal edge lies on two faces, so its patch may take an arc vertex
    // on each of two doubled edges.
    let mut arcs_of = HashMap::<usize, Vec<(VertexEntry, (u32, u32))>>::new();
    let mut added = 0;
    for (edge, users) in doubled {
        let Some(arcs) = face_arcs(&users, segments, grid) else {
            continue;
        };
        for (pair, position) in arcs {
            *vertex_count = run.grow(ExtractionResource::Vertices, *vertex_count, 1)?;
            let entry = VertexEntry {
                builder_index: builder.push_vertex(position),
                position,
                sharpness: 0.0,
            };
            added += 1;
            for user in pair {
                arcs_of.entry(user).or_default().push((entry, edge));
                // Each arc vertex adds one vertex, and so one triangle.
                *face_count = run.grow(ExtractionResource::Faces, *face_count, 1)?;
            }
        }
    }

    let mut out = Vec::with_capacity(patches.len() + 3 * arcs_of.len());
    for (index, (face, region)) in patches.into_iter().enumerate() {
        let Some(arcs) = arcs_of.get(&index) else {
            out.push((face, region));
            continue;
        };
        // Each arc vertex goes on its doubled edge; then the polygon fans
        // from the first one.
        let mut cycle = Vec::with_capacity(face.len() + arcs.len());
        for (i, vertex) in face.iter().enumerate() {
            cycle.push(*vertex);
            let next = face[(i + 1) % face.len()].builder_index;
            let edge = ordered(vertex.builder_index, next);
            if let Some((arc, _)) = arcs.iter().find(|(_, doubled)| *doubled == edge) {
                cycle.push(*arc);
            }
        }
        let first = cycle
            .iter()
            .position(|vertex| vertex.builder_index == arcs[0].0.builder_index)
            .expect("the arc vertex was just inserted");
        cycle.rotate_left(first);
        for pair in cycle[1..].windows(2) {
            out.push((alloc::vec![cycle[0], pair[0], pair[1]], region));
        }
    }
    Ok((out, added))
}

/// The two arcs of an ambiguous face whose four sides generated `users`, as
/// patch pairs with the middle of each arc, or `None` for anything else.
///
/// The face is the square the four segments bound. Where finer cells meet
/// it, a segment may be only part of its side, so each side must hold
/// exactly one of them. The arcs are paired by the asymptotic decider over
/// the face's canonical corner cycle, the same rule cell classification
/// applies, so the arcs here are the ones the cells' components contain.
fn face_arcs<F: ScalarField>(
    users: &[usize],
    segments: &[EdgeSegmentKey],
    grid: &AdaptiveGrid<'_, F>,
) -> Option<[([usize; 2], [f32; 3]); 2]> {
    let sides = users.iter().map(|&user| segments[user]).collect::<Vec<_>>();
    let mut axes = sides
        .iter()
        .map(|side| usize::from(side.axis))
        .collect::<Vec<_>>();
    axes.sort_unstable();
    let (u, v) = match axes.as_slice() {
        [a, b, c, d] if a == b && c == d && b != c => (*a, *c),
        _ => return None,
    };
    let w = 3 - u - v;
    let coordinate = |key: CornerKey, axis: usize| [key.x, key.y, key.z][axis];
    let ends = sides
        .iter()
        .flat_map(|side| [side.start, segment_end(*side)])
        .collect::<Vec<_>>();
    let plane = coordinate(ends[0], w);
    if ends.iter().any(|&key| coordinate(key, w) != plane) {
        return None;
    }
    let range = |axis: usize| {
        let values = ends.iter().map(|&key| coordinate(key, axis));
        Some((values.clone().min()?, values.max()?))
    };
    let ((low_u, high_u), (low_v, high_v)) = (range(u)?, range(v)?);
    if high_u == low_u || high_u - low_u != high_v - low_v {
        return None;
    }
    let corner = |at_u: u32, at_v: u32| {
        let mut key = [0; 3];
        (key[u], key[v], key[w]) = (at_u, at_v, plane);
        CornerKey::new(key[0], key[1], key[2])
    };
    // The canonical cycle: origin, along `u`, along both, along `v`. Side
    // `i` runs from corner `i` to corner `i + 1`.
    let corners = [
        corner(low_u, low_v),
        corner(high_u, low_v),
        corner(high_u, high_v),
        corner(low_u, high_v),
    ];
    let side_of = |side: EdgeSegmentKey| {
        let start = side.start;
        if usize::from(side.axis) == u {
            let at = coordinate(start, v);
            (at == low_v).then_some(0).or((at == high_v).then_some(2))
        } else {
            let at = coordinate(start, u);
            (at == high_u).then_some(1).or((at == low_u).then_some(3))
        }
    };
    let mut on_side = [None; 4];
    for (&user, &side) in users.iter().zip(&sides) {
        if on_side[side_of(side)?].replace((user, side)).is_some() {
            return None;
        }
    }
    let on_side = on_side.map(|entry| entry.expect("four segments on four sides"));
    let values = corners.map(|corner| grid.value(corner));
    let [Some(f0), Some(f1), Some(f2), Some(f3)] = values else {
        return None;
    };
    let inside = [f0, f1, f2, f3].map(|value| value < 0.0);
    if inside[0] != inside[2] || inside[1] != inside[3] || inside[0] == inside[1] {
        return None;
    }
    let determinant = f0 * f2 - f1 * f3;
    let isolated = if determinant.is_finite() && determinant > 0.0 {
        [1, 3]
    } else {
        [0, 2]
    };
    let mut arcs = [([0; 2], [0.0; 3]); 2];
    for (arc, corner) in arcs.iter_mut().zip(isolated) {
        // The arc around a corner crosses the two sides that meet there.
        let (first, p) = on_side[(corner + 3) % 4];
        let (second, q) = on_side[corner];
        let (p, q) = (crossing(grid, p)?, crossing(grid, q)?);
        *arc = (
            [first, second],
            core::array::from_fn(|axis| 0.5 * (p[axis] + q[axis])),
        );
    }
    Some(arcs)
}

/// Where the field crosses zero along `side`, by linear interpolation of
/// its end values.
fn crossing<F: ScalarField>(grid: &AdaptiveGrid<'_, F>, side: EdgeSegmentKey) -> Option<[f32; 3]> {
    let (start, end) = (side.start, segment_end(side));
    let (a, b) = (grid.value(start)?, grid.value(end)?);
    let t = (a / (a - b)).clamp(0.0, 1.0);
    let (p, q) = (grid.point(start), grid.point(end));
    Some(core::array::from_fn(|axis| {
        p[axis] + t * (q[axis] - p[axis])
    }))
}

fn ordered(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}
