// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact-sign insphere evaluation.

use core::cell::Cell;
use core::cmp::Ordering;

use super::dyadic::Accumulator;

/// Position of a point relative to an oriented circumsphere.
///
/// For `a`, `b`, `c`, `d` with `d` [below](super::Orientation3d::Below) the
/// plane of `a`, `b`, `c` — Shewchuk's positive orientation —
/// [`Inside`](Self::Inside) means `e` lies inside their circumsphere.
/// Exchanging any two of the four reverses inside and outside. When the four
/// are coplanar, the result is only the exact sign of the algebraic
/// determinant; no geometric sphere exists.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum InSphere {
    /// Positive determinant: inside for a positively oriented tetrahedron.
    Inside,
    /// Negative determinant: outside for a positively oriented tetrahedron.
    Outside,
    /// Exactly zero determinant.
    Cospherical,
}

/// Arithmetic path used to evaluate an [`insphere()`] query.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InspherePath {
    /// The ordinary floating-point error-bound filter proved the sign.
    Filter,
    /// A fixed-size exact dyadic accumulator proved the degree-five sign.
    Dyadic,
    /// The query contained a non-finite coordinate, so no sign was evaluated.
    ///
    /// [`InsphereEvaluation::position`] is the deterministic
    /// [`InSphere::Cospherical`] sentinel for this path, not an exact
    /// geometric classification.
    NonFiniteInput,
}

/// Result and evaluated path for one [`insphere_evaluated`] query.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct InsphereEvaluation {
    /// Exact determinant sign for a finite query.
    ///
    /// This is the deterministic [`InSphere::Cospherical`] sentinel when
    /// [`path`](Self::path) is [`InspherePath::NonFiniteInput`].
    pub position: InSphere,
    /// Arithmetic path that established the sign, or an explicit invalid-input
    /// status when no sign was evaluated.
    pub path: InspherePath,
}

/// Half machine epsilon: the unit roundoff `2^-53`.
const U: f64 = f64::EPSILON / 2.0;
/// Shewchuk's error bound for the insphere floating-point filter.
const INSPHERE_ERRBOUND_A: f64 = (16.0 + 224.0 * U) * U;
/// The same bound with half again as much slack for the weighted lift's
/// extra subtraction.
const WEIGHTED_ERRBOUND_A: f64 = (24.0 + 336.0 * U) * U;
/// Enough bits for the complete finite-f64 insphere determinant.
///
/// A five-significand product occupies at most 265 bits. Its largest base
/// exponent is 4855, placing its highest bit at 5120. Relative to the minimum
/// exponent -5370 that is bit 10490; summing the at most 480 monomials of the
/// weighted test can carry through bit 10499, while 165 limbs store through
/// bit 10559.
const DYADIC_LIMBS: usize = 165;

/// Returns the exact position of `e` relative to the oriented circumsphere
/// through `a`, `b`, `c` and `d`.
///
/// For a positively oriented tetrahedron (see [`InSphere`]), the variants
/// have their ordinary geometric meaning. The sign is exact for all finite
/// inputs.
#[must_use]
#[inline]
pub fn insphere(a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3], e: [f64; 3]) -> InSphere {
    insphere_evaluated(a, b, c, d, e).position
}

/// Exact insphere sign plus the arithmetic path that established it.
///
/// This diagnostic form performs no global bookkeeping. Non-finite inputs use
/// a deterministic [`InSphere::Cospherical`] sentinel and report
/// [`InspherePath::NonFiniteInput`]; no geometric classification is claimed.
#[must_use]
#[inline]
pub fn insphere_evaluated(
    a: [f64; 3],
    b: [f64; 3],
    c: [f64; 3],
    d: [f64; 3],
    e: [f64; 3],
) -> InsphereEvaluation {
    let points = [a, b, c, d, e];
    if points
        .iter()
        .flatten()
        .any(|coordinate| !coordinate.is_finite())
    {
        return InsphereEvaluation {
            position: InSphere::Cospherical,
            path: InspherePath::NonFiniteInput,
        };
    }
    if let Some(position) = insphere_filter(points, [0.0; 5], INSPHERE_ERRBOUND_A) {
        return InsphereEvaluation {
            position,
            path: InspherePath::Filter,
        };
    }
    InsphereEvaluation {
        position: sign_of_dyadic_lifted(points, [0.0; 5]),
        path: InspherePath::Dyadic,
    }
}

