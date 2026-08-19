// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact ray-parity point-in-mesh over promoted f64 triangles.
//!
//! Every geometric decision routes through the exact-sign
//! [`exedra_triangulate::predicates::orient3d`] predicate; a hit that lands
//! on an edge, vertex, or the plane of a triangle end-point is degenerate
//! and retried with the next deterministic direction instead of guessed.

use exedra_triangulate::predicates::{Orientation3d, orient3d};

/// Deterministic, pairwise-independent ray directions. Irrational-looking
/// components keep axis-aligned model edges off the ray in most retries.
const RAY_DIRECTIONS: [[f64; 3]; 8] = [
    [
        0.531_407_921_364_58,
        0.297_183_046_512_77,
        0.794_326_150_483_29,
    ],
    [
        -0.612_954_083_726_41,
        0.703_218_465_091_37,
        0.361_047_298_615_53,
    ],
    [
        0.409_617_253_048_81,
        -0.735_082_164_927_63,
        0.540_193_487_262_19,
    ],
    [
        0.263_508_147_296_84,
        0.441_275_063_918_46,
        -0.858_361_492_075_27,
    ],
    [
        -0.307_264_951_083_62,
        -0.552_483_017_694_28,
        0.774_058_326_149_71,
    ],
    [
        0.687_402_513_968_14,
        -0.219_536_084_712_93,
        -0.653_147_920_586_32,
    ],
    [
        -0.781_063_254_917_48,
        -0.469_218_530_764_12,
        -0.412_357_046_891_25,
    ],
    [
        0.919_463_082_571_36,
        0.152_084_637_291_58,
        0.348_916_205_473_61,
    ],
];

/// Parity result for one point.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum Parity {
    /// Odd crossings: inside.
    Inside,
    /// Even crossings: outside.
    Outside,
    /// All ray directions hit degeneracies; the point is unclassifiable.
    Exhausted,
}

/// Classifies a point against a closed triangle soup by exact ray parity.
#[must_use]
pub(crate) fn point_in_triangles(point: [f64; 3], triangles: &[[[f64; 3]; 3]]) -> Parity {
    // Ray length: everything is unit-ish scale; span the model generously.
    let mut extent = 1.0_f64;
    for triangle in triangles {
        for corner in triangle {
            for axis in 0..3 {
                extent = extent.max((corner[axis] - point[axis]).abs());
            }
        }
    }
    let scale = 8.0 * extent;

    'directions: for direction in RAY_DIRECTIONS {
        let far = [
            point[0] + direction[0] * scale,
            point[1] + direction[1] * scale,
            point[2] + direction[2] * scale,
        ];
        let mut crossings = 0_u64;
        for triangle in triangles {
            match segment_crosses_triangle(point, far, *triangle) {
                Crossing::Miss => {}
                Crossing::Hit => crossings += 1,
                Crossing::Degenerate => continue 'directions,
            }
        }
        return if crossings % 2 == 1 {
            Parity::Inside
        } else {
            Parity::Outside
        };
    }
    Parity::Exhausted
}

enum Crossing {
    Miss,
    Hit,
    Degenerate,
}

/// Exact segment/triangle crossing via five orient3d signs. Any zero sign
/// (segment endpoint on the triangle plane, or the segment threading an
/// edge/vertex) is degenerate.
fn segment_crosses_triangle(p: [f64; 3], q: [f64; 3], tri: [[f64; 3]; 3]) -> Crossing {
    let [a, b, c] = tri;
    let side_p = orient3d(a, b, c, p);
    let side_q = orient3d(a, b, c, q);
    if side_p == Orientation3d::Coplanar || side_q == Orientation3d::Coplanar {
        // Degenerate only if the triangle could actually intersect the
        // segment's line; a flat (zero-area) triangle reports coplanar for
        // everything and contributes nothing.
        if triangle_is_flat(tri) {
            return Crossing::Miss;
        }
        return Crossing::Degenerate;
    }
    if side_p == side_q {
        return Crossing::Miss;
    }

    let s1 = orient3d(p, q, a, b);
    let s2 = orient3d(p, q, b, c);
    let s3 = orient3d(p, q, c, a);
    if s1 == Orientation3d::Coplanar
        || s2 == Orientation3d::Coplanar
        || s3 == Orientation3d::Coplanar
    {
        return Crossing::Degenerate;
    }
    if s1 == s2 && s2 == s3 {
        Crossing::Hit
    } else {
        Crossing::Miss
    }
}

/// Exact zero-area test: three exactly-collinear (or coincident) corners.
fn triangle_is_flat(tri: [[f64; 3]; 3]) -> bool {
    let [a, b, c] = tri;
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    cross == [0.0, 0.0, 0.0]
}

#[cfg(test)]
mod tests {
    use super::{Parity, point_in_triangles};

    /// Unit tetrahedron triangles, outward wound.
    fn tetrahedron() -> Vec<[[f64; 3]; 3]> {
        let o = [0.0, 0.0, 0.0];
        let x = [1.0, 0.0, 0.0];
        let y = [0.0, 1.0, 0.0];
        let z = [0.0, 0.0, 1.0];
        vec![[o, y, x], [o, x, z], [o, z, y], [x, y, z]]
    }

    #[test]
    fn tetrahedron_parity_is_correct() {
        let triangles = tetrahedron();
        assert_eq!(
            point_in_triangles([0.1, 0.1, 0.1], &triangles),
            Parity::Inside
        );
        assert_eq!(
            point_in_triangles([0.9, 0.9, 0.9], &triangles),
            Parity::Outside
        );
        assert_eq!(
            point_in_triangles([-0.05, 0.2, 0.2], &triangles),
            Parity::Outside
        );
    }
}
