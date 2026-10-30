// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic ear clipping over an index ring.
//!
//! The ring is a cyclic sequence of point indices (indices may repeat once
//! hole bridging splices rings together). Each round clips the valid ear
//! whose vertex has the lowest `(point index, ring position)` — the stable
//! tie-break Exedra brief 11 prescribes — so output triangle order is a pure
//! function of the input.
//!
//! Hole bridging synthesizes a weakly simple traversal with repeated vertices.
//! Exhausting ears in that traversal is a construction failure, not evidence
//! that the original input boundaries intersect. The caller diagnoses original
//! boundary contacts separately.

use alloc::vec::Vec;

use crate::{TriError, TriangulationStage};
use exedra_math::predicates::{Orientation, orient2d};

/// Narrows a validated count to `u32`.
///
/// Callers guarantee the `u32` index budget via [`crate::PolygonInput::validate`].
pub(crate) fn len_u32(n: usize) -> u32 {
    debug_assert!(u32::try_from(n).is_ok(), "index budget validated upstream");
    #[expect(
        clippy::cast_possible_truncation,
        reason = "PolygonInput::validate bounds the vertex count to u32"
    )]
    {
        n as u32
    }
}

/// Clips `ring` (a cyclic sequence of indices into `points`, counter-
/// clockwise) into triangles appended to `out`.
pub(crate) fn earclip_ring(
    points: &[[f64; 2]],
    ring: &[u32],
    out: &mut Vec<[u32; 3]>,
) -> Result<(), TriError> {
    let mut live = RingState::new(points, ring);
    live.prune_consecutive_duplicates();

    // The live candidate list retains (point index, ring position) order.
    // Removing a vertex unlinks it, so later rounds never scan dead slots.
    while live.len > 3 {
        let mut clipped = false;
        let mut collinear = None;
        let mut pos = live.scan_head;
        for _ in 0..live.len {
            let next_candidate = live.vertices[pos as usize].scan_next;
            match live.classify(pos) {
                VertexClass::CollinearBetween => {
                    // Prefer genuine ears. In a bridged polygon, vertices
                    // from separate aligned boundary components can become
                    // temporarily collinear in the composite ring; pruning
                    // one before adjacent ears are clipped changes which
                    // input boundary the triangulation represents.
                    let _ = collinear.get_or_insert(pos);
                }
                VertexClass::Spike => return Err(stalled()),
                VertexClass::Reflex => {}
                VertexClass::Convex => {
                    if live.is_ear(pos) {
                        live.emit(pos, out);
                        live.remove(pos);
                        clipped = true;
                        break;
                    }
                }
            }
            pos = next_candidate;
        }
        if !clipped && let Some(pos) = collinear {
            live.remove(pos);
            clipped = true;
        }
        if !clipped {
            return Err(stalled());
        }
    }

    if live.len == 3 {
        let pos = (0..len_u32(live.vertices.len()))
            .find(|&p| live.vertices[p as usize].alive)
            .expect("live.len == 3 implies a live position exists");
        match live.classify(pos) {
            VertexClass::Convex => live.emit(pos, out),
            // Final degenerate triangle: nothing left worth emitting.
            VertexClass::CollinearBetween | VertexClass::Spike => {}
            VertexClass::Reflex => return Err(stalled()),
        }
    }
    Ok(())
}

fn stalled() -> TriError {
    TriError::TriangulationFailed {
        stage: TriangulationStage::EarClipping,
    }
}

