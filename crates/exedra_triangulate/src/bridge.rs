// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic hole bridging.
//!
//! Each hole is spliced into the outer ring through a bridge: a segment from
//! the hole's rightmost vertex to a ring vertex it can reach without crossing
//! any edge. The composite is a single degenerate-simple ring (bridge
//! vertices appear twice) that ear clipping consumes directly.
//!
//! Determinism: the first attempt preserves the historical order (rightmost
//! anchor first, equal x by ascending y, bridge candidates by stable input
//! label). The caller can request two fixed fallback orderings for aligned
//! holes: reverse only the equal-x y tie and/or scan bridge candidates by
//! distance. Every ordering has explicit final tie-breaks. Geometric tests use
//! exact-sign predicates; a bridge that passes through a vertex or lies in a
//! boundary edge's local cone is rejected rather than resolved by epsilon.

use alloc::vec::Vec;

use crate::TriError;
use crate::predicates::{Orientation, orient2d};

/// Splices `holes` into `ring`, returning the composite ring.
///
/// `ring` holds indices into `points`; each hole is a `(base, len)` range of
/// point indices (`base..base + len`) in clockwise order. The outer ring must
/// already be counter-clockwise.
pub(crate) fn bridge_holes(
    points: &[[f64; 2]],
    mut ring: Vec<u32>,
    holes: &[(u32, u32)],
    descending_y_ties: bool,
    nearest_candidates: bool,
) -> Result<Vec<u32>, TriError> {
    // Remove exactly-collinear boundary samples before choosing bridges.
    // Ear clipping performs the same area-preserving pruning later, but
    // bridge placement must not depend on redundant samples: with several
    // aligned holes they can force a degenerate bridge corridor that runs
    // out of ears even though the input region is simple. Indices still
    // address the original input points, so caller provenance is unchanged.
    prune_collinear_between(points, &mut ring);
    if ring.len() < 3 {
        return Err(TriError::NonSimple);
    }
    let mut hole_rings: Vec<Vec<u32>> = holes
        .iter()
        .map(|&(base, len)| (base..base + len).collect())
        .collect();
    for hole in &mut hole_rings {
        prune_collinear_between(points, hole);
        if hole.len() < 3 {
            return Err(TriError::NonSimple);
        }
    }
    // Rightmost anchors go first. The normal deterministic order uses lower
    // y first; the caller may reverse only this exact-x tie as a validated
    // fallback when aligned holes otherwise create a zero-width corridor.
    let mut order: Vec<usize> = (0..hole_rings.len()).collect();
    let anchors: Vec<u32> = hole_rings
        .iter()
        .map(|hole| anchor_vertex(points, hole))
        .collect();
    order.sort_unstable_by(|&a, &b| {
        let pa = points[anchors[a] as usize];
        let pb = points[anchors[b] as usize];
        pb[0]
            .partial_cmp(&pa[0])
            .expect("coordinates validated finite")
            .then_with(|| {
                if descending_y_ties {
                    pb[1]
                        .partial_cmp(&pa[1])
                        .expect("coordinates validated finite")
                } else {
                    pa[1]
                        .partial_cmp(&pb[1])
                        .expect("coordinates validated finite")
                }
            })
            .then(a.cmp(&b))
    });

    for (index, &anchor) in anchors.iter().enumerate() {
        if !inside_ring(points, &ring, points[anchor as usize]) {
            return Err(TriError::HoleOutsideOuter { hole: index });
        }
    }

    let mut composite = ring;
    for &hole_index in &order {
        composite = bridge_one(
            points,
            composite,
            &hole_rings[hole_index],
            anchors[hole_index],
            hole_index,
            &hole_rings,
            &order,
            nearest_candidates,
        )?;
    }
    Ok(composite)
}

/// The hole's anchor: rightmost vertex, ties by lower y, then lower index.
fn anchor_vertex(points: &[[f64; 2]], ring: &[u32]) -> u32 {
    let mut best = ring[0];
    for &i in &ring[1..] {
        let p = points[i as usize];
        let b = points[best as usize];
        if p[0] > b[0] || (p[0] == b[0] && p[1] < b[1]) {
            best = i;
        }
    }
    best
}

