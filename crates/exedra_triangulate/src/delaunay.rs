// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic constrained-Delaunay edge legalization.

use alloc::vec::Vec;

use exedra_math::predicates::{InCircle, Orientation, incircle, orient2d};

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

/// A checked cover has at most two triangles incident to an edge. Keeping
/// these inline avoids allocating a tiny vector for every edge and flip.
#[derive(Default)]
struct Incident {
    triangles: [usize; 2],
    len: usize,
}

impl Incident {
    fn as_slice(&self) -> &[usize] {
        &self.triangles[..self.len]
    }

    fn len(&self) -> usize {
        self.len
    }

    fn insert(&mut self, triangle: usize) {
        if let Err(position) = self.as_slice().binary_search(&triangle) {
            assert!(self.len < 2, "legalization requires a manifold cover");
            self.triangles.copy_within(position..self.len, position + 1);
            self.triangles[position] = triangle;
            self.len += 1;
        }
    }

    fn remove(&mut self, triangle: usize) {
        if let Ok(position) = self.as_slice().binary_search(&triangle) {
            self.triangles.copy_within(position + 1..self.len, position);
            self.len -= 1;
        }
    }
}

const NOT_QUEUED: usize = usize::MAX;

struct EdgeRecord {
    key: Edge,
    incident: Incident,
    queued_at: usize,
}

/// Flat adjacency is prepared once. Flips reuse the old diagonal's slot;
/// perimeter edge slots and triangle identities remain stable.
struct Adjacency {
    edges: Vec<EdgeRecord>,
    triangle_edges: Vec<[usize; 3]>,
}

impl Adjacency {
    fn new(triangles: &[[u32; 3]]) -> Self {
        let mut entries = Vec::with_capacity(triangles.len() * 3);
        for (triangle, &vertices) in triangles.iter().enumerate() {
            for (slot, [a, b]) in triangle_edges(vertices).into_iter().enumerate() {
                entries.push((Edge::new(a, b), triangle, slot));
            }
        }
        entries.sort_unstable();
        let edge_count = usize::from(!entries.is_empty())
            + entries
                .windows(2)
                .filter(|pair| pair[0].0 != pair[1].0)
                .count();
        let mut edges: Vec<EdgeRecord> = Vec::with_capacity(edge_count);
        let mut triangle_edges = alloc::vec![[0; 3]; triangles.len()];
        for (key, triangle, slot) in entries {
            if edges.last().is_none_or(|record| record.key != key) {
                edges.push(EdgeRecord {
                    key,
                    incident: Incident::default(),
                    queued_at: NOT_QUEUED,
                });
            }
            let edge = edges.len() - 1;
            edges[edge].incident.insert(triangle);
            triangle_edges[triangle][slot] = edge;
        }
        Self {
            edges,
            triangle_edges,
        }
    }
}

/// Indexed minimum heap. Each edge has at most one queued occurrence, so
/// storage is bounded by the original edge count and flips never allocate.
struct Worklist {
    heap: Vec<usize>,
}

impl Worklist {
    fn new(adjacency: &mut Adjacency) -> Self {
        let mut work = Self {
            heap: Vec::with_capacity(adjacency.edges.len()),
        };
        for edge in 0..adjacency.edges.len() {
            if adjacency.edges[edge].incident.len() == 2 {
                work.insert(edge, &mut adjacency.edges);
            }
        }
        work
    }

    fn swap(&mut self, a: usize, b: usize, edges: &mut [EdgeRecord]) {
        self.heap.swap(a, b);
        edges[self.heap[a]].queued_at = a;
        edges[self.heap[b]].queued_at = b;
    }

    fn up(&mut self, mut at: usize, edges: &mut [EdgeRecord]) -> usize {
        while at > 0 {
            let parent = (at - 1) / 2;
            if edges[self.heap[parent]].key <= edges[self.heap[at]].key {
                break;
            }
            self.swap(parent, at, edges);
            at = parent;
        }
        at
    }

    fn down(&mut self, mut at: usize, edges: &mut [EdgeRecord]) {
        loop {
            let left = 2 * at + 1;
            if left >= self.heap.len() {
                break;
            }
            let right = left + 1;
            let child = if right < self.heap.len()
                && edges[self.heap[right]].key < edges[self.heap[left]].key
            {
                right
            } else {
                left
            };
            if edges[self.heap[at]].key <= edges[self.heap[child]].key {
                break;
            }
            self.swap(at, child, edges);
            at = child;
        }
    }

    fn insert(&mut self, edge: usize, edges: &mut [EdgeRecord]) {
        if edges[edge].queued_at != NOT_QUEUED {
            return;
        }
        edges[edge].queued_at = self.heap.len();
        self.heap.push(edge);
        self.up(self.heap.len() - 1, edges);
    }

    fn remove(&mut self, edge: usize, edges: &mut [EdgeRecord]) {
        let at = edges[edge].queued_at;
        if at == NOT_QUEUED {
            return;
        }
        edges[edge].queued_at = NOT_QUEUED;
        let last = self.heap.pop().expect("queued edge has a heap entry");
        if at < self.heap.len() {
            self.heap[at] = last;
            edges[last].queued_at = at;
            if self.up(at, edges) == at {
                self.down(at, edges);
            }
        }
    }

