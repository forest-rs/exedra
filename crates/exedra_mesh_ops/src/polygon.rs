// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Contact tests for finite polygonal boundaries in a shared XY frame.
//!
//! These predicates retain ordinary f64 determinant arithmetic. They are not
//! exact predicates and do not establish an arithmetic error bound. Callers
//! validate finiteness and bound pair work before using them for checked geometry.

/// Whether any edge of `first` crosses or touches any edge of `second`.
pub fn rings_intersect(first: &[[f64; 2]], second: &[[f64; 2]]) -> bool {
    first.iter().enumerate().any(|(first_index, &first_start)| {
        let first_end = first[(first_index + 1) % first.len()];
        second
            .iter()
            .enumerate()
            .any(|(second_index, &second_start)| {
                let second_end = second[(second_index + 1) % second.len()];
                segments_intersect(first_start, first_end, second_start, second_end)
            })
    })
}

/// Whether any two non-adjacent edges of a single ring cross or touch.
///
/// Adjacent edges always share an endpoint by construction, so only pairs
/// at least two apart around the cycle are tested. The ring must be free of
/// repeated vertices, or a zero-length
/// edge would make its neighbors look like a contact. Bounding boxes reject
/// the overwhelming majority of pairs before the orientation tests run.
pub fn ring_self_intersects(ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    if n < 4 {
        return false;
    }
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        let (ax0, ax1) = (a[0].min(b[0]), a[0].max(b[0]));
        let (ay0, ay1) = (a[1].min(b[1]), a[1].max(b[1]));
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let c = ring[j];
            let d = ring[(j + 1) % n];
            if c[0].min(d[0]) > ax1
                || c[0].max(d[0]) < ax0
                || c[1].min(d[1]) > ay1
                || c[1].max(d[1]) < ay0
            {
                continue;
            }
            if segments_intersect(a, b, c, d) {
                return true;
            }
        }
    }
    false
}

fn segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let ab_c = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd_a = cross(c, d, a);
    let cd_b = cross(c, d, b);
    ((ab_c > 0.0 && ab_d < 0.0) || (ab_c < 0.0 && ab_d > 0.0))
        && ((cd_a > 0.0 && cd_b < 0.0) || (cd_a < 0.0 && cd_b > 0.0))
        || ab_c == 0.0 && point_on_segment(c, a, b)
        || ab_d == 0.0 && point_on_segment(d, a, b)
        || cd_a == 0.0 && point_on_segment(a, c, d)
        || cd_b == 0.0 && point_on_segment(b, c, d)
}

fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_on_segment(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> bool {
    point[0] >= start[0].min(end[0])
        && point[0] <= start[0].max(end[0])
        && point[1] >= start[1].min(end[1])
        && point[1] <= start[1].max(end[1])
}