/// Splices one hole into `ring` via the first valid bridge.
#[expect(
    clippy::too_many_arguments,
    reason = "internal helper threading fixed evaluation context"
)]
fn bridge_one(
    points: &[[f64; 2]],
    ring: Vec<u32>,
    hole: &[u32],
    anchor: u32,
    hole_index: usize,
    holes: &[Vec<u32>],
    order: &[usize],
    nearest_candidates: bool,
) -> Result<Vec<u32>, TriError> {
    let h = points[anchor as usize];

    // Geometry decides whether a bridge is admissible. The historical label
    // order is tried first to preserve existing deterministic output. The
    // caller can retry nearest-first: aligned holes can make a far, low-label
    // bridge create a weakly-simple corridor that ear clipping cannot cover
    // without changing an input boundary.
    let mut candidates: Vec<u32> = (0..).take(ring.len()).collect();
    if nearest_candidates {
        candidates.sort_unstable_by(|&a, &b| {
            let pa = points[ring[a as usize] as usize];
            let pb = points[ring[b as usize] as usize];
            let distance_a = (pa[0] - h[0]) * (pa[0] - h[0]) + (pa[1] - h[1]) * (pa[1] - h[1]);
            let distance_b = (pb[0] - h[0]) * (pb[0] - h[0]) + (pb[1] - h[1]) * (pb[1] - h[1]);
            distance_a
                .partial_cmp(&distance_b)
                .expect("coordinates validated finite")
                .then(ring[a as usize].cmp(&ring[b as usize]))
                .then(a.cmp(&b))
        });
    } else {
        candidates.sort_unstable_by_key(|&pos| (ring[pos as usize], pos));
    }

    // Holes not yet spliced (processed after this one) still stand as
    // independent rings the bridge must not cross.
    let position = order
        .iter()
        .position(|&o| o == hole_index)
        .expect("hole_index comes from order");
    'candidate: for &pos in &candidates {
        let pos = pos as usize;
        let o = points[ring[pos] as usize];
        if o == h {
            continue;
        }
        // `blocks_bridge` intentionally ignores edges that share a bridge
        // endpoint. Check the endpoint cones separately: if the bridge is
        // collinear with either incident boundary edge, splicing duplicates
        // that edge as a zero-width corridor. A composite ring can contain
        // the same target coordinate more than once after earlier bridges,
        // so every occurrence must admit the new bridge.
        let anchor_position = hole
            .iter()
            .position(|&index| index == anchor)
            .expect("anchor belongs to hole");
        let hole_previous = points[hole[(anchor_position + hole.len() - 1) % hole.len()] as usize];
        let hole_next = points[hole[(anchor_position + 1) % hole.len()] as usize];
        if [hole_previous, hole_next]
            .into_iter()
            .any(|neighbor| orient2d(h, o, neighbor) == Orientation::Collinear)
        {
            continue;
        }
        for ring_pos in 0..ring.len() {
            if points[ring[ring_pos] as usize] != o {
                continue;
            }
            let previous = points[ring[(ring_pos + ring.len() - 1) % ring.len()] as usize];
            let next = points[ring[(ring_pos + 1) % ring.len()] as usize];
            if [previous, next]
                .into_iter()
                .any(|neighbor| orient2d(h, o, neighbor) == Orientation::Collinear)
            {
                continue 'candidate;
            }
        }
        // The bridge must clear every edge of the current composite ring…
        for i in 0..ring.len() {
            let a = points[ring[i] as usize];
            let b = points[ring[(i + 1) % ring.len()] as usize];
            if blocks_bridge(h, o, a, b) {
                continue 'candidate;
            }
        }
        // …every edge of this hole…
        for i in 0..hole.len() {
            let a = points[hole[i] as usize];
            let b = points[hole[(i + 1) % hole.len()] as usize];
            if blocks_bridge(h, o, a, b) {
                continue 'candidate;
            }
        }
        // …and every edge of the holes still waiting to be spliced.
        for &pending_index in &order[position + 1..] {
            let pending = &holes[pending_index];
            for i in 0..pending.len() {
                let a = points[pending[i] as usize];
                let b = points[pending[(i + 1) % pending.len()] as usize];
                if blocks_bridge(h, o, a, b) {
                    continue 'candidate;
                }
            }
        }
        return Ok(splice(
            &ring,
            u32::try_from(pos).expect("ring position originated as u32"),
            hole,
            anchor,
        ));
    }
    Err(TriError::UnbridgeableHole { hole: hole_index })
}

