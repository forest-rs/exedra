// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic, budgeted Delaunay refinement with generated vertices.
//!
//! The refiner starts from a legalized cover whose boundary edges are exactly
//! the edges with one incident triangle, so constraints need no separate
//! bookkeeping: an edge without a neighbor is a boundary segment. It then
//! runs quality-directed Ruppert-style refinement: a boundary segment that
//! blocks required quality work splits at its midpoint before the worst
//! remaining triangle by circumradius-to-shortest-edge ratio receives its
//! circumcenter. Boundary encroachment by a compliant or input-limited
//! triangle is not chased on its own.
//!
//! Topological decisions use the exact predicates; only the generated
//! coordinates, the quality ratio, and the encroachment test are rounded
//! floating-point arithmetic. Every candidate insertion is checked with
//! `orient2d` before mutation, so the cover never contains a non-CCW
//! triangle, and a candidate that would produce one is declined and counted.
//! Ordered sets drive all iteration, so results are independent of hash
//! order and allocation addresses.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::cmp::Reverse;

use crate::predicates::{InCircle, Orientation, incircle, orient2d};
use crate::{BoundarySplits, RefineParams, RefineStats, SteinerOrigin};

/// Neighbor sentinel for a boundary edge.
const NONE: u32 = u32::MAX;

/// Normalized undirected edge key.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Edge([u32; 2]);

impl Edge {
    const fn new(a: u32, b: u32) -> Self {
        if a < b { Self([a, b]) } else { Self([b, a]) }
    }
}

/// Live ownership of one boundary segment plus its input provenance.
#[derive(Copy, Clone, Debug)]
struct Segment {
    tri: u32,
    slot: u8,
    origin: [u32; 2],
}

/// Result of locating a point from a starting triangle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Locate {
    Inside(u32),
    OnEdge(u32, u8),
    OnVertex,
    Blocked(u32, u8),
    Lost,
}

/// Output of one refinement run over a legalized cover.
pub(crate) struct Refined {
    pub(crate) triangles: Vec<[u32; 3]>,
    pub(crate) origins: Vec<SteinerOrigin>,
    pub(crate) stats: RefineStats,
}

/// Refines `triangles`, a legalized CCW cover over `points`, appending
/// generated vertices to `points`.
pub(crate) fn refine_cover(
    points: &mut Vec<[f64; 2]>,
    triangles: Vec<[u32; 3]>,
    params: &RefineParams,
    initial_flips: usize,
) -> Refined {
    if !can_append_triangle_ids(triangles.len(), 0) {
        // `NONE` reserves the largest u32 value for adjacency, so a cover
        // that already needs more triangle IDs cannot safely enter the
        // append-only refiner. Preserve the cover and report the bounded
        // stop instead of reaching an overflowing triangle index.
        let mut output: Vec<[u32; 3]> = triangles.into_iter().map(canonical).collect();
        output.sort_unstable();
        let stats = RefineStats {
            edge_flips: initial_flips,
            budget_exhausted: true,
            ..RefineStats::default()
        };
        return Refined {
            triangles: output,
            origins: Vec::new(),
            stats,
        };
    }
    let mut refiner = Refiner::new(points, triangles, params);
    refiner.stats.edge_flips = initial_flips;
    refiner.run();
    refiner.finish()
}

struct Refiner<'a> {
    points: &'a mut Vec<[f64; 2]>,
    tris: Vec<[u32; 3]>,
    adj: Vec<[u32; 3]>,
    alive: Vec<bool>,
    segments: BTreeMap<Edge, Segment>,
    encroached: BTreeSet<Edge>,
    unsplittable: BTreeSet<Edge>,
    bad: BTreeSet<(Reverse<u64>, u32)>,
    origins: Vec<SteinerOrigin>,
    ratio2_bound: f64,
    max_steiner: usize,
    split_boundary: bool,
    stats: RefineStats,
}

impl<'a> Refiner<'a> {
    fn new(points: &'a mut Vec<[f64; 2]>, triangles: Vec<[u32; 3]>, params: &RefineParams) -> Self {
        let count = triangles.len();
        let mut refiner = Self {
            points,
            tris: triangles,
            adj: alloc::vec![[NONE; 3]; count],
            alive: alloc::vec![true; count],
            segments: BTreeMap::new(),
            encroached: BTreeSet::new(),
            unsplittable: BTreeSet::new(),
            bad: BTreeSet::new(),
            origins: Vec::new(),
            ratio2_bound: params.max_radius_edge_ratio * params.max_radius_edge_ratio,
            max_steiner: params.max_steiner_points as usize,
            split_boundary: params.boundary_splits == BoundarySplits::Allowed,
            stats: RefineStats::default(),
        };
        refiner.link_neighbors();
        for tri in 0..count {
            refiner.register(tri_index(tri));
        }
        refiner
    }

