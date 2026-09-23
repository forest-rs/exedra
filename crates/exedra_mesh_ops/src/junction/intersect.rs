// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact intersection tests between triangles that may share vertices.
//!
//! Every decision uses exact orientation predicates on the input coordinates,
//! so two triangles are reported as intersecting exactly when their closed
//! point sets meet anywhere other than at the vertices and edges they share
//! by identity. A triangle whose vertices are exactly collinear, or
//! coincide, is treated as the union of its edges.

use exedra_triangulate::predicates::{Orientation, Orientation3d, orient2d, orient3d};

use super::{cross, sub};

/// True when the triangles meet other than along the vertices and edges they
/// share. Vertex identity is given by `ia` and `ib`.
pub(super) fn triangles_intersect<V: Copy + Eq>(
    ia: [V; 3],
    a: &[[f64; 3]; 3],
    ib: [V; 3],
    b: &[[f64; 3]; 3],
) -> bool {
    if !bounds_overlap(a, b) {
        return false;
    }
    let (degenerate_a, degenerate_b) = (degenerate(a), degenerate(b));
    if degenerate_a || degenerate_b {
        // Orientation tests against a degenerate triangle report every point
        // as coplanar, so its point set is taken as the union of its edges.
        let (ia, a, ib, b, degenerate_b) = if degenerate_a {
            (ia, a, ib, b, degenerate_b)
        } else {
            (ib, b, ia, a, degenerate_a)
        };
        return (0..3).any(|k| {
            let (u, v) = (ia[k], ia[(k + 1) % 3]);
            let (p, q) = (a[k], a[(k + 1) % 3]);
            edge_meets_triangle([u, v], [p, q], ib, b, degenerate_b)
        });
    }
    let shared = ia.iter().filter(|v| ib.contains(v)).count();
    let edges = |t: &[[f64; 3]; 3]| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])];
    match shared {
        0 => {
            edges(a)
                .iter()
                .any(|&(p, q)| segment_hits_triangle(p, q, b))
                || edges(b)
                    .iter()
                    .any(|&(p, q)| segment_hits_triangle(p, q, a))
        }
        1 => {
            // Starting at the shared vertex, an edge can meet the other
            // triangle elsewhere only through the far edges.
            let far = |ids: &[V; 3], t: &[[f64; 3]; 3], other: &[V; 3]| {
                let mut keep = (0..3).filter(|&k| !other.contains(&ids[k]));
                let first = keep.next().expect("two unshared vertices");
                let second = keep.next().expect("two unshared vertices");
                (t[first], t[second])
            };
            let (p, q) = far(&ia, a, &ib);
            let (r, s) = far(&ib, b, &ia);
            segment_hits_triangle(p, q, b) || segment_hits_triangle(r, s, a)
        }
        2 => {
            // Sharing an edge, they meet elsewhere only by folding onto each
            // other: coplanar, with both far vertices on the same side.
            let common_a = (0..3).filter(|&k| ib.contains(&ia[k]));
            let mut common = [0; 2];
            for (slot, k) in common.iter_mut().zip(common_a) {
                *slot = k;
            }
            let (u, w) = (a[common[0]], a[common[1]]);
            let far_a = a[(0..3).find(|k| !common.contains(k)).expect("far vertex")];
            let far_b = b[(0..3).find(|&k| !ia.contains(&ib[k])).expect("far vertex")];
            if orient3d(u, w, far_a, far_b) != Orientation3d::Coplanar {
                return false;
            }
            let project = projector(a);
            orient2d(project(u), project(w), project(far_a))
                == orient2d(project(u), project(w), project(far_b))
        }
        _ => true,
    }
}

/// True when the three points are exactly collinear or coincide: every
/// component of the cross product is zero, and each is an exact 2D
/// orientation of an axis-aligned projection.
fn degenerate(t: &[[f64; 3]; 3]) -> bool {
    collinear(t[0], t[1], t[2])
}

fn collinear(p: [f64; 3], q: [f64; 3], r: [f64; 3]) -> bool {
    (0..3).all(|axis| {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let project = |x: [f64; 3]| [x[u], x[v]];
        orient2d(project(p), project(q), project(r)) == Orientation::Collinear
    })
}

