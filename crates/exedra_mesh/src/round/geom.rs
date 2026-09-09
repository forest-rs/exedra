// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! f64 geometry helpers for the rounding pass.
//!
//! Geometry is constructed in f64 and narrowed to f32 exactly once when new
//! vertices materialize — the same single-narrowing discipline as boolean
//! splitting and constructive tessellation. Vector arithmetic and the
//! promote/narrow pair come from `exedra_math`; what lives here is the
//! polygon-level work the pass defines the meaning of: Newell normals, plane
//! solves, line closest approach, and arc sampling.

use alloc::vec::Vec;

use exedra_math::{add, cross, distance_squared, dot, norm, normalize, scale, sub};

use crate::math::FloatExt;

/// Unnormalized Newell normal of a polygon.
pub(super) fn newell(points: &[[f64; 3]]) -> [f64; 3] {
    let mut normal = [0.0_f64; 3];
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    normal
}

/// Maximum radial deficit of a triangle inscribed in a sphere at the origin.
/// The closest point is the plane projection when it lies inside the triangle,
/// or a point on one of its edges otherwise. A plane-only bound is too strict
/// for obtuse triangles and does not converge when a boundary edge is fixed.
pub(super) fn spherical_triangle_error(points: [[f64; 3]; 3], radius: f64) -> Option<f64> {
    let [a, b, c] = points;
    let normal = normalize(cross(sub(b, a), sub(c, a)))?;
    let height = dot(normal, a);
    let projection = scale(normal, height);
    let sides = [(a, b), (b, c), (c, a)];
    let inside = sides
        .iter()
        .all(|&(from, to)| dot(cross(sub(to, from), sub(projection, from)), normal) >= 0.0);
    let distance = if inside {
        height.abs()
    } else {
        sides.iter().fold(f64::INFINITY, |distance, &(from, to)| {
            let direction = sub(to, from);
            let t = (-dot(from, direction) / dot(direction, direction)).clamp(0.0, 1.0);
            distance.min(norm(add(from, scale(direction, t))))
        })
    };
    Some(radius - distance)
}

/// Split a corner-layer quad along its shorter diagonal, visiting the triangle
/// incident to the existing outer boundary (indices 1 and 2) first.
pub(super) fn corner_quad(points: [[f64; 3]; 4]) -> [[usize; 3]; 2] {
    if distance_squared(points[0], points[2]) <= distance_squared(points[1], points[3]) {
        [[0, 1, 2], [0, 2, 3]]
    } else {
        [[3, 1, 2], [0, 1, 3]]
    }
}

/// A fitted face plane: unit normal plus the maximum absolute deviation of
/// the fitted points from the centroid plane.
#[derive(Copy, Clone, Debug)]
pub(super) struct Plane {
    pub normal: [f64; 3],
    pub max_deviation: f64,
}

/// Solves `rows * x = rhs` via the adjugate; `None` when near-singular.
pub(super) fn solve3(rows: [[f64; 3]; 3], rhs: [f64; 3]) -> Option<[f64; 3]> {
    let det = dot(rows[0], cross(rows[1], rows[2]));
    if det.abs() <= 1e-12 {
        return None;
    }
    // For a matrix with rows m1, m2, m3 the inverse columns are the cross
    // products of the complementary rows.
    let x = add(
        add(
            scale(cross(rows[1], rows[2]), rhs[0]),
            scale(cross(rows[2], rows[0]), rhs[1]),
        ),
        scale(cross(rows[0], rows[1]), rhs[2]),
    );
    Some(scale(x, 1.0 / det))
}

/// Closest-approach midpoint of two lines plus the gap between them.
///
/// Returns `None` for (near-)parallel lines.
pub(super) fn line_intersection(
    p0: [f64; 3],
    d0: [f64; 3],
    p1: [f64; 3],
    d1: [f64; 3],
) -> Option<([f64; 3], f64)> {
    let a = dot(d0, d0);
    let b = dot(d0, d1);
    let c = dot(d1, d1);
    let denom = a * c - b * b;
    if denom.abs() <= 1e-12 * a * c {
        return None;
    }
    let w = sub(p0, p1);
    let d = dot(d0, w);
    let e = dot(d1, w);
    let s = (b * e - c * d) / denom;
    let t = (a * e - b * d) / denom;
    let q0 = add(p0, scale(d0, s));
    let q1 = add(p1, scale(d1, t));
    Some((scale(add(q0, q1), 0.5), norm(sub(q0, q1))))
}

/// Circular-arc interpolation from `from` to `to` around `center`.
///
/// The endpoints are reproduced exactly; interior samples sweep the angle
/// between the two center rays with linearly interpolated radius, so mildly
/// asymmetric inputs (averaged frames) still yield watertight sharing.
/// Returns `segments + 1` points, or `None` for degenerate rays.
pub(super) fn arc_points(
    center: [f64; 3],
    from: [f64; 3],
    to: [f64; 3],
    segments: u32,
) -> Option<Vec<[f64; 3]>> {
    let ray_from = sub(from, center);
    let ray_to = sub(to, center);
    let radius_from = norm(ray_from);
    let radius_to = norm(ray_to);
    let u = normalize(ray_from)?;
    let v = normalize(ray_to)?;
    let cos_sweep = dot(u, v).clamp(-1.0, 1.0);
    let sweep = cos_sweep.acos_ext();
    let w = normalize(sub(v, scale(u, cos_sweep)))?;

    let mut points = Vec::with_capacity(segments as usize + 1);
    points.push(from);
    for k in 1..segments {
        let fraction = f64::from(k) / f64::from(segments);
        let theta = sweep * fraction;
        let radius = radius_from + (radius_to - radius_from) * fraction;
        let direction = add(scale(u, theta.cos_ext()), scale(w, theta.sin_ext()));
        points.push(add(center, scale(direction, radius)));
    }
    points.push(to);
    Some(points)
}