/// True when edge `(a, b)` blocks the bridge segment `(h, o)`.
///
/// Edges sharing an endpoint coordinate with the bridge are transparent;
/// among the rest, both proper crossings and exact touches block — a bridge
/// through a vertex or along an edge would create an ambiguous composite
/// ring, so it is rejected rather than resolved by epsilon.
fn blocks_bridge(h: [f64; 2], o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> bool {
    if a == h || a == o || b == h || b == o {
        return false;
    }
    let abh = orient2d(a, b, h);
    let abo = orient2d(a, b, o);
    let hoa = orient2d(h, o, a);
    let hob = orient2d(h, o, b);

    // Proper crossing: each segment strictly straddles the other's line.
    if abh != abo
        && hoa != hob
        && abh != Orientation::Collinear
        && abo != Orientation::Collinear
        && hoa != Orientation::Collinear
        && hob != Orientation::Collinear
    {
        return true;
    }
    // Exact touches and collinear overlaps: any endpoint lying on the other
    // segment blocks the bridge.
    (hoa == Orientation::Collinear && between(h, o, a))
        || (hob == Orientation::Collinear && between(h, o, b))
        || (abh == Orientation::Collinear && between(a, b, h))
        || (abo == Orientation::Collinear && between(a, b, o))
}

/// True when collinear point `q` lies within the axis-aligned span of `(a, b)`.
fn between(a: [f64; 2], b: [f64; 2], q: [f64; 2]) -> bool {
    (a[0].min(b[0]) <= q[0] && q[0] <= a[0].max(b[0]))
        && (a[1].min(b[1]) <= q[1] && q[1] <= a[1].max(b[1]))
}

/// Builds the composite ring: `ring[..=pos], hole cycle from the anchor,
/// anchor again, ring[pos..]`.
fn splice(ring: &[u32], pos: u32, hole: &[u32], anchor: u32) -> Vec<u32> {
    let pos = pos as usize;
    let anchor_position = hole
        .iter()
        .position(|&index| index == anchor)
        .expect("anchor belongs to hole");
    let mut out = Vec::with_capacity(ring.len() + hole.len() + 2);
    out.extend_from_slice(&ring[..=pos]);
    for i in 0..hole.len() {
        out.push(hole[(anchor_position + i) % hole.len()]);
    }
    out.push(anchor);
    out.extend_from_slice(&ring[pos..]);
    out
}

/// Removes vertices that lie exactly between their neighbors. Repeats until
/// stable because deleting one sample can expose a longer collinear run.
pub(crate) fn prune_collinear_between(points: &[[f64; 2]], ring: &mut Vec<u32>) {
    let mut changed = true;
    while changed && ring.len() >= 3 {
        changed = false;
        for i in 0..ring.len() {
            let previous = points[ring[(i + ring.len() - 1) % ring.len()] as usize];
            let current = points[ring[i] as usize];
            let next = points[ring[(i + 1) % ring.len()] as usize];
            if orient2d(previous, current, next) == Orientation::Collinear
                && between(previous, next, current)
            {
                ring.remove(i);
                changed = true;
                break;
            }
        }
    }
}

/// True when point `q` lies strictly inside the counter-clockwise `ring`,
/// by exact ray crossing count. Points exactly on the boundary count as
/// outside.
pub(crate) fn inside_ring(points: &[[f64; 2]], ring: &[u32], q: [f64; 2]) -> bool {
    let mut inside = false;
    for i in 0..ring.len() {
        let a = points[ring[i] as usize];
        let b = points[ring[(i + 1) % ring.len()] as usize];
        if a == q || b == q {
            return false;
        }
        if (a[1] > q[1]) != (b[1] > q[1]) {
            let side = orient2d(a, b, q);
            if side == Orientation::Collinear {
                return false;
            }
            // Upward edge: inside toggles when q is left of a->b; the edge
            // direction decides which exact side counts.
            let crosses = if b[1] > a[1] {
                side == Orientation::Ccw
            } else {
                side == Orientation::Cw
            };
            if crosses {
                inside = !inside;
            }
        }
    }
    inside
}
