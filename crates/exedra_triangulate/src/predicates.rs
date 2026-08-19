// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact-sign orientation predicates.
//!
//! [`orient2d`] returns the exact sign of the 2×2 orientation determinant: a
//! fast floating-point filter answers the overwhelming majority of queries,
//! and borderline cases fall back to exact expansion arithmetic (Dekker/Knuth
//! error-free transformations, after Shewchuk). No epsilons, no
//! transcendentals, and the result is bit-identical on every platform.
//!
//! The exact fallback evaluates products of the *original* coordinates, so
//! inputs must stay within [`MAX_COORDINATE`] to make overflow impossible;
//! [`crate::PolygonInput::validate`] enforces that bound.

/// Largest coordinate magnitude the predicates accept.
///
/// With `|coordinate| <= 1e100`, every intermediate product is at most
/// `1e200`, far below f64 overflow, so the exact fallback cannot produce
/// infinities.
pub const MAX_COORDINATE: f64 = 1e100;

/// Orientation of an ordered point triple.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// Counter-clockwise turn (positive determinant).
    Ccw,
    /// Clockwise turn (negative determinant).
    Cw,
    /// Exactly collinear (zero determinant).
    Collinear,
}

/// Half machine epsilon: the unit roundoff `2^-53`.
const U: f64 = f64::EPSILON / 2.0;
/// Shewchuk's error bound for the orient2d floating-point filter.
const CCW_ERRBOUND_A: f64 = (3.0 + 16.0 * U) * U;
/// Veltkamp splitting constant `2^27 + 1`.
const SPLITTER: f64 = 134_217_729.0;

/// Knuth's exact two-term sum: returns `(a + b, roundoff)`.
#[inline]
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let bv = x - a;
    let av = x - bv;
    let br = b - bv;
    let ar = a - av;
    (x, ar + br)
}

/// Veltkamp split of `a` into high and low halves.
#[inline]
fn split(a: f64) -> (f64, f64) {
    let c = SPLITTER * a;
    let hi = c - (c - a);
    (hi, a - hi)
}

/// Dekker's exact product: returns `(a * b, roundoff)`.
#[inline]
fn two_product(a: f64, b: f64) -> (f64, f64) {
    let x = a * b;
    let (ahi, alo) = split(a);
    let (bhi, blo) = split(b);
    let err = x - ahi * bhi - alo * bhi - ahi * blo;
    (x, alo * blo - err)
}

/// Adds scalar `b` into the nonoverlapping expansion `e[..n]`, writing the
/// resulting expansion into `out` and returning its length.
///
/// After Shewchuk's `grow_expansion`; the result is again a nonoverlapping
/// expansion in increasing-magnitude order.
fn grow_expansion(e: &[f64], b: f64, out: &mut [f64]) -> usize {
    let mut q = b;
    let mut n = 0;
    for &term in e {
        let (sum, err) = two_sum(q, term);
        out[n] = err;
        n += 1;
        q = sum;
    }
    out[n] = q;
    n + 1
}

/// Exact sign of a sum of exact two-term products.
///
/// Each `(a, b)` pair contributes the exact value `a * b` (sign folded into
/// the operands by the caller). The products are accumulated into one exact
/// expansion; the sign of the expansion is the sign of its
/// largest-magnitude (last nonzero) component.
fn sign_of_product_sum(terms: &[(f64, f64)]) -> Orientation {
    // Each product contributes 2 components; growing by one scalar adds at
    // most one component. 6 products -> at most 12 components.
    let mut e = [0.0_f64; 12];
    let mut scratch = [0.0_f64; 12];
    let mut len = 0;
    for &(a, b) in terms {
        let (hi, lo) = two_product(a, b);
        len = grow_expansion(&e[..len], lo, &mut scratch);
        e[..len].copy_from_slice(&scratch[..len]);
        len = grow_expansion(&e[..len], hi, &mut scratch);
        e[..len].copy_from_slice(&scratch[..len]);
    }
    for &component in e[..len].iter().rev() {
        if component > 0.0 {
            return Orientation::Ccw;
        }
        if component < 0.0 {
            return Orientation::Cw;
        }
    }
    Orientation::Collinear
}