/// Returns the exact position of the weighted point `e` relative to the
/// orthosphere of the weighted points `a`, `b`, `c` and `d`: the power test of
/// regular (weighted Delaunay) tetrahedralizations.
///
/// Each point is `[x, y, z, weight]`, the weight being a squared radius. For a
/// positively oriented tetrahedron (see [`InSphere`]), [`InSphere::Inside`]
/// means `e` has negative power distance to the sphere orthogonal to all four,
/// so `e` conflicts with the tetrahedron. A heavier `e` conflicts sooner;
/// heavier corners shrink the orthosphere, which stays orthogonal to their
/// larger balls. With every weight zero this is
/// [`insphere()`]. The sign is exact for all finite inputs.
#[must_use]
#[inline]
pub fn insphere_weighted(
    a: [f64; 4],
    b: [f64; 4],
    c: [f64; 4],
    d: [f64; 4],
    e: [f64; 4],
) -> InSphere {
    insphere_weighted_evaluated(a, b, c, d, e).position
}

/// Exact power-test sign plus the arithmetic path that established it.
///
/// Non-finite inputs use the [`InSphere::Cospherical`] sentinel and report
/// [`InspherePath::NonFiniteInput`].
#[must_use]
pub fn insphere_weighted_evaluated(
    a: [f64; 4],
    b: [f64; 4],
    c: [f64; 4],
    d: [f64; 4],
    e: [f64; 4],
) -> InsphereEvaluation {
    let weighted = [a, b, c, d, e];
    if weighted.iter().flatten().any(|value| !value.is_finite()) {
        return InsphereEvaluation {
            position: InSphere::Cospherical,
            path: InspherePath::NonFiniteInput,
        };
    }
    let points = weighted.map(|[x, y, z, _]| [x, y, z]);
    let weights = weighted.map(|[_, _, _, w]| w);
    if let Some(position) = insphere_filter(points, weights, WEIGHTED_ERRBOUND_A) {
        return InsphereEvaluation {
            position,
            path: InspherePath::Filter,
        };
    }
    InsphereEvaluation {
        position: sign_of_dyadic_lifted(points, weights),
        path: InspherePath::Dyadic,
    }
}