    /// Pairs every shared edge; unpaired edges become boundary segments whose
    /// provenance is the pair of input indices bounding them.
    fn link_neighbors(&mut self) {
        let mut seen: BTreeMap<Edge, (u32, u8)> = BTreeMap::new();
        for tri in 0..self.tris.len() {
            let index = tri_index(tri);
            for slot in 0..3_u8 {
                let edge = self.edge(index, slot);
                match seen.get(&edge) {
                    Some(&(other, other_slot)) => {
                        self.adj[tri][slot as usize] = other;
                        self.adj[other as usize][other_slot as usize] = index;
                    }
                    None => {
                        seen.insert(edge, (index, slot));
                    }
                }
            }
        }
        for tri in 0..self.tris.len() {
            for slot in 0..3_u8 {
                if self.adj[tri][slot as usize] == NONE {
                    let edge = self.edge(tri_index(tri), slot);
                    self.segments.insert(
                        edge,
                        Segment {
                            tri: tri_index(tri),
                            slot,
                            origin: edge.0,
                        },
                    );
                }
            }
        }
    }

    fn run(&mut self) {
        loop {
            if self.stats.budget_exhausted {
                return;
            }
            if self.split_boundary
                && let Some(edge) = self.encroached.pop_first()
            {
                if !self.segments.contains_key(&edge) {
                    continue;
                }
                if self.budget_exhausted() {
                    self.stats.budget_exhausted = true;
                    return;
                }
                self.split_segment_midpoint(edge);
                continue;
            }
            let Some((Reverse(key), tri)) = self.bad.pop_first() else {
                return;
            };
            if !self.alive[tri as usize]
                || self.quality_key(tri) != Some(key)
                || self.input_limited(tri)
            {
                continue;
            }
            if self.budget_exhausted() {
                self.stats.budget_exhausted = true;
                return;
            }
            self.insert_circumcenter(tri);
        }
    }

    fn finish(self) -> Refined {
        let mut triangles: Vec<[u32; 3]> = self
            .tris
            .iter()
            .zip(&self.alive)
            .filter_map(|(&tri, &alive)| alive.then_some(canonical(tri)))
            .collect();
        triangles.sort_unstable();
        let mut stats = self.stats;
        for tri in (0..self.tris.len()).filter(|&tri| self.alive[tri]) {
            let index = tri_index(tri);
            if self.quality_key(index).is_some() {
                stats.remaining_bad_triangles += 1;
                if self.input_limited(index) {
                    stats.input_limited_triangles += 1;
                }
            }
        }
        stats.steiner_points = len_u32(self.origins.len());
        Refined {
            triangles,
            origins: self.origins,
            stats,
        }
    }

    fn budget_exhausted(&self) -> bool {
        self.origins.len() >= self.max_steiner
            || !can_append_u32_ids(self.points.len(), 1)
            || !can_append_triangle_ids(self.tris.len(), 1)
    }

    // --- geometry queries -------------------------------------------------

    fn point(&self, vertex: u32) -> [f64; 2] {
        self.points[vertex as usize]
    }

    fn vertex(&self, tri: u32, slot: u8) -> u32 {
        self.tris[tri as usize][slot as usize % 3]
    }

    /// Undirected edge opposite `slot`.
    fn edge(&self, tri: u32, slot: u8) -> Edge {
        Edge::new(self.vertex(tri, slot + 1), self.vertex(tri, slot + 2))
    }

    fn slot_of(&self, tri: u32, vertex: u32) -> u8 {
        let verts = self.tris[tri as usize];
        if verts[0] == vertex {
            0
        } else if verts[1] == vertex {
            1
        } else {
            debug_assert_eq!(verts[2], vertex, "vertex must belong to the triangle");
            2
        }
    }

    fn slot_facing(&self, tri: u32, neighbor: u32) -> u8 {
        let adj = self.adj[tri as usize];
        if adj[0] == neighbor {
            0
        } else if adj[1] == neighbor {
            1
        } else {
            debug_assert_eq!(adj[2], neighbor, "neighbor must be adjacent");
            2
        }
    }

    /// Squared circumradius over squared shortest edge when the triangle
    /// violates the bound, encoded as an order-preserving key. Non-finite
    /// ratios count as violations.
    fn quality_key(&self, tri: u32) -> Option<u64> {
        let [a, b, c] = self.tris[tri as usize].map(|v| self.point(v));
        let sa = dist2(b, c);
        let sb = dist2(c, a);
        let sc = dist2(a, b);
        let shortest = sa.min(sb).min(sc);
        let area2 = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        let scale = shortest / area2;
        let ratio2 = (sa / shortest) * (sb / shortest) * (sc / shortest) * scale * scale / 4.0;
        if !ratio2.is_finite() {
            return Some(u64::MAX);
        }
        (ratio2 > self.ratio2_bound).then(|| ratio2.to_bits())
    }

    /// Whether the triangle's smallest angle sits between two boundary
    /// segments. No insertion can raise an angle the input itself fixes, so
    /// such triangles are reported instead of refined; chasing them would
    /// only split segments until the budget ran out.
    fn input_limited(&self, tri: u32) -> bool {
        let [a, b, c] = self.tris[tri as usize].map(|v| self.point(v));
        let lengths = [dist2(b, c), dist2(c, a), dist2(a, b)];
        let mut apex = 0;
        for slot in 1..3 {
            if lengths[slot] < lengths[apex] {
                apex = slot;
            }
        }
        let adj = self.adj[tri as usize];
        adj[(apex + 1) % 3] == NONE && adj[(apex + 2) % 3] == NONE
    }