/// Exact orientation of the ordered triple `(a, b, c)`.
///
/// Returns [`Orientation::Ccw`] when `c` lies to the left of the directed
/// line `a -> b`, [`Orientation::Cw`] when it lies to the right, and
/// [`Orientation::Collinear`] when the three points are exactly collinear.
///
/// The sign is exact for all finite inputs within [`MAX_COORDINATE`].
#[must_use]
pub fn orient2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Orientation {
    let detleft = (a[0] - c[0]) * (b[1] - c[1]);
    let detright = (a[1] - c[1]) * (b[0] - c[0]);
    let det = detleft - detright;

    let detsum = if detleft > 0.0 {
        if detright <= 0.0 {
            return sign_of_det(det);
        }
        detleft + detright
    } else if detleft < 0.0 {
        if detright >= 0.0 {
            return sign_of_det(det);
        }
        -detleft - detright
    } else {
        return sign_of_det(det);
    };

    let errbound = CCW_ERRBOUND_A * detsum;
    if det >= errbound || -det >= errbound {
        return sign_of_det(det);
    }

    // Exact fallback over original coordinates:
    // det = ax*by - ax*cy - cx*by - ay*bx + ay*cx + cy*bx
    sign_of_product_sum(&[
        (a[0], b[1]),
        (a[0], -c[1]),
        (-c[0], b[1]),
        (-a[1], b[0]),
        (a[1], c[0]),
        (c[1], b[0]),
    ])
}

#[inline]
fn sign_of_det(det: f64) -> Orientation {
    if det > 0.0 {
        Orientation::Ccw
    } else if det < 0.0 {
        Orientation::Cw
    } else {
        Orientation::Collinear
    }
}

/// Orientation of a point relative to an oriented plane in 3D.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Orientation3d {
    /// `d` lies on the positive side of the plane through `(a, b, c)` —
    /// the side the counter-clockwise normal `cross(b - a, c - a)` points
    /// toward.
    Above,
    /// `d` lies on the negative side of the plane.
    Below,
    /// `d` lies exactly on the plane.
    Coplanar,
}

/// Shewchuk's error bound for the orient3d floating-point filter.
const O3D_ERRBOUND_A: f64 = (7.0 + 56.0 * U) * U;

/// Exact orientation of point `d` relative to the plane through
/// `(a, b, c)`.
///
/// Returns [`Orientation3d::Above`] when `d` lies on the side pointed to
/// by `cross(b - a, c - a)` (the counter-clockwise normal of the triangle
/// as seen from above), [`Orientation3d::Below`] on the opposite side,
/// and [`Orientation3d::Coplanar`] when the four points are exactly
/// coplanar.
///
/// A floating-point filter answers clear cases; borderline cases fall
/// back to exact expansion arithmetic over the original coordinates. The
/// sign is exact for all finite inputs within [`MAX_COORDINATE`], and the
/// result is bit-identical on every platform.
#[must_use]
pub fn orient3d(a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]) -> Orientation3d {
    let adx = a[0] - d[0];
    let ady = a[1] - d[1];
    let adz = a[2] - d[2];
    let bdx = b[0] - d[0];
    let bdy = b[1] - d[1];
    let bdz = b[2] - d[2];
    let cdx = c[0] - d[0];
    let cdy = c[1] - d[1];
    let cdz = c[2] - d[2];

    let bdxcdy = bdx * cdy;
    let cdxbdy = cdx * bdy;
    let cdxady = cdx * ady;
    let adxcdy = adx * cdy;
    let adxbdy = adx * bdy;
    let bdxady = bdx * ady;

    let det = adz * (bdxcdy - cdxbdy) + bdz * (cdxady - adxcdy) + cdz * (adxbdy - bdxady);
    let permanent = (bdxcdy.abs() + cdxbdy.abs()) * adz.abs()
        + (cdxady.abs() + adxcdy.abs()) * bdz.abs()
        + (adxbdy.abs() + bdxady.abs()) * cdz.abs();
    let errbound = O3D_ERRBOUND_A * permanent;
    if det > errbound {
        return Orientation3d::Below;
    }
    if -det > errbound {
        return Orientation3d::Above;
    }
    orient3d_exact(a, b, c, d)
}

