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
//! With exact-sign predicates, the two-ears theorem gives an honest failure
//! mode: a simple polygon always has an ear, so exhausting candidates proves
//! the input was not a simple polygon (or degenerated to one), reported as
//! [`TriError::NonSimple`] rather than guessed around.

use alloc::vec::Vec;

use crate::TriError;
use crate::predicates::{Orientation, orient2d};

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

    // Scan order: ring positions sorted by (point index, position). Each
    // round clips the first live valid ear in this order.
    let mut scan: Vec<u32> = (0..len_u32(ring.len())).collect();
    scan.sort_unstable_by_key(|&pos| (ring[pos as usize], pos));

    while live.len > 3 {
        let mut clipped = false;
        let mut collinear = None;
        for &pos in &scan {
            if !live.alive[pos as usize] {
                continue;
            }
            match live.classify(pos) {
                VertexClass::CollinearBetween => {
                    // Prefer genuine ears. In a bridged polygon, vertices
                    // from separate aligned boundary components can become
                    // temporarily collinear in the composite ring; pruning
                    // one before adjacent ears are clipped changes which
                    // input boundary the triangulation represents.
                    let _ = collinear.get_or_insert(pos);
                }
                VertexClass::Spike => return Err(TriError::NonSimple),
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
        }
        if !clipped && let Some(pos) = collinear {
            live.remove(pos);
            clipped = true;
        }
        if !clipped {
            // Two-ears theorem: a simple polygon always has an ear, so the
            // input cannot have been simple.
            return Err(TriError::NonSimple);
        }
    }

    if live.len == 3 {
        let pos = (0..len_u32(live.alive.len()))
            .find(|&p| live.alive[p as usize])
            .expect("live.len == 3 implies a live position exists");
        match live.classify(pos) {
            VertexClass::Convex => live.emit(pos, out),
            // Final degenerate triangle: nothing left worth emitting.
            VertexClass::CollinearBetween | VertexClass::Spike => {}
            VertexClass::Reflex => return Err(TriError::NonSimple),
        }
    }
    Ok(())
}

/// Computes twice the signed area of `ring` over `points`, in ring order.
///
/// Positive means counter-clockwise. Plain f64 accumulation in a fixed
/// order: deterministic, and only sign-ambiguous for inputs that are
/// degenerate to begin with.
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

struct RingState<'a> {
    points: &'a [[f64; 2]],
    ring: &'a [u32],
    next: Vec<u32>,
    prev: Vec<u32>,
    alive: Vec<bool>,
    len: usize,
}

impl<'a> RingState<'a> {
    fn new(points: &'a [[f64; 2]], ring: &'a [u32]) -> Self {
        let n = len_u32(ring.len());
        let next = (0..n).map(|i| (i + 1) % n).collect();
        let prev = (0..n).map(|i| (i + n - 1) % n).collect();
        Self {
            points,
            ring,
            next,
            prev,
            alive: alloc::vec![true; ring.len()],
            len: ring.len(),
        }
    }

    fn coords(&self, pos: u32) -> [f64; 2] {
        self.points[self.ring[pos as usize] as usize]
    }

    /// Removes exactly coincident consecutive vertices so the classifier
    /// never sees zero-length edges.
    fn prune_consecutive_duplicates(&mut self) {
        let n = len_u32(self.ring.len());
        let mut pos = 0_u32;
        let mut steps = 0;
        while steps < 2 * n as usize && self.len > 3 {
            if !self.alive[pos as usize] {
                pos = (pos + 1) % n;
                steps += 1;
                continue;
            }
            let nxt = self.next[pos as usize];
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

    fn classify(&self, pos: u32) -> VertexClass {
        let p = self.coords(self.prev[pos as usize]);
        let v = self.coords(pos);
        let n = self.coords(self.next[pos as usize]);
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
    fn is_ear(&self, pos: u32) -> bool {
        let p_pos = self.prev[pos as usize];
        let n_pos = self.next[pos as usize];
        let p = self.coords(p_pos);
        let v = self.coords(pos);
        let n = self.coords(n_pos);

        let mut q_pos = self.next[n_pos as usize];
        while q_pos != p_pos {
            let q = self.coords(q_pos);
            if q != p && q != v && q != n && triangle_contains(p, v, n, q) {
                return false;
            }
            q_pos = self.next[q_pos as usize];
        }
        true
    }

    fn emit(&self, pos: u32, out: &mut Vec<[u32; 3]>) {
        let p = self.ring[self.prev[pos as usize] as usize];
        let v = self.ring[pos as usize];
        let n = self.ring[self.next[pos as usize] as usize];
        out.push([p, v, n]);
    }

    fn remove(&mut self, pos: u32) {
        let p = self.prev[pos as usize];
        let n = self.next[pos as usize];
        self.next[p as usize] = n;
        self.prev[n as usize] = p;
        self.alive[pos as usize] = false;
        self.len -= 1;
    }
}

/// True when `q` lies inside or on the counter-clockwise triangle `(a, b, c)`.
fn triangle_contains(a: [f64; 2], b: [f64; 2], c: [f64; 2], q: [f64; 2]) -> bool {
    orient2d(a, b, q) != Orientation::Cw
        && orient2d(b, c, q) != Orientation::Cw
        && orient2d(c, a, q) != Orientation::Cw
}