    /// Strict diametral-circle test in rounded arithmetic.
    fn encroaches(&self, vertex: u32, edge: Edge) -> bool {
        self.encroaches_point(self.point(vertex), edge)
    }

    fn encroaches_point(&self, point: [f64; 2], edge: Edge) -> bool {
        let p = self.point(edge.0[0]);
        let q = self.point(edge.0[1]);
        (point[0] - p[0]) * (point[0] - q[0]) + (point[1] - p[1]) * (point[1] - q[1]) < 0.0
    }

    /// Records a created or rewritten live triangle: segment ownership,
    /// encroachment of its segment edges by its apexes, and quality.
    fn register(&mut self, tri: u32) {
        let quality = self.quality_key(tri);
        let input_limited = quality.is_some() && self.input_limited(tri);
        for slot in 0..3_u8 {
            if self.adj[tri as usize][slot as usize] != NONE {
                continue;
            }
            let edge = self.edge(tri, slot);
            if let Some(segment) = self.segments.get_mut(&edge) {
                segment.tri = tri;
                segment.slot = slot;
            }
            // An encroached segment only matters when this triangle needs a
            // quality insertion. Input-limited violations and compliant
            // triangles must not turn their input geometry into refinement
            // work; candidate insertions still queue blocking segments via
            // `defer_to_segments` below.
            if self.split_boundary
                && quality.is_some()
                && !input_limited
                && self.encroaches(self.vertex(tri, slot), edge)
            {
                self.queue_segment(edge);
            }
        }
        if let Some(key) = quality {
            self.bad.insert((Reverse(key), tri));
        }
    }

    fn orientation(&self, a: u32, b: u32, c: u32) -> Orientation {
        orient2d(self.point(a), self.point(b), self.point(c))
    }

    // --- point location ---------------------------------------------------

    /// Visibility walk from `start` toward `target`, bounded by the triangle
    /// count so a pathological walk declines instead of looping.
    fn locate(&self, start: u32, target: [f64; 2]) -> Locate {
        let mut tri = start;
        for _ in 0..self.tris.len() + 8 {
            let mut collinear: Option<u8> = None;
            let mut collinear_count = 0;
            let mut next = None;
            for slot in 0..3_u8 {
                let from = self.point(self.vertex(tri, slot + 1));
                let to = self.point(self.vertex(tri, slot + 2));
                match orient2d(from, to, target) {
                    Orientation::Ccw => {}
                    Orientation::Cw => {
                        next = Some(slot);
                        break;
                    }
                    Orientation::Collinear => {
                        collinear_count += 1;
                        collinear = Some(slot);
                    }
                }
            }
            match next {
                Some(slot) => {
                    let neighbor = self.adj[tri as usize][slot as usize];
                    if neighbor == NONE {
                        return Locate::Blocked(tri, slot);
                    }
                    tri = neighbor;
                }
                None => {
                    return match collinear_count {
                        0 => Locate::Inside(tri),
                        1 => Locate::OnEdge(tri, collinear.expect("one collinear slot")),
                        _ => Locate::OnVertex,
                    };
                }
            }
        }
        Locate::Lost
    }

    // --- insertion --------------------------------------------------------

    fn decline(&mut self) {
        self.stats.declined_insertions += 1;
    }

    /// Queues a boundary segment for a midpoint split unless a previous
    /// attempt proved it unsplittable. Returns whether it was queued.
    fn queue_segment(&mut self, edge: Edge) -> bool {
        if self.unsplittable.contains(&edge) {
            return false;
        }
        self.encroached.insert(edge);
        true
    }

    /// Ruppert's rule: a circumcenter that encroaches segments is not
    /// inserted; the segments split first and the triangle is reconsidered.
    /// When no split can be queued the triangle is declined instead, so a
    /// declined split can never cycle.
    fn defer_to_segments(&mut self, tri: u32, segments: &[Edge]) {
        if !self.split_boundary {
            self.decline();
            return;
        }
        let mut queued = false;
        for &edge in segments {
            queued |= self.queue_segment(edge);
        }
        if !queued {
            self.decline();
            return;
        }
        if let Some(key) = self.quality_key(tri) {
            self.bad.insert((Reverse(key), tri));
        }
    }

    /// Live boundary segments whose diametral circles contain `point`.
    ///
    /// Some input vertices may intentionally leave segments encroached when
    /// their angle is input-limited. A cavity-only walk would then rely on a
    /// false global no-encroachment invariant and miss a segment outside the
    /// cavity, so every live constrained segment is checked deterministically.
    fn encroached_by(&self, point: [f64; 2]) -> Vec<Edge> {
        self.segments
            .keys()
            .copied()
            .filter(|&edge| self.encroaches_point(point, edge))
            .collect()
    }