/// Exact fallback: the sign of the homogeneous 4x4 determinant as a sum
/// of 24 exact triple products of original coordinates.
fn orient3d_exact(a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]) -> Orientation3d {
    // det3(rows a-d, b-d, c-d) == det4 of homogeneous rows (a b c d | 1),
    // expanded along the ones column: -M(b,c,d) + M(a,c,d) - M(a,b,d)
    // + M(a,b,c), each minor a 3x3 determinant contributing six triple
    // products. 24 triple products, each exact as a 4-component expansion;
    // the running sum stays a nonoverlapping expansion via grow_expansion.
    let mut e = [0.0_f64; 100];
    let mut scratch = [0.0_f64; 100];
    let mut len = 0_usize;

    let mut acc = |x: f64, y: f64, z: f64, e: &mut [f64; 100], len: &mut usize| {
        let (hi, lo) = two_product(x, y);
        let (hh, hl) = two_product(hi, z);
        let (lh, ll) = two_product(lo, z);
        for value in [ll, lh, hl, hh] {
            if *len + 1 >= scratch.len() {
                // Unreachable by construction (bounded at 96 components);
                // stay safe rather than indexing out of range.
                break;
            }
            let n = grow_expansion(&e[..*len], value, &mut scratch);
            e[..n].copy_from_slice(&scratch[..n]);
            *len = n;
        }
    };

    let mut det3 =
        |p: [f64; 3], q: [f64; 3], r: [f64; 3], sign: f64, e: &mut [f64; 100], len: &mut usize| {
            // px(qy rz - qz ry) - py(qx rz - qz rx) + pz(qx ry - qy rx)
            acc(sign * p[0], q[1], r[2], e, len);
            acc(-sign * p[0], q[2], r[1], e, len);
            acc(-sign * p[1], q[0], r[2], e, len);
            acc(sign * p[1], q[2], r[0], e, len);
            acc(sign * p[2], q[0], r[1], e, len);
            acc(-sign * p[2], q[1], r[0], e, len);
        };

    det3(b, c, d, -1.0, &mut e, &mut len);
    det3(a, c, d, 1.0, &mut e, &mut len);
    det3(a, b, d, -1.0, &mut e, &mut len);
    det3(a, b, c, 1.0, &mut e, &mut len);

    for &component in e[..len].iter().rev() {
        if component > 0.0 {
            return Orientation3d::Below;
        }
        if component < 0.0 {
            return Orientation3d::Above;
        }
    }
    Orientation3d::Coplanar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_turns() {
        assert_eq!(
            orient2d([0.0, 0.0], [1.0, 0.0], [0.0, 1.0]),
            Orientation::Ccw
        );
        assert_eq!(
            orient2d([0.0, 0.0], [0.0, 1.0], [1.0, 0.0]),
            Orientation::Cw
        );
        assert_eq!(
            orient2d([0.0, 0.0], [1.0, 1.0], [2.0, 2.0]),
            Orientation::Collinear
        );
    }

    #[test]
    fn near_collinear_is_exact() {
        // Classic filter-breaking configuration: points nearly on the line
        // y = x, offset by one ulp. The naive determinant rounds to zero or
        // the wrong sign for many of these; the exact path must resolve them.
        let base = 0.5;
        let ulp = f64::EPSILON * 0.5;
        for i in 1..=64_u32 {
            let eps = ulp * f64::from(i);
            let a = [base, base + eps];
            let b = [12.0, 12.0];
            let c = [24.0, 24.0];
            // `a` is strictly above the line through b and c (slope 1), so
            // (a, b, c) makes a clockwise turn... verify against exact
            // rational reasoning: det = (ax-cx)(by-cy)-(ay-cy)(bx-cx)
            //   = (base-24)(-12) - (base+eps-24)(-12) = 12*eps > 0 -> CCW.
            assert_eq!(orient2d(a, b, c), Orientation::Ccw, "i = {i}");
        }
    }

    #[test]
    fn exact_zero_on_shifted_line() {
        // Points exactly on a line with irrational-looking f64 coefficients:
        // collinear by construction (b is the midpoint of a and c in exact
        // f64 arithmetic when coordinates are powers of two apart).
        let a = [3.5, 7.25];
        let c = [11.5, 23.25];
        let b = [7.5, 15.25];
        assert_eq!(orient2d(a, b, c), Orientation::Collinear);
    }

    #[test]
    fn tiny_perturbation_flips_sign() {
        let a = [3.5, 7.25];
        let c = [11.5, 23.25];
        // One ulp at 15.25 (binade [8, 16): ulp = 8 * EPSILON), so the
        // perturbed coordinate is exactly representable and distinct.
        let up = [7.5, 15.25 + f64::EPSILON * 8.0];
        let down = [7.5, 15.25 - f64::EPSILON * 8.0];
        assert_eq!(orient2d(a, up, c), Orientation::Cw);
        assert_eq!(orient2d(a, down, c), Orientation::Ccw);
    }

    #[test]
    fn two_product_is_exact() {
        let (hi, lo) = two_product(1.0 + f64::EPSILON, 1.0 + f64::EPSILON);
        // (1+e)^2 = 1 + 2e + e^2; the head cannot hold e^2.
        assert_eq!(hi, 1.0 + 2.0 * f64::EPSILON);
        assert_eq!(lo, f64::EPSILON * f64::EPSILON);
    }

    #[test]
    fn orient3d_clear_cases() {
        let a = [0.0, 0.0, 0.0];
        let b = [1.0, 0.0, 0.0];
        let c = [0.0, 1.0, 0.0];
        // CCW normal of (a, b, c) is +z; above/below follow it.
        assert_eq!(orient3d(a, b, c, [0.3, 0.3, 1.0]), Orientation3d::Above);
        assert_eq!(orient3d(a, b, c, [0.3, 0.3, -1.0]), Orientation3d::Below);
        assert_eq!(orient3d(a, b, c, [5.0, -3.0, 0.0]), Orientation3d::Coplanar);
    }

    #[test]
    fn orient3d_exact_zero_and_ulp_flips() {
        // Lattice points exactly on the plane z = y.
        let a = [0.0, 0.0, 0.0];
        let b = [4.0, 0.0, 0.0];
        let c = [0.0, 4.0, 4.0];
        let on = [1.0, 2.0, 2.0];
        assert_eq!(orient3d(a, b, c, on), Orientation3d::Coplanar);
        // One ulp at 2.0 (binade [2,4): ulp = 2 * EPSILON).
        let up = [1.0, 2.0, 2.0 + 2.0 * f64::EPSILON];
        let down = [1.0, 2.0, 2.0 - 2.0 * f64::EPSILON];
        let s_up = orient3d(a, b, c, up);
        let s_down = orient3d(a, b, c, down);
        assert_ne!(s_up, Orientation3d::Coplanar);
        assert_ne!(s_down, Orientation3d::Coplanar);
        assert_ne!(s_up, s_down, "one-ulp perturbations flip the exact sign");
    }

    #[test]
    fn orient3d_agrees_with_naive_determinant_when_clear() {
        // Deterministic pseudo-random corpus; assert agreement wherever the
        // naive determinant is decisively nonzero.
        let mut state = 0x1234_5678_9ABC_DEF0_u64;
        let mut next = move || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let mut coord = move || ((next() >> 40) as f64) / 1e4 - 800.0;
        for _ in 0..500 {
            let p = [
                [coord(), coord(), coord()],
                [coord(), coord(), coord()],
                [coord(), coord(), coord()],
                [coord(), coord(), coord()],
            ];
            let (a, b, c, d) = (p[0], p[1], p[2], p[3]);
            let adx = a[0] - d[0];
            let ady = a[1] - d[1];
            let adz = a[2] - d[2];
            let bdx = b[0] - d[0];
            let bdy = b[1] - d[1];
            let bdz = b[2] - d[2];
            let cdx = c[0] - d[0];
            let cdy = c[1] - d[1];
            let cdz = c[2] - d[2];
            let det = adz * (bdx * cdy - cdx * bdy)
                + bdz * (cdx * ady - adx * cdy)
                + cdz * (adx * bdy - bdx * ady);
            if det.abs() < 1.0 {
                continue;
            }
            let expected = if det > 0.0 {
                Orientation3d::Below
            } else {
                Orientation3d::Above
            };
            assert_eq!(orient3d(a, b, c, d), expected);
        }
    }
}
