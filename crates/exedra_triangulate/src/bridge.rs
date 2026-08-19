// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic hole bridging.
//!
//! Each hole is spliced into the outer ring through a bridge: a segment from
//! the hole's rightmost vertex to a ring vertex it can reach without crossing
//! any edge. The composite is a single degenerate-simple ring (bridge
//! vertices appear twice) that ear clipping consumes directly.
//!
//! Determinism: holes are processed in a fixed order (rightmost vertex,
//! descending x, ties by ascending y then hole index), the anchor vertex of
//! each hole is chosen by fixed tie-breaks, and bridge candidates are scanned
//! in ascending `(point index, ring position)`. Every geometric test is an
//! exact-sign predicate; a bridge that would pass exactly through a vertex is
//! rejected rather than resolved by epsilon.

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
    ring: Vec<u32>,
    holes: &[(u32, u32)],
) -> Result<Vec<u32>, TriError> {
    // Fixed processing order: rightmost anchor first.
    let mut order: Vec<usize> = (0..holes.len()).collect();
    let anchors: Vec<u32> = holes
        .iter()
        .map(|&(base, len)| anchor_vertex(points, base, len))
        .collect();
    order.sort_unstable_by(|&a, &b| {
        let pa = points[anchors[a] as usize];
        let pb = points[anchors[b] as usize];
        pb[0]
            .partial_cmp(&pa[0])
            .expect("coordinates validated finite")
            .then(
                pa[1]
                    .partial_cmp(&pb[1])
                    .expect("coordinates validated finite"),
            )
            .then(a.cmp(&b))
    });

    for (index, &anchor) in anchors.iter().enumerate() {
        if !inside_ring(points, &ring, points[anchor as usize]) {
            return Err(TriError::HoleOutsideOuter { hole: index });
        }
    }

    let mut composite = ring;
    for &hole_index in &order {
        let (base, len) = holes[hole_index];
        composite = bridge_one(
            points,
            composite,
            base,
            len,
            anchors[hole_index],
            hole_index,
            holes,
            &order,
        )?;
    }
    Ok(composite)
}

/// The hole's anchor: rightmost vertex, ties by lower y, then lower index.
fn anchor_vertex(points: &[[f64; 2]], base: u32, len: u32) -> u32 {
    let mut best = base;
    for i in base..base + len {
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
    base: u32,
    len: u32,
    anchor: u32,
    hole_index: usize,
    holes: &[(u32, u32)],
    order: &[usize],
) -> Result<Vec<u32>, TriError> {
    let h = points[anchor as usize];

    // Candidate ring positions in ascending (point index, position).
    let mut candidates: Vec<u32> = (0..).take(ring.len()).collect();
    candidates.sort_unstable_by_key(|&pos| (ring[pos as usize], pos));

    // Holes not yet spliced (processed after this one) still stand as
    // independent rings the bridge must not cross.
    let position = order
        .iter()
        .position(|&o| o == hole_index)
        .expect("hole_index comes from order");
    let pending: Vec<(u32, u32)> = order[position + 1..].iter().map(|&o| holes[o]).collect();

    'candidate: for &pos in &candidates {
        let o = points[ring[pos as usize] as usize];
        if o == h {
            continue;
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
        for i in 0..len {
            let a = points[(base + i) as usize];
            let b = points[(base + (i + 1) % len) as usize];
            if blocks_bridge(h, o, a, b) {
                continue 'candidate;
            }
        }
        // …and every edge of the holes still waiting to be spliced.
        for &(pbase, plen) in &pending {
            for i in 0..plen {
                let a = points[(pbase + i) as usize];
                let b = points[(pbase + (i + 1) % plen) as usize];
                if blocks_bridge(h, o, a, b) {
                    continue 'candidate;
                }
            }
        }
        return Ok(splice(&ring, pos, base, len, anchor));
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
fn splice(ring: &[u32], pos: u32, base: u32, len: u32, anchor: u32) -> Vec<u32> {
    let pos = pos as usize;
    let mut out = Vec::with_capacity(ring.len() + len as usize + 2);
    out.extend_from_slice(&ring[..=pos]);
    for i in 0..len {
        out.push(base + (anchor - base + i) % len);
    }
    out.push(anchor);
    out.extend_from_slice(&ring[pos..]);
    out
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