    fn insert_circumcenter(&mut self, tri: u32) {
        let [a, b, c] = self.tris[tri as usize].map(|v| self.point(v));
        let Some(center) = circumcenter(a, b, c) else {
            self.decline();
            return;
        };
        let (target, on_edge) = match self.locate(tri, center) {
            Locate::Inside(target) => (target, None),
            Locate::OnEdge(target, slot) => {
                if self.adj[target as usize][slot as usize] == NONE {
                    // Exactly on a segment: that segment is encroached.
                    self.defer_to_segments(tri, &[self.edge(target, slot)]);
                    return;
                }
                (target, Some(slot))
            }
            Locate::Blocked(target, slot) => {
                self.defer_to_segments(tri, &[self.edge(target, slot)]);
                return;
            }
            Locate::OnVertex | Locate::Lost => {
                self.decline();
                return;
            }
        };
        let encroached = self.encroached_by(center);
        if !encroached.is_empty() {
            self.defer_to_segments(tri, &encroached);
            return;
        }
        match on_edge {
            None => self.split_triangle(target, center),
            Some(slot) => self.split_edge(target, slot, center),
        }
    }

    fn push_vertex(&mut self, point: [f64; 2], origin: SteinerOrigin) -> Option<u32> {
        if !can_append_u32_ids(self.points.len(), 1) {
            self.stats.budget_exhausted = true;
            return None;
        }
        let Some(index) = u32::try_from(self.points.len()).ok() else {
            self.stats.budget_exhausted = true;
            return None;
        };
        self.points.push(point);
        self.origins.push(origin);
        Some(index)
    }

    fn triangle_base(&mut self, additions: usize) -> Option<u32> {
        if !can_append_triangle_ids(self.tris.len(), additions) {
            self.stats.budget_exhausted = true;
            return None;
        }
        let Some(index) = u32::try_from(self.tris.len()).ok() else {
            self.stats.budget_exhausted = true;
            return None;
        };
        Some(index)
    }

    fn new_triangle(&mut self, verts: [u32; 3], adj: [u32; 3]) {
        self.tris.push(verts);
        self.adj.push(adj);
        self.alive.push(true);
    }

    fn kill(&mut self, tri: u32) {
        self.alive[tri as usize] = false;
    }

    /// Repoints `neighbor`'s reference to `old` at `new`.
    fn relink(&mut self, neighbor: u32, old: u32, new: u32) {
        if neighbor != NONE {
            let slot = self.slot_facing(neighbor, old);
            self.adj[neighbor as usize][slot as usize] = new;
        }
    }

    /// Splits `tri` into three around an interior point.
    fn split_triangle(&mut self, tri: u32, point: [f64; 2]) {
        let [v0, v1, v2] = self.tris[tri as usize];
        let [n0, n1, n2] = self.adj[tri as usize];
        let Some(base) = self.triangle_base(3) else {
            return;
        };
        let Some(p) = self.push_vertex(point, SteinerOrigin::Interior) else {
            return;
        };
        self.stats.interior_insertions += 1;
        self.kill(tri);
        let (t0, t1, t2) = (base, base + 1, base + 2);
        self.new_triangle([v0, v1, p], [t1, t2, n2]);
        self.new_triangle([v1, v2, p], [t2, t0, n0]);
        self.new_triangle([v2, v0, p], [t0, t1, n1]);
        self.relink(n2, tri, t0);
        self.relink(n0, tri, t1);
        self.relink(n1, tri, t2);
        for created in [t0, t1, t2] {
            self.register(created);
        }
        self.legalize(p, alloc::vec![t0, t1, t2]);
    }

    /// Splits the interior edge opposite `slot` of `tri` and its neighbor
    /// into four triangles around a point exactly on that edge.
    fn split_edge(&mut self, tri: u32, slot: u8, point: [f64; 2]) {
        let neighbor = self.adj[tri as usize][slot as usize];
        let nslot = self.slot_facing(neighbor, tri);
        let u = self.vertex(tri, slot + 1);
        let w = self.vertex(tri, slot + 2);
        let r = self.vertex(tri, slot);
        let o = self.vertex(neighbor, nslot);
        let a = self.adj[tri as usize][(slot as usize + 2) % 3];
        let b = self.adj[tri as usize][(slot as usize + 1) % 3];
        let c = self.adj[neighbor as usize][(nslot as usize + 2) % 3];
        let d = self.adj[neighbor as usize][(nslot as usize + 1) % 3];
        let Some(base) = self.triangle_base(4) else {
            return;
        };
        let Some(p) = self.push_vertex(point, SteinerOrigin::Interior) else {
            return;
        };
        self.stats.interior_insertions += 1;
        self.kill(tri);
        self.kill(neighbor);
        let (ta, tb, tc, td) = (base, base + 1, base + 2, base + 3);
        self.new_triangle([u, p, r], [tb, a, td]);
        self.new_triangle([p, w, r], [b, ta, tc]);
        self.new_triangle([w, p, o], [td, c, tb]);
        self.new_triangle([p, u, o], [d, tc, ta]);
        self.relink(a, tri, ta);
        self.relink(b, tri, tb);
        self.relink(c, neighbor, tc);
        self.relink(d, neighbor, td);
        for created in [ta, tb, tc, td] {
            self.register(created);
        }
        self.legalize(p, alloc::vec![ta, tb, tc, td]);
    }