/// Closed segment `pq` (vertex ids `ids`) against triangle `b` (ids `ib`),
/// ignoring the segment's endpoints that `b` shares and the whole segment
/// when it is one of `b`'s edges.
fn edge_meets_triangle<V: Copy + Eq>(
    ids: [V; 2],
    [p, q]: [[f64; 3]; 2],
    ib: [V; 3],
    b: &[[f64; 3]; 3],
    b_degenerate: bool,
) -> bool {
    let shared_p = ib.contains(&ids[0]);
    let shared_q = ib.contains(&ids[1]);
    if shared_p && shared_q {
        return false;
    }
    if b_degenerate {
        return (0..3).any(|k| {
            let other = [ib[k], ib[(k + 1) % 3]];
            segments_meet_3d(ids, [p, q], other, [b[k], b[(k + 1) % 3]])
        });
    }
    if shared_p {
        leaves_vertex_into(p, q, ib, ids[0], b)
    } else if shared_q {
        leaves_vertex_into(q, p, ib, ids[1], b)
    } else {
        segment_hits_triangle(p, q, b)
    }
}

/// True when the segment from `b`'s vertex `start` (id `start_id`) toward
/// `end` meets the closed, non-degenerate triangle `b` anywhere past `start`.
fn leaves_vertex_into<V: Copy + Eq>(
    start: [f64; 3],
    end: [f64; 3],
    ib: [V; 3],
    start_id: V,
    b: &[[f64; 3]; 3],
) -> bool {
    if start == end || orient3d(b[0], b[1], b[2], end) != Orientation3d::Coplanar {
        return false;
    }
    let k = (0..3).find(|&k| ib[k] == start_id).expect("shared vertex");
    let project = projector(b);
    let (s, e) = (project(start), project(end));
    let (mut u, mut w) = (project(b[(k + 1) % 3]), project(b[(k + 2) % 3]));
    if orient2d(s, u, w) == Orientation::Cw {
        core::mem::swap(&mut u, &mut w);
    }
    // Inside the closed wedge at the vertex.
    orient2d(s, u, e) != Orientation::Cw && orient2d(s, e, w) != Orientation::Cw
}

/// Closed 3D segments, ignoring endpoints shared by identity.
fn segments_meet_3d<V: Copy + Eq>(
    ia: [V; 2],
    [p, q]: [[f64; 3]; 2],
    ib: [V; 2],
    [a, b]: [[f64; 3]; 2],
) -> bool {
    let shared = |x: V| ib.contains(&x);
    match (shared(ia[0]), shared(ia[1])) {
        (true, true) => false,
        (true, false) => continues_along(p, q, if ib[0] == ia[0] { b } else { a }),
        (false, true) => continues_along(q, p, if ib[0] == ia[1] { b } else { a }),
        (false, false) => closed_segments_meet(p, q, a, b),
    }
}

/// Two segments leaving the common point `c` overlap past it exactly when
/// they are collinear and point the same way.
fn continues_along(c: [f64; 3], q: [f64; 3], b: [f64; 3]) -> bool {
    if c == q || c == b || !collinear(c, q, b) {
        return false;
    }
    let axis = (0..3).find(|&i| q[i] != c[i]).expect("distinct points");
    (q[axis] > c[axis]) == (b[axis] > c[axis])
}

fn closed_segments_meet(p: [f64; 3], q: [f64; 3], a: [f64; 3], b: [f64; 3]) -> bool {
    if orient3d(p, q, a, b) != Orientation3d::Coplanar {
        return false;
    }
    let points = [p, q, a, b];
    // A non-collinear triple fixes the common plane; its dominant normal axis
    // gives an injective projection.
    let triple = [[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]]
        .into_iter()
        .find(|&[i, j, k]| !collinear(points[i], points[j], points[k]));
    let Some([i, j, k]) = triple else {
        // All four on one line: compare intervals along an axis the line
        // varies in, or positions when it is a single point.
        let Some(axis) = (0..3).find(|&axis| points.iter().any(|x| x[axis] != p[axis])) else {
            return true;
        };
        let (low_pq, high_pq) = (p[axis].min(q[axis]), p[axis].max(q[axis]));
        let (low_ab, high_ab) = (a[axis].min(b[axis]), a[axis].max(b[axis]));
        return low_pq <= high_ab && low_ab <= high_pq;
    };
    let plane = [points[i], points[j], points[k]];
    let project = projector(&plane);
    segments_meet(project(p), project(q), project(a), project(b))
}

