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
}
