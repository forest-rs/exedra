// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic constrained-Delaunay edge legalization.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use crate::predicates::{InCircle, Orientation, incircle, orient2d};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Edge {
    vertices: [u32; 2],
}

impl Edge {
    const fn new(a: u32, b: u32) -> Self {
        if a < b {
            Self { vertices: [a, b] }
        } else {
            Self { vertices: [b, a] }
        }
    }
}

type Adjacency = BTreeMap<Edge, Vec<usize>>;

/// Legalizes unconstrained interior edges and returns the number of flips.
///
/// Boundary edges occur once and cannot enter the worklist. Hole-bridge edges
/// occur twice in the ear-clipped cover, so they correctly behave like other
/// interior edges rather than becoming artificial constraints.
///
/// # Termination and uniqueness
///
/// A strictly illegal edge flips when the opposite vertex lies strictly inside
/// the circumcircle. An exactly cocircular quadrilateral flips only toward the
/// diagonal that contains the lowest input index of its four corners (see
/// [`is_illegal`]). That tie rule is the exact-arithmetic answer for a
/// symbolic perturbation of the standard paraboloid lift, where each vertex is
/// lowered by an infinitesimal amount that decreases with its index: the
/// lowest-index vertex of any cocircular set lies inside every circle through
/// the others. Strict decisions are unchanged by an infinitesimal perturbation,
/// so every flip performed here is a legal Lawson flip on a point set with no
/// cocircular quadruples. Lawson's argument then gives termination, and the
/// constrained Delaunay triangulation of a fixed boundary is unique in general
/// position, so the resulting triangle set does not depend on the ear-clipped
/// cover it started from.
pub(crate) fn legalize_edges(points: &[[f64; 2]], triangles: &mut [[u32; 3]]) -> usize {
    let mut adjacency = Adjacency::new();
    for (triangle_index, &triangle) in triangles.iter().enumerate() {
        add_triangle(&mut adjacency, triangle_index, triangle);
    }
    let mut worklist: BTreeSet<Edge> = adjacency
        .iter()
        .filter_map(|(&edge, incident)| (incident.len() == 2).then_some(edge))
        .collect();

    let mut flips = 0;
    while let Some(edge) = worklist.iter().next().copied() {
        worklist.remove(&edge);
        let Some(candidate) = flip_candidate(&adjacency, triangles, edge) else {
            continue;
        };
        if !is_illegal(points, triangles, candidate) {
            continue;
        }
        replace_edge(points, triangles, &mut adjacency, &mut worklist, candidate);
        flips += 1;
    }
    for triangle in &mut *triangles {
        *triangle = canonical_triangle(*triangle);
    }
    triangles.sort_unstable_by_key(|triangle| canonical_triangle(*triangle));
    flips
}

#[derive(Copy, Clone, Debug)]
struct Flip {
    edge: Edge,
    first_triangle: usize,
    second_triangle: usize,
    first_opposite: u32,
    second_opposite: u32,
}

fn flip_candidate(adjacency: &Adjacency, triangles: &[[u32; 3]], edge: Edge) -> Option<Flip> {
    let incident = adjacency.get(&edge)?;
    let [first_triangle, second_triangle] = *incident.as_slice() else {
        return None;
    };
    Some(Flip {
        edge,
        first_triangle,
        second_triangle,
        first_opposite: opposite_vertex(triangles[first_triangle], edge)?,
        second_opposite: opposite_vertex(triangles[second_triangle], edge)?,
    })
}

/// Reports whether `candidate` must flip.
///
/// The quadrilateral must be strictly convex, which the ear-clipped cover and
/// every prior flip already guarantee for a strictly illegal edge; the check
/// keeps degenerate covers from producing inverted triangles. Cocircular ties
/// prefer the diagonal containing the lowest index of the four corners, which
/// the lexicographic edge comparison expresses because the four indices are
/// distinct.
fn is_illegal(points: &[[f64; 2]], triangles: &[[u32; 3]], candidate: Flip) -> bool {
    let [u, v] = candidate.edge.vertices;
    let a = candidate.first_opposite;
    let b = candidate.second_opposite;
    if a == b
        || !strictly_opposite(
            points[a as usize],
            points[b as usize],
            points[u as usize],
            points[v as usize],
        )
    {
        return false;
    }

    let triangle = triangles[candidate.first_triangle];
    let position = incircle(
        points[triangle[0] as usize],
        points[triangle[1] as usize],
        points[triangle[2] as usize],
        points[b as usize],
    );
    match position {
        InCircle::Inside => true,
        InCircle::Outside => false,
        InCircle::Cocircular => Edge::new(a, b) < candidate.edge,
    }
}