/// Computes twice the signed area of `ring` over `points`, in ring order.
///
/// Positive means counter-clockwise. Plain f64 accumulation in a fixed
/// order: deterministic, but cancellation at extreme dynamic ranges can lose
/// the area or its sign even for nondegenerate input.
pub(crate) fn twice_signed_area(points: &[[f64; 2]], ring: &[u32]) -> f64 {
    let mut sum = 0.0;
    for (i, &a) in ring.iter().enumerate() {
        let b = ring[(i + 1) % ring.len()];
        let pa = points[a as usize];
        let pb = points[b as usize];
        sum += (pa[0] - pb[0]) * (pa[1] + pb[1]);
    }
    sum
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum VertexClass {
    Convex,
    Reflex,
    /// Collinear with its neighbors and strictly between them.
    CollinearBetween,
    /// Collinear with its neighbors but outside the segment they span: a
    /// zero-width spike, which no simple polygon contains.
    Spike,
}

struct RingVertex {
    next: u32,
    prev: u32,
    scan_next: u32,
    scan_prev: u32,
    alive: bool,
    class: Option<VertexClass>,
    /// A live witness blocks this ear until either neighbor changes.
    blocked_by: Option<u32>,
}

struct RingState<'a> {
    points: &'a [[f64; 2]],
    ring: &'a [u32],
    vertices: Vec<RingVertex>,
    /// Static spatial order; dead positions are skipped at query time.
    spatial: Vec<u32>,
    spatial_axis: usize,
    scan_head: u32,
    len: usize,
}