fn bounds_overlap(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> bool {
    (0..3).all(|axis| {
        let low = |t: &[[f64; 3]; 3]| t.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
        let high = |t: &[[f64; 3]; 3]| t.iter().map(|p| p[axis]).fold(f64::NEG_INFINITY, f64::max);
        low(a) <= high(b) && low(b) <= high(a)
    })
}

fn opposite(a: Orientation3d, b: Orientation3d) -> bool {
    matches!(
        (a, b),
        (Orientation3d::Above, Orientation3d::Below) | (Orientation3d::Below, Orientation3d::Above)
    )
}

/// Drops the coordinate along which the triangle's normal is largest. The
/// projection is a bijection of the triangle's plane, and it copies
/// coordinates, so 2D predicates on the result stay exact.
fn projector(t: &[[f64; 3]; 3]) -> impl Fn([f64; 3]) -> [f64; 2] {
    let normal = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let axis = (0..3)
        .max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .expect("three axes");
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    move |p: [f64; 3]| [p[u], p[v]]
}

/// Closed 2D segment intersection.
fn segments_meet(p: [f64; 2], q: [f64; 2], a: [f64; 2], b: [f64; 2]) -> bool {
    let strict = |x: Orientation, y: Orientation| {
        matches!(
            (x, y),
            (Orientation::Ccw, Orientation::Cw) | (Orientation::Cw, Orientation::Ccw)
        )
    };
    if strict(orient2d(a, b, p), orient2d(a, b, q)) && strict(orient2d(p, q, a), orient2d(p, q, b))
    {
        return true;
    }
    let on = |a: [f64; 2], b: [f64; 2], p: [f64; 2]| {
        orient2d(a, b, p) == Orientation::Collinear
            && p[0] >= a[0].min(b[0])
            && p[0] <= a[0].max(b[0])
            && p[1] >= a[1].min(b[1])
            && p[1] <= a[1].max(b[1])
    };
    on(a, b, p) || on(a, b, q) || on(p, q, a) || on(p, q, b)
}

/// Closed 2D point-in-triangle.
fn inside(p: [f64; 2], t: [[f64; 2]; 3]) -> bool {
    let o = [
        orient2d(t[0], t[1], p),
        orient2d(t[1], t[2], p),
        orient2d(t[2], t[0], p),
    ];
    !(o.contains(&Orientation::Ccw) && o.contains(&Orientation::Cw))
}

/// Closed segment against closed triangle.
fn segment_hits_triangle(p: [f64; 3], q: [f64; 3], t: &[[f64; 3]; 3]) -> bool {
    let op = orient3d(t[0], t[1], t[2], p);
    let oq = orient3d(t[0], t[1], t[2], q);
    if op == oq && op != Orientation3d::Coplanar {
        return false;
    }
    if op == Orientation3d::Coplanar && oq == Orientation3d::Coplanar {
        let project = projector(t);
        let (p, q) = (project(p), project(q));
        let t = t.map(&project);
        return inside(p, t)
            || inside(q, t)
            || (0..3).any(|k| segments_meet(p, q, t[k], t[(k + 1) % 3]));
    }
    // The segment reaches the plane; it meets the triangle when the line
    // through it passes inside or along every edge.
    let s = [
        orient3d(p, q, t[0], t[1]),
        orient3d(p, q, t[1], t[2]),
        orient3d(p, q, t[2], t[0]),
    ];
    !(opposite(s[0], s[1]) || opposite(s[1], s[2]) || opposite(s[0], s[2]))
}