    fn split_segment_midpoint(&mut self, edge: Edge) {
        let p = self.point(edge.0[0]);
        let q = self.point(edge.0[1]);
        let midpoint = [p[0].midpoint(q[0]), p[1].midpoint(q[1])];
        self.split_segment_at(edge, midpoint);
    }

    /// Splits boundary segment `edge` at `point`, declining when the result
    /// would not be two strictly CCW triangles.
    fn split_segment_at(&mut self, edge: Edge, point: [f64; 2]) {
        let Some(segment) = self.segments.get(&edge).copied() else {
            self.decline();
            return;
        };
        let tri = segment.tri;
        let slot = segment.slot;
        let p = self.vertex(tri, slot + 1);
        let q = self.vertex(tri, slot + 2);
        let r = self.vertex(tri, slot);
        let (pp, pq, pr) = (self.point(p), self.point(q), self.point(r));
        if point == pp
            || point == pq
            || orient2d(pp, point, pr) != Orientation::Ccw
            || orient2d(point, pq, pr) != Orientation::Ccw
        {
            self.unsplittable.insert(edge);
            self.decline();
            return;
        }
        let Some(base) = self.triangle_base(2) else {
            return;
        };
        let a = self.adj[tri as usize][(slot as usize + 2) % 3];
        let b = self.adj[tri as usize][(slot as usize + 1) % 3];
        let Some(m) = self.push_vertex(
            point,
            SteinerOrigin::Boundary {
                edge: segment.origin,
            },
        ) else {
            return;
        };
        self.stats.boundary_splits += 1;
        self.kill(tri);
        let (t1, t2) = (base, base + 1);
        self.new_triangle([p, m, r], [t2, a, NONE]);
        self.new_triangle([m, q, r], [b, t1, NONE]);
        self.relink(a, tri, t1);
        self.relink(b, tri, t2);
        self.segments.remove(&edge);
        self.segments.insert(
            Edge::new(p, m),
            Segment {
                tri: t1,
                slot: 2,
                origin: segment.origin,
            },
        );
        self.segments.insert(
            Edge::new(m, q),
            Segment {
                tri: t2,
                slot: 2,
                origin: segment.origin,
            },
        );
        self.register(t1);
        self.register(t2);
        self.legalize(m, alloc::vec![t1, t2]);
    }

    /// Lawson legalization of the edges opposite `p` in the given triangles,
    /// using the crate's cocircular tie rule.
    fn legalize(&mut self, p: u32, mut stack: Vec<u32>) {
        while let Some(tri) = stack.pop() {
            let k = self.slot_of(tri, p);
            let neighbor = self.adj[tri as usize][k as usize];
            if neighbor == NONE {
                continue;
            }
            let u = self.vertex(tri, k + 1);
            let w = self.vertex(tri, k + 2);
            let kn = self.slot_facing(neighbor, tri);
            let o = self.vertex(neighbor, kn);
            let opposite = matches!(
                (self.orientation(p, o, u), self.orientation(p, o, w)),
                (Orientation::Ccw, Orientation::Cw) | (Orientation::Cw, Orientation::Ccw)
            );
            if !opposite {
                continue;
            }
            let [a, b, c] = self.tris[tri as usize].map(|v| self.point(v));
            let illegal = match incircle(a, b, c, self.point(o)) {
                InCircle::Inside => true,
                InCircle::Outside => false,
                InCircle::Cocircular => Edge::new(p, o) < Edge::new(u, w),
            };
            if !illegal {
                continue;
            }
            // Outer neighbors: across (w, p) and (p, u) of `tri`, across
            // (u, o) and (o, w) of `neighbor`.
            let across_wp = self.adj[tri as usize][(k as usize + 1) % 3];
            let across_pu = self.adj[tri as usize][(k as usize + 2) % 3];
            let across_uo = self.adj[neighbor as usize][(kn as usize + 1) % 3];
            let across_ow = self.adj[neighbor as usize][(kn as usize + 2) % 3];
            self.tris[tri as usize] = [p, u, o];
            self.adj[tri as usize] = [across_uo, neighbor, across_pu];
            self.tris[neighbor as usize] = [p, o, w];
            self.adj[neighbor as usize] = [across_ow, across_wp, tri];
            self.relink(across_uo, neighbor, tri);
            self.relink(across_wp, tri, neighbor);
            self.stats.edge_flips += 1;
            self.register(tri);
            self.register(neighbor);
            stack.push(tri);
            stack.push(neighbor);
        }
    }
}

fn tri_index(index: usize) -> u32 {
    len_u32(index)
}

fn len_u32(len: usize) -> u32 {
    u32::try_from(len).expect("triangle and vertex counts fit the validated u32 budget")
}

/// Whether `additions` entries fit after `current_len` in a `u32`-indexed
/// append-only container. The entry at `u32::MAX` is valid, so the container
/// may hold `u32::MAX + 1` entries even though its length cannot fit in `u32`.
fn can_append_u32_ids(current_len: usize, additions: usize) -> bool {
    let capacity = u64::from(u32::MAX) + 1;
    let current_len = current_len as u64;
    let additions = additions as u64;
    current_len <= capacity && additions <= capacity - current_len
}