impl<'a> RingState<'a> {
    fn new(points: &'a [[f64; 2]], ring: &'a [u32]) -> Self {
        let n = len_u32(ring.len());
        let mut vertices: Vec<_> = (0..n)
            .map(|i| RingVertex {
                next: (i + 1) % n,
                prev: (i + n - 1) % n,
                scan_next: 0,
                scan_prev: 0,
                alive: true,
                class: None,
                blocked_by: None,
            })
            .collect();
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for &index in ring {
            let point = points[index as usize];
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        let spatial_axis = usize::from(max[1] - min[1] > max[0] - min[0]);
        let mut spatial: Vec<_> = (0..n).collect();
        // Link the stable candidate list, then reuse this allocation for
        // spatial queries. Neither index is rebuilt while clipping.
        spatial.sort_unstable_by_key(|&pos| (ring[pos as usize], pos));
        let scan_head = spatial.first().copied().unwrap_or(0);
        for (i, &pos) in spatial.iter().enumerate() {
            vertices[pos as usize].scan_next = spatial[(i + 1) % spatial.len()];
            vertices[pos as usize].scan_prev = spatial[(i + spatial.len() - 1) % spatial.len()];
        }
        spatial.sort_unstable_by(|&a, &b| {
            points[ring[a as usize] as usize][spatial_axis]
                .total_cmp(&points[ring[b as usize] as usize][spatial_axis])
                .then_with(|| a.cmp(&b))
        });
        Self {
            points,
            ring,
            vertices,
            spatial,
            spatial_axis,
            scan_head,
            len: ring.len(),
        }
    }

    fn coords(&self, pos: u32) -> [f64; 2] {
        self.points[self.ring[pos as usize] as usize]
    }

    /// Removes exactly coincident consecutive vertices before selecting ears.
    fn prune_consecutive_duplicates(&mut self) {
        let n = len_u32(self.ring.len());
        let mut pos = 0_u32;
        let mut steps = 0;
        while steps < 2 * n as usize && self.len > 3 {
            if !self.vertices[pos as usize].alive {
                pos = (pos + 1) % n;
                steps += 1;
                continue;
            }
            let nxt = self.vertices[pos as usize].next;
            if nxt != pos && self.coords(pos) == self.coords(nxt) {
                // Keep the earlier ring position for stability.
                self.remove(nxt);
                steps = 0;
            } else {
                pos = (pos + 1) % n;
                steps += 1;
            }
        }
    }

    fn classify(&mut self, pos: u32) -> VertexClass {
        if let Some(class) = self.vertices[pos as usize].class {
            return class;
        }
        let class = self.compute_class(pos);
        self.vertices[pos as usize].class = Some(class);
        class
    }

    fn compute_class(&self, pos: u32) -> VertexClass {
        let p = self.coords(self.vertices[pos as usize].prev);
        let v = self.coords(pos);
        let n = self.coords(self.vertices[pos as usize].next);
        match orient2d(p, v, n) {
            Orientation::Ccw => VertexClass::Convex,
            Orientation::Cw => VertexClass::Reflex,
            Orientation::Collinear => {
                let between_x = (p[0].min(n[0]) <= v[0]) && (v[0] <= p[0].max(n[0]));
                let between_y = (p[1].min(n[1]) <= v[1]) && (v[1] <= p[1].max(n[1]));
                if between_x && between_y {
                    VertexClass::CollinearBetween
                } else {
                    VertexClass::Spike
                }
            }
        }
    }

    /// True when the convex vertex at `pos` is an ear: no other live vertex
    /// lies inside or on its triangle.
    ///
    /// Vertices whose coordinates exactly equal a triangle corner are
    /// skipped — bridged rings visit bridge endpoints twice, and a
    /// coincident copy must not block the ear its twin participates in.
    fn is_ear(&mut self, pos: u32) -> bool {
        if let Some(blocker) = self.vertices[pos as usize].blocked_by
            && self.vertices[blocker as usize].alive
        {
            return false;
        }
        let p_pos = self.vertices[pos as usize].prev;
        let n_pos = self.vertices[pos as usize].next;
        let p = self.coords(p_pos);
        let v = self.coords(pos);
        let n = self.coords(n_pos);

        // A point outside these inclusive bounds cannot lie in the triangle.
        // Use comparisons only: no rounded geometric predicate or tolerance
        // can discard a point on an edge, even at extreme coordinate scales.
        let min = [p[0].min(v[0]).min(n[0]), p[1].min(v[1]).min(n[1])];
        let max = [p[0].max(v[0]).max(n[0]), p[1].max(v[1]).max(n[1])];

        let axis = self.spatial_axis;
        let start = self
            .spatial
            .partition_point(|&pos| self.coords(pos)[axis] < min[axis]);
        for &q_pos in &self.spatial[start..] {
            let q = self.coords(q_pos);
            if q[axis] > max[axis] {
                break;
            }
            if !self.vertices[q_pos as usize].alive {
                continue;
            }
            if q[0] >= min[0]
                && q[0] <= max[0]
                && q[1] >= min[1]
                && q[1] <= max[1]
                && q != p
                && q != v
                && q != n
                && triangle_contains(p, v, n, q)
            {
                self.vertices[pos as usize].blocked_by = Some(q_pos);
                return false;
            }
        }
        self.vertices[pos as usize].blocked_by = None;
        true
    }

    fn emit(&self, pos: u32, out: &mut Vec<[u32; 3]>) {
        let p = self.ring[self.vertices[pos as usize].prev as usize];
        let v = self.ring[pos as usize];
        let n = self.ring[self.vertices[pos as usize].next as usize];
        out.push([p, v, n]);
    }

    fn remove(&mut self, pos: u32) {
        let p = self.vertices[pos as usize].prev;
        let n = self.vertices[pos as usize].next;
        self.vertices[p as usize].next = n;
        self.vertices[n as usize].prev = p;
        self.vertices[pos as usize].alive = false;
        let scan_previous = self.vertices[pos as usize].scan_prev;
        let scan_next = self.vertices[pos as usize].scan_next;
        self.vertices[scan_previous as usize].scan_next = scan_next;
        self.vertices[scan_next as usize].scan_prev = scan_previous;
        if self.scan_head == pos {
            self.scan_head = scan_next;
        }
        self.len -= 1;
        // Only these two turns changed. An unchanged triangle's live blocker
        // remains inside it, independent of edits elsewhere in the ring.
        for neighbor in [p, n] {
            self.vertices[neighbor as usize].blocked_by = None;
            self.vertices[neighbor as usize].class = None;
        }
    }
}

/// True when `q` lies inside or on the counter-clockwise triangle `(a, b, c)`.
fn triangle_contains(a: [f64; 2], b: [f64; 2], c: [f64; 2], q: [f64; 2]) -> bool {
    orient2d(a, b, q) != Orientation::Cw
        && orient2d(b, c, q) != Orientation::Cw
        && orient2d(c, a, q) != Orientation::Cw
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full traversal without spatial rejection or a cached blocking witness.
    fn linear_is_ear(state: &RingState<'_>, pos: u32) -> bool {
        let previous = state.vertices[pos as usize].prev;
        let next = state.vertices[pos as usize].next;
        let [a, b, c] = [
            state.coords(previous),
            state.coords(pos),
            state.coords(next),
        ];
        let mut q = state.vertices[next as usize].next;
        while q != previous {
            let point = state.coords(q);
            if point != a && point != b && point != c && triangle_contains(a, b, c, point) {
                return false;
            }
            q = state.vertices[q as usize].next;
        }
        true
    }

    fn check_each_removal(points: &[[f64; 2]], ring: &[u32]) {
        let mut state = RingState::new(points, ring);
        state.prune_consecutive_duplicates();
        let mut order: Vec<_> = (0..len_u32(ring.len())).collect();
        order.sort_unstable_by_key(|&pos| (ring[pos as usize], pos));
        while state.len > 3 {
            let expected_order: Vec<_> = order
                .iter()
                .copied()
                .filter(|&pos| state.vertices[pos as usize].alive)
                .collect();
            let mut actual_order = Vec::new();
            let mut candidate = state.scan_head;
            for _ in 0..state.len {
                actual_order.push(candidate);
                candidate = state.vertices[candidate as usize].scan_next;
            }
            assert_eq!(candidate, state.scan_head);
            assert_eq!(actual_order, expected_order);
            let mut ear = None;
            let mut collinear = None;
            for &pos in &order {
                if !state.vertices[pos as usize].alive {
                    continue;
                }
                assert_eq!(state.classify(pos), state.compute_class(pos));
                match state.classify(pos) {
                    VertexClass::Convex => {
                        let expected = linear_is_ear(&state, pos);
                        assert_eq!(state.is_ear(pos), expected);
                        // Repeating the query exercises the witness cache.
                        assert_eq!(state.is_ear(pos), expected);
                        if expected && ear.is_none() {
                            ear = Some(pos);
                        }
                    }
                    VertexClass::CollinearBetween => {
                        collinear.get_or_insert(pos);
                    }
                    VertexClass::Reflex => {}
                    VertexClass::Spike => panic!("valid test ring developed a spike"),
                }
            }
            state.remove(ear.or(collinear).expect("ring must have an ear"));
        }
    }

    #[test]
    fn spatial_queries_match_full_traversal_through_bridged_ring_removals() {
        let original = [
            [-0.0, 0.0],
            [12.0, 0.0],
            [12.0, 12.0],
            [0.0, 12.0],
            [2.0, 2.0],
            [2.0, 4.0],
            [4.0, 4.0],
            [4.0, 2.0],
            [7.0, 5.0],
            [7.0, 9.0],
            [10.0, 9.0],
            [10.0, 5.0],
        ];
        for scale in [1e-300, 1e-90, 1.0, 1e90] {
            for transpose in [false, true] {
                let points: Vec<_> = original
                    .iter()
                    .map(|&[x, y]| {
                        if transpose {
                            [-y * scale, x * scale]
                        } else {
                            [x * scale, y * scale]
                        }
                    })
                    .collect();
                let ring = crate::bridge::bridge_holes(
                    &points,
                    alloc::vec![0, 1, 2, 3],
                    &[(4, 4), (8, 4)],
                    false,
                    false,
                )
                .expect("two separated holes");
                check_each_removal(&points, &ring);
            }
        }
    }

    #[test]
    fn cached_ear_is_reconsidered_when_its_blocker_is_removed() {
        let points = [[0.0, 0.0], [6.0, 0.0], [6.0, 6.0], [3.0, 2.0], [0.0, 6.0]];
        let mut state = RingState::new(&points, &[0, 1, 2, 3, 4]);
        assert!(!state.is_ear(0));
        assert_eq!(state.vertices[0].blocked_by, Some(3));
        state.remove(3);
        assert!(state.is_ear(0));
    }

    #[test]
    fn changing_an_ear_neighbor_invalidates_its_live_blocker() {
        let points = [[0.0, 0.0], [6.0, 0.0], [6.0, 6.0], [3.0, 2.0], [0.0, 6.0]];
        let mut state = RingState::new(&points, &[0, 1, 2, 3, 4]);
        assert!(!state.is_ear(0));
        state.remove(1);
        assert!(state.vertices[3].alive);
        assert_eq!(state.vertices[0].blocked_by, None);
        assert!(state.is_ear(0));
    }
}