/// Shewchuk's floating-point insphere with its static error bound, each lift
/// reduced by its point's weight relative to `e`'s.
///
/// The bound assumes no product underflows, so any product that lost bits to
/// underflow, and any non-finite or subnormal result, defers to the exact
/// path instead. Weights enter the permanent by magnitude, so the same
/// derivation bounds the weighted lift's one extra rounding.
#[inline]
fn insphere_filter(points: [[f64; 3]; 5], weights: [f64; 5], bound: f64) -> Option<InSphere> {
    let [a, b, c, d, e] = points;
    let exact = Cell::new(true);
    let mul = |left: f64, right: f64| {
        let product = left * right;
        if !(left == 0.0 || right == 0.0 || product.is_normal()) {
            exact.set(false);
        }
        product
    };

    let [aex, aey, aez] = [a[0] - e[0], a[1] - e[1], a[2] - e[2]];
    let [bex, bey, bez] = [b[0] - e[0], b[1] - e[1], b[2] - e[2]];
    let [cex, cey, cez] = [c[0] - e[0], c[1] - e[1], c[2] - e[2]];
    let [dex, dey, dez] = [d[0] - e[0], d[1] - e[1], d[2] - e[2]];

    let aexbey = mul(aex, bey);
    let bexaey = mul(bex, aey);
    let bexcey = mul(bex, cey);
    let cexbey = mul(cex, bey);
    let cexdey = mul(cex, dey);
    let dexcey = mul(dex, cey);
    let dexaey = mul(dex, aey);
    let aexdey = mul(aex, dey);
    let aexcey = mul(aex, cey);
    let cexaey = mul(cex, aey);
    let bexdey = mul(bex, dey);
    let dexbey = mul(dex, bey);
    let ab = aexbey - bexaey;
    let bc = bexcey - cexbey;
    let cd = cexdey - dexcey;
    let da = dexaey - aexdey;
    let ac = aexcey - cexaey;
    let bd = bexdey - dexbey;

    let abc = mul(aez, bc) - mul(bez, ac) + mul(cez, ab);
    let bcd = mul(bez, cd) - mul(cez, bd) + mul(dez, bc);
    let cda = mul(cez, da) + mul(dez, ac) + mul(aez, cd);
    let dab = mul(dez, ab) + mul(aez, bd) + mul(bez, da);

    let [asq, bsq, csq, dsq] = [
        mul(aex, aex) + mul(aey, aey) + mul(aez, aez),
        mul(bex, bex) + mul(bey, bey) + mul(bez, bez),
        mul(cex, cex) + mul(cey, cey) + mul(cez, cez),
        mul(dex, dex) + mul(dey, dey) + mul(dez, dez),
    ];
    let shift = [0, 1, 2, 3].map(|i| weights[i] - weights[4]);
    let [alift, blift, clift, dlift] = [
        asq - shift[0],
        bsq - shift[1],
        csq - shift[2],
        dsq - shift[3],
    ];
    let [aplus, bplus, cplus, dplus] = [
        asq + shift[0].abs(),
        bsq + shift[1].abs(),
        csq + shift[2].abs(),
        dsq + shift[3].abs(),
    ];

    let det = (mul(dlift, abc) - mul(clift, dab)) + (mul(blift, cda) - mul(alift, bcd));

    let [aez, bez, cez, dez] = [aez.abs(), bez.abs(), cez.abs(), dez.abs()];
    let [aexbey, bexaey, bexcey, cexbey] = [aexbey.abs(), bexaey.abs(), bexcey.abs(), cexbey.abs()];
    let [cexdey, dexcey, dexaey, aexdey] = [cexdey.abs(), dexcey.abs(), dexaey.abs(), aexdey.abs()];
    let [aexcey, cexaey, bexdey, dexbey] = [aexcey.abs(), cexaey.abs(), bexdey.abs(), dexbey.abs()];
    let permanent = mul(
        mul(cexdey + dexcey, bez) + mul(dexbey + bexdey, cez) + mul(bexcey + cexbey, dez),
        aplus,
    ) + mul(
        mul(dexaey + aexdey, cez) + mul(aexcey + cexaey, dez) + mul(cexdey + dexcey, aez),
        bplus,
    ) + mul(
        mul(aexbey + bexaey, dez) + mul(bexdey + dexbey, aez) + mul(dexaey + aexdey, bez),
        cplus,
    ) + mul(
        mul(bexcey + cexbey, aez) + mul(cexaey + aexcey, bez) + mul(aexbey + bexaey, cez),
        dplus,
    );

    if !exact.get() || !det.is_finite() || !permanent.is_finite() {
        return None;
    }
    let errbound = bound * permanent;
    if errbound.is_normal() && (det > errbound || -det > errbound) {
        return Some(sign_of_det(det));
    }
    None
}

/// Even and odd permutations of three columns, for 3×3 determinants.
const PERMUTATIONS: [([usize; 3], bool); 6] = [
    ([0, 1, 2], true),
    ([1, 2, 0], true),
    ([2, 0, 1], true),
    ([0, 2, 1], false),
    ([1, 0, 2], false),
    ([2, 1, 0], false),
];