    fn pop(&mut self, edges: &mut [EdgeRecord]) -> Option<usize> {
        let edge = *self.heap.first()?;
        self.remove(edge, edges);
        Some(edge)
    }
}

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
    let mut adjacency = Adjacency::new(triangles);
    let mut worklist = Worklist::new(&mut adjacency);

    let mut flips = 0;
    while let Some(edge) = worklist.pop(&mut adjacency.edges) {
        let Some(candidate) = flip_candidate(&adjacency, triangles, edge) else {
            continue;
        };
        if !is_illegal(points, triangles, adjacency.edges[edge].key, candidate) {
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
    edge: usize,
    first_triangle: usize,
    second_triangle: usize,
    first_opposite: u32,
    second_opposite: u32,
}

fn flip_candidate(adjacency: &Adjacency, triangles: &[[u32; 3]], edge: usize) -> Option<Flip> {
    let record = &adjacency.edges[edge];
    let incident = &record.incident;
    let [first_triangle, second_triangle] = *incident.as_slice() else {
        return None;
    };
    Some(Flip {
        edge,
        first_triangle,
        second_triangle,
        first_opposite: opposite_vertex(triangles[first_triangle], record.key)?,
        second_opposite: opposite_vertex(triangles[second_triangle], record.key)?,
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
fn is_illegal(points: &[[f64; 2]], triangles: &[[u32; 3]], edge: Edge, candidate: Flip) -> bool {
    let [u, v] = edge.vertices;
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
        InCircle::Cocircular => Edge::new(a, b) < edge,
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
    worklist: &mut Worklist,
    candidate: Flip,
) {
    let [u, v] = adjacency.edges[candidate.edge].key.vertices;
    let a = candidate.first_opposite;
    let b = candidate.second_opposite;
    let first = ccw_triangle(points, [a, b, u]);
    let second = ccw_triangle(points, [b, a, v]);
    let old_first = adjacency.triangle_edges[candidate.first_triangle];
    let old_second = adjacency.triangle_edges[candidate.second_triangle];
    let affected = [
        old_first[0],
        old_first[1],
        old_first[2],
        old_second[0],
        old_second[1],
        old_second[2],
    ];

    // Only the diagonal changes its queue key. Perimeter edges retain their
    // keys and can stay queued while their incident triangles are updated.
    // The insertion below schedules those already popped without disturbing
    // the remaining heap entries. Reusing the diagonal's slot keeps storage
    // bounded by the original edge count.
    worklist.remove(candidate.edge, &mut adjacency.edges);
    for (triangle, edges) in [
        (candidate.first_triangle, old_first),
        (candidate.second_triangle, old_second),
    ] {
        for edge in edges {
            adjacency.edges[edge].incident.remove(triangle);
        }
    }
    adjacency.edges[candidate.edge].key = Edge::new(a, b);
    for (index, vertices) in [
        (candidate.first_triangle, first),
        (candidate.second_triangle, second),
    ] {
        triangles[index] = vertices;
        for (slot, [left, right]) in triangle_edges(vertices).into_iter().enumerate() {
            let key = Edge::new(left, right);
            let edge = *affected
                .iter()
                .find(|&&edge| adjacency.edges[edge].key == key)
                .expect("flipped triangle uses only its diagonal and perimeter");
            adjacency.edges[edge].incident.insert(index);
            adjacency.triangle_edges[index][slot] = edge;
        }
    }
    for edge in affected {
        if adjacency.edges[edge].incident.len() == 2 {
            worklist.insert(edge, &mut adjacency.edges);
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
    let adjacency = Adjacency::new(triangles);
    adjacency
        .edges
        .iter()
        .enumerate()
        .filter(|&(edge, record)| {
            flip_candidate(&adjacency, triangles, edge)
                .is_some_and(|candidate| is_illegal(points, triangles, record.key, candidate))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeSet;

    #[test]
    fn worklist_matches_ordered_set_through_removal_and_key_changes() {
        let mut edges: Vec<_> = (0..32)
            .map(|i| EdgeRecord {
                key: Edge::new(i, 100 + i),
                incident: Incident::default(),
                queued_at: NOT_QUEUED,
            })
            .collect();
        let mut queue = Worklist {
            heap: Vec::with_capacity(edges.len()),
        };
        let capacity = queue.heap.capacity();
        let mut expected = BTreeSet::new();
        let mut random = 0x8b1c_8d4f_29a1_7f03_u64;
        for _ in 0..4096 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let index = (random % 32) as usize;
            if random & 3 == 0 {
                let wanted = expected.pop_first().map(|(_, index)| index);
                assert_eq!(queue.pop(&mut edges), wanted);
            } else {
                queue.remove(index, &mut edges);
                expected.remove(&(edges[index].key, index));
                edges[index].key = Edge::new(
                    (random % 1000) as u32 * 32 + u32::try_from(index).expect("small queue index"),
                    100_000 + u32::try_from(index).expect("small queue index"),
                );
                if random & 3 != 1 {
                    queue.insert(index, &mut edges);
                    expected.insert((edges[index].key, index));
                }
            }
            assert_eq!(queue.heap.len(), expected.len());
            assert_eq!(
                queue.heap.capacity(),
                capacity,
                "updates never grow the queue"
            );
            for (position, &edge) in queue.heap.iter().enumerate() {
                assert_eq!(edges[edge].queued_at, position);
                assert!(expected.contains(&(edges[edge].key, edge)));
            }
        }
        while let Some((_, wanted)) = expected.pop_first() {
            assert_eq!(queue.pop(&mut edges), Some(wanted));
        }
        assert_eq!(queue.pop(&mut edges), None);
    }
    use alloc::vec::Vec;

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