fn strictly_opposite(a: [f64; 2], b: [f64; 2], u: [f64; 2], v: [f64; 2]) -> bool {
    matches!(
        (orient2d(a, b, u), orient2d(a, b, v)),
        (Orientation::Ccw, Orientation::Cw) | (Orientation::Cw, Orientation::Ccw)
    )
}

fn replace_edge(
    points: &[[f64; 2]],
    triangles: &mut [[u32; 3]],
    adjacency: &mut Adjacency,
    worklist: &mut BTreeSet<Edge>,
    candidate: Flip,
) {
    let [u, v] = candidate.edge.vertices;
    let a = candidate.first_opposite;
    let b = candidate.second_opposite;
    let first = ccw_triangle(points, [a, b, u]);
    let second = ccw_triangle(points, [b, a, v]);
    let old_first = triangles[candidate.first_triangle];
    let old_second = triangles[candidate.second_triangle];
    let mut affected = BTreeSet::new();
    for [left, right] in triangle_edges(old_first)
        .into_iter()
        .chain(triangle_edges(old_second))
        .chain(triangle_edges(first))
        .chain(triangle_edges(second))
    {
        affected.insert(Edge::new(left, right));
    }

    remove_triangle(adjacency, candidate.first_triangle, old_first);
    remove_triangle(adjacency, candidate.second_triangle, old_second);
    triangles[candidate.first_triangle] = first;
    triangles[candidate.second_triangle] = second;
    add_triangle(adjacency, candidate.first_triangle, first);
    add_triangle(adjacency, candidate.second_triangle, second);

    for edge in affected {
        worklist.remove(&edge);
        if adjacency
            .get(&edge)
            .is_some_and(|incident| incident.len() == 2)
        {
            worklist.insert(edge);
        }
    }
}

fn add_triangle(adjacency: &mut Adjacency, triangle_index: usize, triangle: [u32; 3]) {
    for [a, b] in triangle_edges(triangle) {
        let incident = adjacency.entry(Edge::new(a, b)).or_default();
        if let Err(position) = incident.binary_search(&triangle_index) {
            incident.insert(position, triangle_index);
        }
    }
}

fn remove_triangle(adjacency: &mut Adjacency, triangle_index: usize, triangle: [u32; 3]) {
    for [a, b] in triangle_edges(triangle) {
        let edge = Edge::new(a, b);
        let remove_edge = if let Some(incident) = adjacency.get_mut(&edge) {
            if let Ok(position) = incident.binary_search(&triangle_index) {
                incident.remove(position);
            }
            incident.is_empty()
        } else {
            false
        };
        if remove_edge {
            adjacency.remove(&edge);
        }
    }
}

fn ccw_triangle(points: &[[f64; 2]], mut triangle: [u32; 3]) -> [u32; 3] {
    if orient2d(
        points[triangle[0] as usize],
        points[triangle[1] as usize],
        points[triangle[2] as usize],
    ) == Orientation::Cw
    {
        triangle.swap(1, 2);
    }
    debug_assert_eq!(
        orient2d(
            points[triangle[0] as usize],
            points[triangle[1] as usize],
            points[triangle[2] as usize],
        ),
        Orientation::Ccw,
        "a convex edge flip must emit nondegenerate triangles"
    );
    triangle
}

const fn triangle_edges([a, b, c]: [u32; 3]) -> [[u32; 2]; 3] {
    [[a, b], [b, c], [c, a]]
}

fn opposite_vertex(triangle: [u32; 3], edge: Edge) -> Option<u32> {
    triangle
        .into_iter()
        .find(|vertex| !edge.vertices.contains(vertex))
}

const fn canonical_triangle([a, b, c]: [u32; 3]) -> [u32; 3] {
    if a < b && a < c {
        [a, b, c]
    } else if b < c {
        [b, c, a]
    } else {
        [c, a, b]
    }
}