fn sign_of_dyadic_lifted(points: [[f64; 3]; 5], weights: [f64; 5]) -> InSphere {
    let mut sum = Accumulator::<5, DYADIC_LIMBS>::new();

    // The 5×5 determinant of rows (x, y, z, x² + y² + z² - w, 1), which
    // equals the filters' determinants of differences. Expanding along the
    // lift column, row i contributes (-1)^(i+3) lift(i) times the 4×4 minor
    // of the other rows over (x, y, z, 1); expanding that minor along its
    // ones column gives four signed 3×3 determinants of six products each.
    // The lift's three squares make 360 exact five-factor monomials over the
    // original coordinates, so no difference is ever rounded; a weight adds
    // 24 more per point, padded with an exact factor of one.
    let mut accumulated = true;
    for (i, lifted) in points.iter().enumerate() {
        let lift_positive = (i + 3) % 2 == 0;
        let mut rows = [[0.0; 3]; 4];
        for (row, point) in rows
            .iter_mut()
            .zip(points.iter().enumerate().filter(|&(k, _)| k != i))
        {
            *row = *point.1;
        }
        for j in 0..4 {
            let minor_positive = (j + 3) % 2 == 0;
            let mut minor = [[0.0; 3]; 3];
            for (row, point) in minor
                .iter_mut()
                .zip(rows.iter().enumerate().filter(|&(k, _)| k != j))
            {
                *row = *point.1;
            }
            let [p, q, r] = minor;
            for (columns, even) in PERMUTATIONS {
                let positive = (lift_positive == minor_positive) == even;
                for coordinate in lifted {
                    accumulated &= sum.add(
                        [
                            *coordinate,
                            *coordinate,
                            p[columns[0]],
                            q[columns[1]],
                            r[columns[2]],
                        ],
                        positive,
                    );
                }
                accumulated &= sum.add(
                    [weights[i], 1.0, p[columns[0]], q[columns[1]], r[columns[2]]],
                    !positive,
                );
            }
        }
    }
    debug_assert!(accumulated, "reviewed insphere accumulator bound");

    match sum.sign() {
        Ordering::Greater => InSphere::Inside,
        Ordering::Less => InSphere::Outside,
        Ordering::Equal => InSphere::Cospherical,
    }
}