/// Whether `additions` triangle entries fit in the `u32` triangle-ID space.
/// Triangle ID `u32::MAX` is reserved for the boundary-neighbor sentinel, so
/// triangles have one fewer usable ID than ordinary vertex-indexed entries.
fn can_append_triangle_ids(current_len: usize, additions: usize) -> bool {
    let capacity = u64::from(u32::MAX);
    let current_len = current_len as u64;
    let additions = additions as u64;
    current_len <= capacity && additions <= capacity - current_len
}

fn dist2(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    dx * dx + dy * dy
}

/// Circumcenter of a CCW triangle in rounded arithmetic relative to `a`.
fn circumcenter(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<[f64; 2]> {
    let bx = b[0] - a[0];
    let by = b[1] - a[1];
    let cx = c[0] - a[0];
    let cy = c[1] - a[1];
    let d = 2.0 * (bx * cy - by * cx);
    let b2 = bx * bx + by * by;
    let c2 = cx * cx + cy * cy;
    let ux = (cy * b2 - by * c2) / d;
    let uy = (bx * c2 - cx * b2) / d;
    let center = [a[0] + ux, a[1] + uy];
    (center[0].is_finite() && center[1].is_finite()).then_some(center)
}

const fn canonical([a, b, c]: [u32; 3]) -> [u32; 3] {
    if a < b && a < c {
        [a, b, c]
    } else if b < c {
        [b, c, a]
    } else {
        [c, a, b]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delaunay::illegal_edge_count;
    use crate::{PolygonInput, TriError, refine};

    fn square(side: f64) -> [[f64; 2]; 4] {
        [[0.0, 0.0], [side, 0.0], [side, side], [0.0, side]]
    }

    /// Independent circumradius-to-shortest-edge check.
    fn ratio_ok(points: &[[f64; 2]], [a, b, c]: [u32; 3], bound: f64) -> bool {
        let (a, b, c) = (points[a as usize], points[b as usize], points[c as usize]);
        let la = dist2(b, c);
        let lb = dist2(c, a);
        let lc = dist2(a, b);
        let area2 = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        // R = abc / (4A)  =>  R^2 = la lb lc / (4 area2^2)
        let r2 = la * lb * lc / (4.0 * area2 * area2);
        r2 <= bound * bound * la.min(lb).min(lc) * (1.0 + 1e-12)
    }

    #[test]
    fn refine_rejects_invalid_bounds() {
        // The quality comparison is meaningful only for a finite positive
        // radius-edge bound; each other floating-point class is typed input.
        let outer = square(1.0);
        let input = PolygonInput {
            outer: &outer,
            holes: &[],
        };
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let params = RefineParams {
                max_radius_edge_ratio: bad,
                ..RefineParams::default()
            };
            assert_eq!(refine(&input, &params), Err(TriError::InvalidParams));
        }
    }

    #[test]
    fn zero_budget_returns_the_legalized_cover() {
        // A zero work budget still performs deterministic legalization and
        // must not claim exhaustion when that initial cover meets the bound.
        let outer = square(1.0);
        let input = PolygonInput {
            outer: &outer,
            holes: &[],
        };
        let params = RefineParams {
            max_steiner_points: 0,
            ..RefineParams::default()
        };
        let result = refine(&input, &params).expect("square refines");
        assert_eq!(result.points.len(), 4);
        assert_eq!(result.triangles, [[0, 1, 2], [0, 2, 3]]);
        assert!(result.steiner.is_empty());
        assert!(
            !result.stats.budget_exhausted,
            "a square already meets sqrt(2)"
        );
    }

    #[test]
    fn budget_stops_with_pending_work_reported() {
        // A long thin strip needs many insertions to reach 30 degrees.
        let outer = [[0.0, 0.0], [40.0, 0.0], [40.0, 1.0], [0.0, 1.0]];
        let input = PolygonInput {
            outer: &outer,
            holes: &[],
        };
        let params = RefineParams {
            max_radius_edge_ratio: 1.0,
            max_steiner_points: 5,
            ..RefineParams::default()
        };
        let result = refine(&input, &params).expect("strip refines");
        assert_eq!(result.stats.steiner_points, 5);
        assert_eq!(result.steiner.len(), 5);
        assert!(result.stats.budget_exhausted);
        assert!(result.stats.remaining_bad_triangles > 0);
    }

    #[test]
    fn strip_reaches_thirty_degrees_with_boundary_splits() {
        // With boundary splitting enabled and enough budget, every triangle
        // in a long strip must reach the requested 30-degree lower bound.
        let outer = [[0.0, 0.0], [8.0, 0.0], [8.0, 1.0], [0.0, 1.0]];
        let input = PolygonInput {
            outer: &outer,
            holes: &[],
        };
        let params = RefineParams {
            max_radius_edge_ratio: 1.0,
            ..RefineParams::default()
        };
        let result = refine(&input, &params).expect("strip refines");
        assert!(!result.stats.budget_exhausted);
        assert_eq!(result.stats.remaining_bad_triangles, 0);
        assert!(result.stats.boundary_splits > 0);
        for &tri in &result.triangles {
            assert!(
                ratio_ok(&result.points, tri, 1.0),
                "{tri:?} violates the bound"
            );
        }
        assert_eq!(illegal_edge_count(&result.points, &result.triangles), 0);
        for origin in &result.steiner {
            if let SteinerOrigin::Boundary { edge } = *origin {
                assert!(
                    edge[0] < edge[1] && edge[1] < 4,
                    "origin names input vertices"
                );
            }
        }
    }

    #[test]
    fn forbidden_splits_restore_collinear_samples_across_ring_seams() {
        let outer = [
            [0.0, 0.0],
            [2.0, 0.0],
            [8.0, 0.0],
            [10.0, 0.0],
            [10.0, 4.0],
            [0.0, 4.0],
        ];
        let hole = [[3.0, 1.0], [3.0, 3.0], [7.0, 3.0], [7.0, 1.0], [5.0, 1.0]];
        for offset in 0..outer.len() {
            let mut outer = outer;
            outer.rotate_left(offset);
            let mut hole = hole;
            let hole_offset = offset % hole.len();
            hole.rotate_left(hole_offset);
            let result = refine(
                &PolygonInput {
                    outer: &outer,
                    holes: &[&hole],
                },
                &RefineParams::default().with_boundary_splits(BoundarySplits::Forbidden),
            )
            .expect("collinear rings refine");
            let mut edges = BTreeMap::<Edge, usize>::new();
            for &[a, b, c] in &result.triangles {
                assert_eq!(
                    orient2d(
                        result.points[a as usize],
                        result.points[b as usize],
                        result.points[c as usize]
                    ),
                    Orientation::Ccw
                );
                for edge in [Edge::new(a, b), Edge::new(b, c), Edge::new(c, a)] {
                    *edges.entry(edge).or_default() += 1;
                }
            }
            let mut expected = BTreeSet::new();
            for (base, len) in [(0, outer.len()), (outer.len(), hole.len())] {
                for index in 0..len {
                    expected.insert(Edge::new(
                        len_u32(base + index),
                        len_u32(base + (index + 1) % len),
                    ));
                }
            }
            assert!(edges.values().all(|&count| count <= 2));
            assert_eq!(
                edges
                    .into_iter()
                    .filter_map(|(edge, count)| (count == 1).then_some(edge))
                    .collect::<BTreeSet<_>>(),
                expected
            );
            assert_eq!(illegal_edge_count(&result.points, &result.triangles), 0);
            assert!(
                result
                    .steiner
                    .iter()
                    .all(|origin| *origin == SteinerOrigin::Interior)
            );
        }
    }

    #[test]
    fn forbidden_boundary_splits_keep_the_boundary_and_report_declines() {
        // Interior-only refinement must preserve the four original boundary
        // edges and report work blocked by that deliberate restriction.
        let outer = [[0.0, 0.0], [8.0, 0.0], [8.0, 1.0], [0.0, 1.0]];
        let input = PolygonInput {
            outer: &outer,
            holes: &[],
        };
        let params = RefineParams {
            max_radius_edge_ratio: 1.0,
            boundary_splits: BoundarySplits::Forbidden,
            ..RefineParams::default()
        };
        let result = refine(&input, &params).expect("strip refines");
        assert_eq!(result.stats.boundary_splits, 0);
        assert!(
            result
                .steiner
                .iter()
                .all(|origin| *origin == SteinerOrigin::Interior)
        );
        assert!(result.stats.declined_insertions > 0);
        // The boundary is exactly the four input edges.
        let mut boundary = Vec::new();
        let mut edges: Vec<[u32; 2]> = result
            .triangles
            .iter()
            .flat_map(|&[a, b, c]| {
                [
                    [a.min(b), a.max(b)],
                    [b.min(c), b.max(c)],
                    [c.min(a), c.max(a)],
                ]
            })
            .collect();
        edges.sort_unstable();
        let mut index = 0;
        while index < edges.len() {
            let run = edges[index..]
                .iter()
                .take_while(|&&e| e == edges[index])
                .count();
            assert!(
                run <= 2,
                "edge {:?} has {run} incident triangles",
                edges[index]
            );
            if run == 1 {
                boundary.push(edges[index]);
            }
            index += run;
        }
        assert_eq!(boundary, [[0, 1], [0, 3], [1, 2], [2, 3]]);
    }

    #[test]
    fn power_of_two_scaling_is_exact() {
        // Power-of-two scale changes are exact in binary64, so they must
        // preserve topology and work while scaling every generated point.
        let base = [[0.0, 0.0], [5.0, 0.0], [5.0, 1.0], [2.5, 1.5], [0.0, 1.0]];
        let reference = refine(
            &PolygonInput {
                outer: &base,
                holes: &[],
            },
            &RefineParams::default(),
        )
        .expect("base refines");
        assert!(reference.stats.steiner_points > 0);
        for exponent in [-60_i32, -3, 7, 90] {
            let scale = 2.0_f64.powi(exponent);
            let scaled: Vec<[f64; 2]> = base.iter().map(|p| [p[0] * scale, p[1] * scale]).collect();
            let result = refine(
                &PolygonInput {
                    outer: &scaled,
                    holes: &[],
                },
                &RefineParams::default(),
            )
            .expect("scaled refines");
            assert_eq!(result.triangles, reference.triangles, "exponent {exponent}");
            assert_eq!(result.stats, reference.stats, "exponent {exponent}");
            for (got, expected) in result.points.iter().zip(&reference.points) {
                assert_eq!(*got, [expected[0] * scale, expected[1] * scale]);
            }
        }
    }

    #[test]
    fn holed_polygon_keeps_hole_boundaries_on_their_segments() {
        // Generated boundary points on either ring must name their source
        // edge, while interior points must remain outside the material hole.
        let outer = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = [[4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0]];
        let input = PolygonInput {
            outer: &outer,
            holes: &[&hole],
        };
        let result = refine(&input, &RefineParams::default()).expect("holed square refines");
        assert_eq!(result.stats.remaining_bad_triangles, 0);
        for (index, origin) in result.steiner.iter().enumerate() {
            let point = result.points[result.input_vertex_count as usize + index];
            match *origin {
                SteinerOrigin::Boundary { edge } => {
                    let a = result.points[edge[0] as usize];
                    let b = result.points[edge[1] as usize];
                    assert_eq!(
                        orient2d(a, b, point),
                        Orientation::Collinear,
                        "axis-aligned midpoints are exact"
                    );
                }
                SteinerOrigin::Interior => {
                    let inside_outer =
                        point[0] > 0.0 && point[0] < 10.0 && point[1] > 0.0 && point[1] < 10.0;
                    let inside_hole =
                        point[0] > 4.0 && point[0] < 6.0 && point[1] > 4.0 && point[1] < 6.0;
                    assert!(inside_outer && !inside_hole, "{point:?} left the domain");
                }
            }
        }
    }

    #[test]
    fn input_fixed_acute_corner_does_not_chase_encroached_segments() {
        // The tiny input angle cannot be improved by splitting its incident
        // boundary edges, so it must stop without consuming the Steiner cap.
        let outer = [[0.0, 0.0], [10.0, 0.0], [0.1, 0.01]];
        let result = refine(
            &PolygonInput {
                outer: &outer,
                holes: &[],
            },
            &RefineParams::default(),
        )
        .expect("acute triangle refines");
        assert_eq!(result.stats.steiner_points, 0);
        assert_eq!(result.stats.remaining_bad_triangles, 1);
        assert_eq!(result.stats.input_limited_triangles, 1);
        assert!(!result.stats.budget_exhausted);
        assert_eq!(result.triangles, [[0, 1, 2]]);
        assert_eq!(illegal_edge_count(&result.points, &result.triangles), 0);
    }

    #[test]
    fn quality_compliant_obtuse_triangle_does_not_split_boundary() {
        // This obtuse triangle satisfies the default ratio bound; its
        // encroached base is therefore irrelevant refinement work.
        let outer = [[0.0, 0.0], [2.0, 0.0], [1.0, 0.84]];
        let result = refine(
            &PolygonInput {
                outer: &outer,
                holes: &[],
            },
            &RefineParams::default(),
        )
        .expect("obtuse triangle refines");
        assert_eq!(result.stats.steiner_points, 0);
        assert_eq!(result.stats.boundary_splits, 0);
        assert_eq!(result.stats.remaining_bad_triangles, 0);
        assert_eq!(result.triangles, [[0, 1, 2]]);
        assert_eq!(illegal_edge_count(&result.points, &result.triangles), 0);
    }

    #[test]
    fn candidate_checks_residual_encroachment_outside_its_cavity() {
        // An input vertex can still encroach the shallow base after initial
        // registration stops chasing input-limited work. A later candidate
        // must find that segment even when the cavity walk stops earlier.
        let mut points = alloc::vec![
            [-1.0, 0.0], // p: shallow boundary base
            [1.0, 0.0],  // q
            [0.0, 0.1],  // c: encroaches p-q
            [0.0, 0.9],  // d
            [0.4, 0.5],  // e: with c and d, circumcenter is (0, 0.5)
        ];
        let refiner = Refiner::new(
            &mut points,
            alloc::vec![[0, 1, 2], [2, 3, 0], [3, 2, 4]],
            &RefineParams::default(),
        );
        assert!(
            refiner.encroached_by([0.0, 0.5]).contains(&Edge::new(0, 1)),
            "global segment scan must include the encroached base"
        );
    }

    #[test]
    fn u32_id_append_capacity_stops_at_the_inclusive_maximum() {
        // Test the arithmetic near the representable boundary without
        // allocating a vector anywhere near that size.
        let max_id = u32::MAX as usize;
        assert!(can_append_u32_ids(max_id, 1));
        assert!(!can_append_u32_ids(max_id, 2));
        assert!(can_append_u32_ids(max_id.saturating_sub(3), 4));
        assert!(!can_append_u32_ids(max_id.saturating_sub(3), 5));
        assert!(can_append_triangle_ids(max_id.saturating_sub(1), 1));
        assert!(!can_append_triangle_ids(max_id, 1));
        if let Ok(full_len) = usize::try_from(u64::from(u32::MAX) + 1) {
            assert!(can_append_u32_ids(full_len, 0));
            assert!(!can_append_u32_ids(full_len, 1));
        }
    }
}
