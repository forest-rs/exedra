// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Geometric bounds for converting bilinear patches to triangles.

use crate::{cross, dot, norm, normalize, sub};

/// Bounds between a bilinear patch and either diagonal triangulation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BilinearTriangleBounds {
    /// Parameter-preserving interpolation error, one quarter of the mixed
    /// corner difference's norm. Uniform U/V subdivisions divide it by their
    /// interval counts' product.
    pub mixed_deviation: f64,
    /// Geometric bound from the projected patch's planar slab. Infinite when
    /// its projection cannot establish a convex quadrilateral. A convex planar
    /// patch has zero geometric error even if its parameterization is nonaffine.
    /// This bound remains valid when subdividing the patch along its rulings.
    pub planar_deviation: f64,
    /// f64 arithmetic allowance in the supplied coordinate system. Callers
    /// separately account for later narrowing or placement.
    pub roundoff: f64,
}

/// Bounds either diagonal triangulation of corners `[P00,P10,P11,P01]`.
///
/// A bilinear patch differs from either triangle interpolant by at most
/// `|P00-P10+P11-P01|/4` at matching parameters. When projection onto the plane
/// through corners 0, 1 and 3 is convex, both surfaces cover that quadrilateral;
/// the slab height therefore gives an independent geometric distance bound.
/// Use the smaller applicable bound, plus `roundoff`.
///
/// Returns `None` for non-finite inputs or an unrepresentable norm/headroom.
#[must_use]
pub fn bilinear_triangle_bounds(corners: [[f64; 3]; 4]) -> Option<BilinearTriangleBounds> {
    if corners.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let mixed = sub(sub(corners[2], corners[3]), sub(corners[1], corners[0]));
    let mixed_deviation = norm(mixed) * 0.25;
    let scale = corners
        .iter()
        .flatten()
        .map(|v| v.abs())
        .fold(0.0_f64, f64::max);
    let roundoff = scale * (128.0 * f64::EPSILON);
    if !mixed_deviation.is_finite() || !roundoff.is_finite() {
        return None;
    }
    let normal = normalize(cross(
        sub(corners[1], corners[0]),
        sub(corners[3], corners[0]),
    ));
    let planar_deviation = normal
        .filter(|&normal| {
            [[0, 1, 2], [0, 2, 3], [0, 1, 3], [1, 2, 3]]
                .into_iter()
                .all(|[a, b, c]| {
                    let (ab, ac) = (sub(corners[b], corners[a]), sub(corners[c], corners[a]));
                    let margin = norm(ab) * norm(ac) * (64.0 * f64::EPSILON);
                    dot(cross(ab, ac), normal) > margin
                })
        })
        .map_or(f64::INFINITY, |normal| {
            // Include every corner: a numerically computed normal need not
            // be exactly perpendicular to either defining edge.
            let (lo, hi) = corners
                .iter()
                .map(|&p| dot(sub(p, corners[0]), normal))
                .fold((0.0_f64, 0.0_f64), |(lo, hi), h| (lo.min(h), hi.max(h)));
            hi - lo
        });
    Some(BilinearTriangleBounds {
        mixed_deviation,
        planar_deviation,
        roundoff,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{add, lerp, scale};

    #[test]
    fn planar_trapezoids_and_twisted_quads_have_distinct_geometric_bounds() {
        let flat = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let bound = bilinear_triangle_bounds(flat).unwrap();
        assert_eq!(bound.mixed_deviation, 0.25);
        assert_eq!(bound.planar_deviation, 0.0);
        let p = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 2.0],
            [0.0, 1.0, 0.0],
        ];
        let bound = bilinear_triangle_bounds(p).unwrap();
        assert_eq!(bound.mixed_deviation, 0.5);
        for i in 0..=20 {
            for j in 0..=20 {
                let (u, v) = (f64::from(i) / 20.0, f64::from(j) / 20.0);
                let patch = lerp(lerp(p[0], p[1], u), lerp(p[3], p[2], u), v);
                let triangle = if v <= u {
                    add(
                        scale(p[0], 1.0 - u),
                        add(scale(p[1], u - v), scale(p[2], v)),
                    )
                } else {
                    add(
                        scale(p[0], 1.0 - v),
                        add(scale(p[2], u), scale(p[3], v - u)),
                    )
                };
                assert!(norm(sub(patch, triangle)) <= bound.mixed_deviation + bound.roundoff);
            }
        }
        assert!(bilinear_triangle_bounds([[f64::NAN; 3]; 4]).is_none());
    }
}