#[inline]
fn sign_of_det(det: f64) -> InSphere {
    if det > 0.0 {
        InSphere::Inside
    } else if det < 0.0 {
        InSphere::Outside
    } else {
        InSphere::Cospherical
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::predicates::{Orientation3d, orient3d};

    fn power_of_two(exponent: i32) -> f64 {
        debug_assert!((-1022..=1023).contains(&exponent));
        f64::from_bits(((exponent + 1023) as u64) << 52)
    }

    /// A positively oriented corner tetrahedron of edge `scale`, whose
    /// circumsphere also passes through `[scale; 3]`.
    fn corner(scale: f64) -> [[f64; 3]; 4] {
        let tetrahedron = [
            [0.0, 0.0, 0.0],
            [0.0, scale, 0.0],
            [scale, 0.0, 0.0],
            [0.0, 0.0, scale],
        ];
        assert_eq!(
            orient3d(
                tetrahedron[0],
                tetrahedron[1],
                tetrahedron[2],
                tetrahedron[3]
            ),
            Orientation3d::Below
        );
        tetrahedron
    }

    #[test]
    fn classifies_clear_and_oriented_cases() {
        // Clear inside, outside, tie, and orientation-reversal cases should
        // stay on the inexpensive filter whenever the sign is well separated.
        let [a, b, c, d] = corner(1.0);
        assert_eq!(insphere(a, b, c, d, [0.25; 3]), InSphere::Inside);
        assert_eq!(insphere(a, b, c, d, [2.0; 3]), InSphere::Outside);
        assert_eq!(insphere(a, b, c, d, [1.0; 3]), InSphere::Cospherical);
        assert_eq!(insphere(b, a, c, d, [0.25; 3]), InSphere::Outside);
        assert_eq!(
            insphere_evaluated(a, b, c, d, [0.25; 3]).path,
            InspherePath::Filter
        );
    }

    #[test]
    fn resolves_one_ulp_around_an_exact_sphere() {
        // Adjacent binary64 values around a cospherical query need the exact
        // fallback to tell a one-ULP displacement apart.
        let [a, b, c, d] = corner(1.0);
        let one_below = f64::from_bits(1.0_f64.to_bits() - 1);
        let one_above = f64::from_bits(1.0_f64.to_bits() + 1);

        assert_eq!(
            insphere(a, b, c, d, [1.0, 1.0, one_below]),
            InSphere::Inside
        );
        assert_eq!(insphere(a, b, c, d, [1.0; 3]), InSphere::Cospherical);
        assert_eq!(
            insphere(a, b, c, d, [1.0, 1.0, one_above]),
            InSphere::Outside
        );
        assert_eq!(
            insphere_evaluated(a, b, c, d, [1.0; 3]).path,
            InspherePath::Dyadic
        );
    }

    #[test]
    fn exact_path_handles_scaled_and_extreme_inputs() {
        // Ties and their neighbors keep their sign at tiny and huge scales,
        // including scales whose degree-five determinant overflows binary64.
        for scale in [
            power_of_two(-200),
            1.0,
            power_of_two(200),
            1e100,
            f64::MAX / 4.0,
        ] {
            let [a, b, c, d] = corner(scale);
            let below = f64::from_bits(scale.to_bits() - 1);
            let above = f64::from_bits(scale.to_bits() + 1);
            assert_eq!(insphere(a, b, c, d, [scale; 3]), InSphere::Cospherical);
            assert_eq!(
                insphere(a, b, c, d, [scale, scale, below]),
                InSphere::Inside
            );
            assert_eq!(
                insphere(a, b, c, d, [scale, scale, above]),
                InSphere::Outside
            );
        }
        let tiny = f64::from_bits(1);
        let [a, b, c, d] = corner(tiny);
        assert_eq!(insphere(a, b, c, d, [tiny; 3]), InSphere::Cospherical);
        assert_eq!(insphere(a, b, c, d, [0.0; 3]), InSphere::Cospherical);
        assert_eq!(
            insphere(a, b, c, d, [tiny, tiny, 0.0]),
            InSphere::Cospherical
        );
    }

    #[test]
    fn degenerate_and_nonfinite_contracts_are_explicit() {
        // Coincident finite input has an exact zero determinant, while
        // non-finite input takes the documented diagnostic sentinel path.
        let point = [2.0, -3.0, 5.0];
        assert_eq!(
            insphere(point, point, point, point, [4.0, 5.0, 6.0]),
            InSphere::Cospherical
        );
        let [a, b, c, d] = corner(1.0);
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let evaluated = insphere_evaluated(a, b, c, d, [invalid, 0.0, 0.0]);
            assert_eq!(evaluated.position, InSphere::Cospherical);
            assert_eq!(evaluated.path, InspherePath::NonFiniteInput);
        }
    }

    #[test]
    fn weights_move_the_orthosphere() {
        // A point on the circumsphere conflicts once it carries weight, and
        // stops conflicting once heavier corners shrink the orthosphere.
        let [a, b, c, d] = corner(1.0).map(|[x, y, z]| [x, y, z, 0.0]);
        assert_eq!(
            insphere_weighted(a, b, c, d, [1.0, 1.0, 1.0, 0.0]),
            InSphere::Cospherical
        );
        assert_eq!(
            insphere_weighted(a, b, c, d, [1.0, 1.0, 1.0, 0.25]),
            InSphere::Inside
        );
        assert_eq!(
            insphere_weighted(a, b, c, d, [1.0, 1.0, 1.0, -0.25]),
            InSphere::Outside
        );
        let heavy = [a, b, c, d].map(|[x, y, z, _]| [x, y, z, 0.5]);
        assert_eq!(
            insphere_weighted(heavy[0], heavy[1], heavy[2], heavy[3], [1.0, 1.0, 1.0, 0.0]),
            InSphere::Outside
        );
        // Clear cases stay on the floating-point filter.
        assert_eq!(
            insphere_weighted_evaluated(a, b, c, d, [0.25, 0.25, 0.25, 0.01]).path,
            InspherePath::Filter
        );
        // Equal weights everywhere cancel exactly.
        assert_eq!(
            insphere_weighted(heavy[0], heavy[1], heavy[2], heavy[3], [1.0, 1.0, 1.0, 0.5]),
            InSphere::Cospherical
        );
    }

    #[test]
    fn weighted_test_matches_an_independent_integer_oracle() {
        // Integer coordinates and weights admit an exact `i128` power test;
        // zero weights must agree with the unweighted predicate.
        let mut state = 0x1d6e_2a90_c3f4_8b57_u64;
        let mut next = |range: i32| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            i128::from((state >> 33) as i32 % range)
        };
        for round in 0..4_000 {
            let range = if round % 2 == 0 { 4 } else { 1 << 12 };
            let points: [[i128; 4]; 5] =
                core::array::from_fn(|_| [next(range), next(range), next(range), next(range * 4)]);
            let e = points[4];
            let rows = [0, 1, 2, 3].map(|i| {
                let [x, y, z] = [0, 1, 2].map(|k| points[i][k] - e[k]);
                [x, y, z, x * x + y * y + z * z - (points[i][3] - e[3])]
            });
            let det3 = |p: [i128; 4], q: [i128; 4], r: [i128; 4]| {
                p[0] * (q[1] * r[2] - q[2] * r[1]) - p[1] * (q[0] * r[2] - q[2] * r[0])
                    + p[2] * (q[0] * r[1] - q[1] * r[0])
            };
            let [a, b, c, d] = rows;
            let det = d[3] * det3(a, b, c) - c[3] * det3(a, b, d) + b[3] * det3(a, c, d)
                - a[3] * det3(b, c, d);
            let expected = match det.cmp(&0) {
                Ordering::Greater => InSphere::Inside,
                Ordering::Less => InSphere::Outside,
                Ordering::Equal => InSphere::Cospherical,
            };
            let [a, b, c, d, e] = points.map(|p| p.map(|v| v as f64));
            assert_eq!(insphere_weighted(a, b, c, d, e), expected);
            let unweighted = |p: [f64; 4]| [p[0], p[1], p[2], 0.0];
            let plain = |p: [f64; 4]| [p[0], p[1], p[2]];
            assert_eq!(
                insphere_weighted(
                    unweighted(a),
                    unweighted(b),
                    unweighted(c),
                    unweighted(d),
                    unweighted(e)
                ),
                insphere(plain(a), plain(b), plain(c), plain(d), plain(e))
            );
        }
    }

    #[test]
    fn matches_an_independent_integer_oracle() {
        // Small integer coordinates admit an exact `i128` determinant of
        // differences, checking both the public dispatcher and the dyadic
        // path. Points drawn from a small cube make exact ties common.
        fn exact(points: [[i128; 3]; 5]) -> InSphere {
            let e = points[4];
            let rows = [0, 1, 2, 3].map(|i| {
                let [x, y, z] = [0, 1, 2].map(|k| points[i][k] - e[k]);
                [x, y, z, x * x + y * y + z * z]
            });
            let det3 = |p: [i128; 4], q: [i128; 4], r: [i128; 4], cols: [usize; 3]| {
                let [i, j, k] = cols;
                p[i] * (q[j] * r[k] - q[k] * r[j]) - p[j] * (q[i] * r[k] - q[k] * r[i])
                    + p[k] * (q[i] * r[j] - q[j] * r[i])
            };
            let [a, b, c, d] = rows;
            let det = -a[3] * det3(b, c, d, [0, 1, 2]) + b[3] * det3(a, c, d, [0, 1, 2])
                - c[3] * det3(a, b, d, [0, 1, 2])
                + d[3] * det3(a, b, c, [0, 1, 2]);
            match det.cmp(&0) {
                Ordering::Greater => InSphere::Inside,
                Ordering::Less => InSphere::Outside,
                Ordering::Equal => InSphere::Cospherical,
            }
        }

        let mut state = 0x5be1_e8a3_3f0d_7c29_u64;
        for round in 0..4_000 {
            let range = if round % 2 == 0 { 4 } else { 1 << 15 };
            let mut points = [[0_i128; 3]; 5];
            for coordinate in points.iter_mut().flatten() {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                *coordinate = i128::from((state >> 33) as i32 % range);
            }
            let as_f64 = |point: [i128; 3]| point.map(|v| v as f64);
            let [a, b, c, d, e] = points.map(as_f64);
            let expected = exact(points);
            assert_eq!(insphere(a, b, c, d, e), expected);
            assert_eq!(sign_of_dyadic_lifted([a, b, c, d, e], [0.0; 5]), expected);
        }
    }
}