/// Counts edges the legalization loop would still flip.
///
/// Zero means every unconstrained interior edge is locally Delaunay under the
/// crate's cocircular tie rule; the count is a test oracle for the property
/// [`legalize_edges`] claims to establish.
#[cfg(test)]
pub(crate) fn illegal_edge_count(points: &[[f64; 2]], triangles: &[[u32; 3]]) -> usize {
    let mut adjacency = Adjacency::new();
    for (triangle_index, &triangle) in triangles.iter().enumerate() {
        add_triangle(&mut adjacency, triangle_index, triangle);
    }
    adjacency
        .keys()
        .filter(|&&edge| {
            flip_candidate(&adjacency, triangles, edge)
                .is_some_and(|candidate| is_illegal(points, triangles, candidate))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rational-rotation near-circle: convex, no exactly cocircular quadruple
    /// is guaranteed, but many near-ties exercise the exact incircle path.
    fn near_circle(count: usize) -> Vec<[f64; 2]> {
        let step = 4.0 / count as f64;
        let mut points = Vec::with_capacity(count);
        let (mut x, mut y) = (10.0_f64, 0.0_f64);
        for _ in 0..count {
            points.push([x, y]);
            let next_x = x - y * step;
            let next_y = y + x * step;
            x = next_x;
            y = next_y;
        }
        points
    }

    fn fan(count: u32, apex: u32) -> Vec<[u32; 3]> {
        (1..count - 1)
            .map(|offset| {
                let b = (apex + offset) % count;
                let c = (apex + offset + 1) % count;
                [apex, b, c]
            })
            .collect()
    }

    #[test]
    fn legalized_result_is_independent_of_the_initial_cover() {
        // Uniqueness under the symbolic perturbation: every fan of the same
        // convex ring legalizes to one triangle set, and that set has no
        // remaining illegal edge.
        for count in [5_u32, 8, 13, 16, 29] {
            let points = near_circle(count as usize);
            let mut reference = fan(count, 0);
            legalize_edges(&points, &mut reference);
            assert_eq!(illegal_edge_count(&points, &reference), 0);
            for apex in 1..count {
                let mut triangles = fan(count, apex);
                legalize_edges(&points, &mut triangles);
                assert_eq!(triangles, reference, "count {count} apex {apex}");
                assert_eq!(legalize_edges(&points, &mut triangles), 0, "idempotent");
            }
        }
    }

    #[test]
    fn exactly_cocircular_fans_agree_on_the_lowest_index_rule() {
        // Eight exactly cocircular points: integer coordinates on the circle
        // of radius five. Every fan must legalize to the fan from vertex 0.
        let points = [
            [5.0, 0.0],
            [4.0, 3.0],
            [3.0, 4.0],
            [0.0, 5.0],
            [-3.0, 4.0],
            [-4.0, 3.0],
            [-5.0, 0.0],
            [0.0, -5.0],
        ];
        let expected = fan(8, 0);
        for apex in 0..8 {
            let mut triangles = fan(8, apex);
            legalize_edges(&points, &mut triangles);
            assert_eq!(triangles, expected, "apex {apex}");
            assert_eq!(illegal_edge_count(&points, &triangles), 0);
        }
    }

    #[test]
    fn flips_an_illegal_diagonal() {
        // This asymmetric convex quad starts on the strictly illegal
        // diagonal, which must flip exactly once to the legal alternative.
        let points = [[4.9, 4.9], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let mut triangles = [[3, 0, 1], [3, 1, 2]];
        assert_eq!(legalize_edges(&points, &mut triangles), 1);
        assert_eq!(triangles, [[0, 1, 2], [0, 2, 3]]);
    }

    #[test]
    fn exact_cocircular_tie_chooses_the_lower_diagonal() {
        // A square makes both diagonals geometrically legal, so the stable
        // lowest-index tie rule must pick one and then be idempotent.
        let points = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]];
        let mut triangles = [[3, 0, 1], [1, 2, 3]];
        assert_eq!(legalize_edges(&points, &mut triangles), 1);
        assert_eq!(triangles, [[0, 1, 2], [0, 2, 3]]);

        assert_eq!(legalize_edges(&points, &mut triangles), 0);
        assert_eq!(triangles, [[0, 1, 2], [0, 2, 3]]);
    }

    #[test]
    fn boundary_and_concave_union_edges_do_not_flip() {
        // The only shared edge borders a non-convex union of two triangles;
        // treating it as a flip candidate would invert the replacement.
        let points = [[0.0, 0.0], [2.0, 0.0], [1.0, 0.5], [0.0, 2.0]];
        let mut triangles = [[0, 1, 2], [0, 2, 3]];
        assert_eq!(legalize_edges(&points, &mut triangles), 0);
    }
}
