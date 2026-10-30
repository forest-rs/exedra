// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact-sign geometric predicates.
//!
//! Each predicate returns the exact sign of a determinant over its `f64`
//! inputs, so geometric decisions agree with each other and are
//! bit-identical on every platform. No epsilons, no transcendentals.
//!
//! [`orient2d`] returns the exact sign of the 2×2 orientation determinant: a
//! fast floating-point filter answers the overwhelming majority of queries,
//! narrow exponent spans fall back to exact expansion arithmetic after a
//! lossless common power-of-two scaling, and wider spans use fixed-size exact
//! dyadic accumulation. [`orient3d`] filters the same way, then evaluates
//! exactly from the coordinate differences when subtracting lost nothing, and
//! otherwise falls back to the dyadic accumulator over the original
//! coordinates.
//!
//! [`incircle()`] uses the same shape: a fast error-bound filter followed by a
//! fixed-size exact dyadic accumulator. The exact path expands the homogeneous
//! degree-four determinant directly, avoiding rounded coordinate differences.
//! [`insphere()`] is its degree-five counterpart for circumspheres, summing 360
//! five-factor monomials when the filter cannot decide. [`plane_side`]
//! classifies a point against an authored plane equation through the same
//! accumulator, without normalizing its coefficients.
//!
//! The filters' error bounds and the expansion arithmetic follow Jonathan
//! Richard Shewchuk, *Adaptive Precision Floating-Point Arithmetic and Fast
//! Robust Geometric Predicates* (1997), and its public-domain `predicates.c`.
//!
//! Callers keep coordinates within [`MAX_COORDINATE`].
//! [`orient2d_evaluated`], [`incircle_evaluated`] and [`insphere_evaluated`]
//! expose which arithmetic path proved one query without introducing global
//! mutable counters.

mod dyadic;
mod incircle;
mod insphere;
mod plane;

use core::cmp::Ordering;

use dyadic::Accumulator;
pub use plane::plane_side;

pub use incircle::{InCircle, IncircleEvaluation, IncirclePath, incircle, incircle_evaluated};
pub use insphere::{InSphere, InsphereEvaluation, InspherePath, insphere, insphere_evaluated};

/// Largest coordinate magnitude the predicates accept.
///
/// The exponent-safe [`orient2d`] fallbacks preserve exact signs from the
/// smallest subnormal through this bound. Consumers that validate their input,
/// such as polygon triangulation, check against the same envelope.
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

/// Arithmetic path used to evaluate an [`orient2d`] query.
///
/// This diagnostic is local to one call. It does not use global counters or
/// otherwise change predicate determinism.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Orient2dPath {
    /// The ordinary floating-point error-bound filter proved the sign.
    Filter,
    /// A lossless common power-of-two scaling made expansion arithmetic safe.
    NormalizedExpansion,
    /// A fixed-size exact dyadic accumulator handled a wide exponent span.
    Dyadic,
    /// The query contained a non-finite coordinate, so no sign was evaluated.
    ///
    /// [`Orient2dEvaluation::orientation`] is the deterministic
    /// [`Orientation::Collinear`] sentinel for this path, not an exact
    /// geometric classification. Earlier out-of-domain behavior was
    /// unspecified and could return a different variant.
    NonFiniteInput,
}

/// Result and evaluated path for one [`orient2d_evaluated`] query.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Orient2dEvaluation {
    /// Exact orientation sign for a finite query within [`MAX_COORDINATE`].
    ///
    /// This is the deterministic [`Orientation::Collinear`] sentinel when
    /// [`path`](Self::path) is [`Orient2dPath::NonFiniteInput`].
    pub orientation: Orientation,
    /// Arithmetic path that established the sign, or an explicit invalid-input
    /// status when no sign was evaluated.
    pub path: Orient2dPath,
}

/// Half machine epsilon: the unit roundoff `2^-53`.
const U: f64 = f64::EPSILON / 2.0;
/// Shewchuk's error bound for the orient2d floating-point filter.
const CCW_ERRBOUND_A: f64 = (3.0 + 16.0 * U) * U;
/// Veltkamp splitting constant `2^27 + 1`.
const SPLITTER: f64 = 134_217_729.0;
/// Largest highest-bit exponent span accepted by the normalized expansion.
///
/// After normalizing the largest coordinate to exponent zero, this keeps the
/// smallest possible product bit at exponent `-1004` (`2 * (-450 - 52)`),
/// safely above the normal/subnormal boundary at `-1022`.
const NORMALIZED_EXPONENT_SPAN: i32 = 450;
/// Enough bits for a sum of six products across the complete finite f64
/// exponent domain, not merely the smaller public coordinate domain.
///
/// A largest finite significand product has highest bit 105. Its maximum
/// shift is 4090, placing that bit at 4195. Summing six products can carry
/// through bit 4198, while 66 limbs store through bit 4223.
const DYADIC_LIMBS: usize = 66;

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
#[inline]
pub fn orient2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Orientation {
    orient2d_evaluated(a, b, c).orientation
}

/// Exact orientation plus the arithmetic path that established its sign.
///
/// This is the diagnostic form of [`orient2d`]. It has identical sign
/// semantics and performs no global bookkeeping, so callers can use it for
/// local profiling without introducing hidden mutable state.
///
/// The sign is exact for all finite inputs within [`MAX_COORDINATE`].
/// Non-finite inputs use a deterministic [`Orientation::Collinear`] sentinel
/// and report
/// [`Orient2dPath::NonFiniteInput`]; no arithmetic path or geometric sign is
/// claimed for them. Their previous out-of-domain result was unspecified and
/// may differ.
#[must_use]
#[inline]
pub fn orient2d_evaluated(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Orient2dEvaluation {
    if [a[0], a[1], b[0], b[1], c[0], c[1]]
        .iter()
        .any(|coordinate| !coordinate.is_finite())
    {
        return Orient2dEvaluation {
            orientation: Orientation::Collinear,
            path: Orient2dPath::NonFiniteInput,
        };
    }
    if let Some(orientation) = orient2d_filter(a, b, c) {
        return Orient2dEvaluation {
            orientation,
            path: Orient2dPath::Filter,
        };
    }
    if let Some(orientation) = normalized_product_sum(a, b, c) {
        return Orient2dEvaluation {
            orientation,
            path: Orient2dPath::NormalizedExpansion,
        };
    }
    Orient2dEvaluation {
        orientation: sign_of_dyadic_product_sum(a, b, c).unwrap_or(Orientation::Collinear),
        path: Orient2dPath::Dyadic,
    }
}

#[inline]
fn orient2d_filter(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<Orientation> {
    let detleft = (a[0] - c[0]) * (b[1] - c[1]);
    let detright = (a[1] - c[1]) * (b[0] - c[0]);
    let det = detleft - detright;

    if !detleft.is_finite() || !detright.is_finite() || !det.is_finite() {
        return None;
    }

    let detsum = if detleft > 0.0 {
        if detright <= 0.0 {
            return (det != 0.0).then(|| sign_of_det(det));
        }
        detleft + detright
    } else if detleft < 0.0 {
        if detright >= 0.0 {
            return (det != 0.0).then(|| sign_of_det(det));
        }
        -detleft - detright
    } else {
        return (det != 0.0).then(|| sign_of_det(det));
    };

    let errbound = CCW_ERRBOUND_A * detsum;
    // The relative-error proof assumes normal intermediates. A subnormal
    // error bound can itself have rounded down, so defer that narrow case to
    // exponent-safe exact arithmetic.
    if errbound.is_normal() && (det > errbound || -det > errbound) {
        return Some(sign_of_det(det));
    }

    None
}

fn normalized_product_sum(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<Orientation> {
    let coordinates = [a[0], a[1], b[0], b[1], c[0], c[1]];
    let mut minimum = i32::MAX;
    let mut maximum = i32::MIN;
    for coordinate in coordinates {
        if !coordinate.is_finite() {
            return None;
        }
        if let Some(exponent) = highest_bit_exponent(coordinate) {
            minimum = minimum.min(exponent);
            maximum = maximum.max(exponent);
        }
    }
    if maximum == i32::MIN {
        return Some(Orientation::Collinear);
    }
    if maximum - minimum > NORMALIZED_EXPONENT_SPAN {
        return None;
    }

    let shift = -maximum;
    let scale = |point: [f64; 2]| point.map(|coordinate| scale_power_of_two(coordinate, shift));
    let [a, b, c] = [scale(a), scale(b), scale(c)];

    // Exact fallback over losslessly scaled original coordinates:
    // det = ax*by - ax*cy - cx*by - ay*bx + ay*cx + cy*bx
    Some(sign_of_product_sum(&[
        (a[0], b[1]),
        (a[0], -c[1]),
        (-c[0], b[1]),
        (-a[1], b[0]),
        (a[1], c[0]),
        (c[1], b[0]),
    ]))
}

#[inline]
fn highest_bit_exponent(value: f64) -> Option<i32> {
    let bits = value.to_bits() & 0x7fff_ffff_ffff_ffff;
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    let encoded_exponent = ((bits >> 52) & 0x7ff) as i32;
    if encoded_exponent == 0 {
        (fraction != 0).then(|| {
            let highest_fraction_bit = 63 - fraction.leading_zeros().cast_signed();
            -1074 + highest_fraction_bit
        })
    } else {
        Some(encoded_exponent - 1023)
    }
}

#[inline]
fn scale_power_of_two(value: f64, exponent: i32) -> f64 {
    if value == 0.0 {
        return value;
    }
    if exponent > 1023 {
        value * power_of_two(1023) * power_of_two(exponent - 1023)
    } else {
        value * power_of_two(exponent)
    }
}

#[inline]
fn power_of_two(exponent: i32) -> f64 {
    debug_assert!(
        (-1074..=1023).contains(&exponent),
        "binary64 power-of-two exponent must be representable"
    );
    if exponent >= -1022 {
        f64::from_bits(((exponent + 1023) as u64) << 52)
    } else {
        f64::from_bits(1_u64 << (exponent + 1074))
    }
}

fn sign_of_dyadic_product_sum(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<Orientation> {
    if [a[0], a[1], b[0], b[1], c[0], c[1]]
        .iter()
        .any(|coordinate| !coordinate.is_finite())
    {
        return None;
    }
    dyadic_product_sum(&[
        (a[0], b[1]),
        (a[0], -c[1]),
        (-c[0], b[1]),
        (-a[1], b[0]),
        (a[1], c[0]),
        (c[1], b[0]),
    ])
}

// At most six finite products fit the accumulator's documented carry budget.
fn dyadic_product_sum(terms: &[(f64, f64)]) -> Option<Orientation> {
    debug_assert!(
        terms.len() <= 6,
        "dyadic accumulator carry budget is six products"
    );
    let mut sum = Accumulator::<2, DYADIC_LIMBS>::new();
    for &(left, right) in terms {
        if !sum.add([left, right], true) {
            return None;
        }
    }
    Some(match sum.sign() {
        Ordering::Greater => Orientation::Ccw,
        Ordering::Less => Orientation::Cw,
        Ordering::Equal => Orientation::Collinear,
    })
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
/// A floating-point filter answers clear cases. Borderline cases whose
/// coordinate differences are exact, as for grid-aligned or coplanar input,
/// take Shewchuk's exact expansion of those differences; the rest fall back
/// to a fixed-size exact dyadic accumulator over the original coordinates. The
/// sign is exact for all finite inputs, including subnormal ones, and the
/// result is bit-identical on every platform. Non-finite input returns
/// [`Orientation3d::Coplanar`] as a deterministic sentinel.
#[must_use]
#[inline]
pub fn orient3d(a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]) -> Orientation3d {
    if [a, b, c, d]
        .iter()
        .flatten()
        .any(|coordinate| !coordinate.is_finite())
    {
        return Orientation3d::Coplanar;
    }
    orient3d_filter(a, b, c, d)
        .or_else(|| orient3d_exact_differences(a, b, c, d))
        .unwrap_or_else(|| orient3d_exact(a, b, c, d))
}

/// Shewchuk's floating-point orient3d with its static error bound.
///
/// The bound assumes no product underflows, so an underflowing product, or a
/// non-finite or subnormal result, defers to the exact path instead.
#[inline]
fn orient3d_filter(a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]) -> Option<Orientation3d> {
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

    let bc = bdxcdy - cdxbdy;
    let ca = cdxady - adxcdy;
    let ab = adxbdy - bdxady;
    let (adet, bdet, cdet) = (adz * bc, bdz * ca, cdz * ab);
    let normal =
        |left: f64, right: f64, product: f64| left == 0.0 || right == 0.0 || product.is_normal();
    if !(normal(bdx, cdy, bdxcdy)
        && normal(cdx, bdy, cdxbdy)
        && normal(cdx, ady, cdxady)
        && normal(adx, cdy, adxcdy)
        && normal(adx, bdy, adxbdy)
        && normal(bdx, ady, bdxady)
        && normal(adz, bc, adet)
        && normal(bdz, ca, bdet)
        && normal(cdz, ab, cdet))
    {
        return None;
    }
    let det = adet + bdet + cdet;
    let permanent = (bdxcdy.abs() + cdxbdy.abs()) * adz.abs()
        + (cdxady.abs() + adxcdy.abs()) * bdz.abs()
        + (adxbdy.abs() + bdxady.abs()) * cdz.abs();
    if !det.is_finite() || !permanent.is_finite() {
        return None;
    }
    let errbound = O3D_ERRBOUND_A * permanent;
    if !errbound.is_normal() {
        return None;
    }
    if det > errbound {
        Some(Orientation3d::Below)
    } else if -det > errbound {
        Some(Orientation3d::Above)
    } else {
        None
    }
}

/// Exact sign from the coordinate differences, when they are exact.
///
/// Shewchuk's adaptive middle stage: if subtracting `d` loses nothing, which
/// is the common case for grid-aligned and exactly coplanar input, the
/// determinant of the differences equals the true one and costs six exact
/// triple products. Differences of extreme magnitude, where a product could
/// underflow or overflow, defer to the dyadic accumulator.
fn orient3d_exact_differences(
    a: [f64; 3],
    b: [f64; 3],
    c: [f64; 3],
    d: [f64; 3],
) -> Option<Orientation3d> {
    // Keeps every triple product and its error terms normal.
    const SAFE: core::ops::RangeInclusive<f64> = 1.0e-75..=1.0e75;
    let mut rows = [[0.0; 3]; 3];
    for (row, p) in rows.iter_mut().zip([a, b, c]) {
        for k in 0..3 {
            let (difference, tail) = two_sum(p[k], -d[k]);
            if tail != 0.0 || (difference != 0.0 && !SAFE.contains(&difference.abs())) {
                return None;
            }
            row[k] = difference;
        }
    }
    let [p, q, r] = rows;
    let mut e = [0.0_f64; 25];
    let mut scratch = [0.0_f64; 25];
    let mut len = 0_usize;
    // p·(q × r) as six signed triple products, each a four-term expansion.
    for (x, y, z) in [
        (p[0], q[1], r[2]),
        (-p[0], q[2], r[1]),
        (-p[1], q[0], r[2]),
        (p[1], q[2], r[0]),
        (p[2], q[0], r[1]),
        (-p[2], q[1], r[0]),
    ] {
        let (hi, lo) = two_product(x, y);
        let (hh, hl) = two_product(hi, z);
        let (lh, ll) = two_product(lo, z);
        for value in [ll, lh, hl, hh] {
            let n = grow_expansion(&e[..len], value, &mut scratch);
            e[..n].copy_from_slice(&scratch[..n]);
            len = n;
        }
    }
    // The determinant of rows (a - d, b - d, c - d) is positive when `d`
    // lies below the plane of `a`, `b`, `c`.
    Some(match e[..len].iter().rev().find(|&&v| v != 0.0) {
        Some(&v) if v > 0.0 => Orientation3d::Below,
        Some(_) => Orientation3d::Above,
        None => Orientation3d::Coplanar,
    })
}

/// Enough bits for the complete finite-f64 orient3d determinant.
///
/// A three-significand product occupies at most 159 bits. Its largest base
/// exponent is 2913, placing its highest bit at 3072. Relative to the minimum
/// exponent -3222 that is bit 6294; summing 24 monomials can carry through
/// bit 6299, while 99 limbs store through bit 6335.
const ORIENT3D_LIMBS: usize = 99;

/// Exact fallback: the sign of the homogeneous 4×4 determinant as a sum
/// of 24 exact triple products of original coordinates.
fn orient3d_exact(a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]) -> Orientation3d {
    // det3(rows a-d, b-d, c-d) == det4 of homogeneous rows (a b c d | 1),
    // expanded along the ones column: -M(b,c,d) + M(a,c,d) - M(a,b,d)
    // + M(a,b,c), each minor a 3×3 determinant contributing six triple
    // products. Finite inputs always fit the reviewed accumulator bound.
    let mut sum = Accumulator::<3, ORIENT3D_LIMBS>::new();
    let mut det3 = |p: [f64; 3], q: [f64; 3], r: [f64; 3], positive: bool| {
        // px(qy rz - qz ry) - py(qx rz - qz rx) + pz(qx ry - qy rx)
        sum.add([p[0], q[1], r[2]], positive)
            && sum.add([p[0], q[2], r[1]], !positive)
            && sum.add([p[1], q[0], r[2]], !positive)
            && sum.add([p[1], q[2], r[0]], positive)
            && sum.add([p[2], q[0], r[1]], positive)
            && sum.add([p[2], q[1], r[0]], !positive)
    };
    let accumulated =
        det3(b, c, d, false) && det3(a, c, d, true) && det3(a, b, d, false) && det3(a, b, c, true);
    debug_assert!(accumulated, "reviewed orient3d accumulator bound");

    match sum.sign() {
        Ordering::Greater => Orientation3d::Below,
        Ordering::Less => Orientation3d::Above,
        Ordering::Equal => Orientation3d::Coplanar,
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
    fn uniformly_tiny_turn_does_not_underflow_to_collinear() {
        let evaluated = orient2d_evaluated([0.0, 0.0], [1e-300, 0.0], [0.0, 1e-300]);
        assert_eq!(evaluated.orientation, Orientation::Ccw);
        assert_eq!(evaluated.path, Orient2dPath::NormalizedExpansion);
    }

    #[test]
    fn smallest_subnormal_turn_is_exact_after_normalization() {
        let minimum = f64::from_bits(1);
        let evaluated = orient2d_evaluated([0.0, 0.0], [minimum, 0.0], [0.0, minimum]);
        assert_eq!(evaluated.orientation, Orientation::Ccw);
        assert_eq!(evaluated.path, Orient2dPath::NormalizedExpansion);
    }

    #[test]
    fn every_accepted_uniform_binary_exponent_agrees_between_exact_paths() {
        for exponent in -1074..=332 {
            let scale = power_of_two(exponent);
            let a = [0.0, 0.0];
            let b = [scale, 0.0];
            let c = [0.0, scale];
            assert!(scale <= MAX_COORDINATE, "exponent={exponent}");
            assert_eq!(
                normalized_product_sum(a, b, c),
                Some(Orientation::Ccw),
                "normalized exponent={exponent}"
            );
            assert_eq!(
                sign_of_dyadic_product_sum(a, b, c),
                Some(Orientation::Ccw),
                "dyadic exponent={exponent}"
            );
            assert_eq!(
                orient2d(a, b, c),
                Orientation::Ccw,
                "public exponent={exponent}"
            );
        }
    }

    #[test]
    fn exact_zero_is_preserved_across_uniform_binary_exponents() {
        for exponent in -1074..=330 {
            let scale = power_of_two(exponent);
            let a = [scale, scale];
            let b = [2.0 * scale, 2.0 * scale];
            let c = [3.0 * scale, 3.0 * scale];
            let evaluated = orient2d_evaluated(a, b, c);
            assert_eq!(
                evaluated.orientation,
                Orientation::Collinear,
                "exponent={exponent} path={:?}",
                evaluated.path
            );
            assert_eq!(
                sign_of_dyadic_product_sum(a, b, c),
                Some(Orientation::Collinear),
                "dyadic exponent={exponent}"
            );
        }
    }

    #[test]
    fn mixed_exponent_ulp_below_the_large_binade_uses_dyadic_path() {
        let minimum = f64::from_bits(1);
        for exponent in -622..=332 {
            let scale = power_of_two(exponent);
            let a = [minimum, 0.0];
            let b = [0.5 * scale, 0.5 * scale];
            let c = [scale, scale];
            assert!(scale <= MAX_COORDINATE, "exponent={exponent}");
            let evaluated = orient2d_evaluated(a, b, c);
            assert_eq!(
                evaluated,
                Orient2dEvaluation {
                    orientation: Orientation::Cw,
                    path: Orient2dPath::Dyadic,
                },
                "exponent={exponent}"
            );
            assert_eq!(
                orient2d(c, b, a),
                Orientation::Ccw,
                "reverse exponent={exponent}"
            );
            assert_eq!(
                orient2d(a, c, b),
                Orientation::Ccw,
                "swap exponent={exponent}"
            );
        }
    }

    #[test]
    fn diagnostic_paths_are_locally_observable() {
        assert_eq!(
            orient2d_evaluated([0.0, 0.0], [1.0, 0.0], [0.0, 1.0]).path,
            Orient2dPath::Filter
        );
        assert_eq!(
            orient2d_evaluated([0.0, 0.0], [1e-300, 0.0], [0.0, 1e-300]).path,
            Orient2dPath::NormalizedExpansion
        );
        assert_eq!(
            orient2d_evaluated([f64::from_bits(1), 0.0], [0.5, 0.5], [1.0, 1.0]).path,
            Orient2dPath::Dyadic
        );
    }

    #[test]
    fn normalization_span_boundary_routes_losslessly() {
        for (exponent, expected_path) in [
            (-NORMALIZED_EXPONENT_SPAN, Orient2dPath::NormalizedExpansion),
            (-NORMALIZED_EXPONENT_SPAN - 1, Orient2dPath::Dyadic),
        ] {
            let tiny = power_of_two(exponent);
            let evaluated = orient2d_evaluated([tiny, 0.0], [0.5, 0.5], [1.0, 1.0]);
            assert_eq!(
                evaluated.orientation,
                Orientation::Cw,
                "exponent={exponent}"
            );
            assert_eq!(evaluated.path, expected_path, "exponent={exponent}");
        }
    }

    #[test]
    fn wide_exponent_exact_tie_uses_dyadic_path() {
        let minimum = f64::from_bits(1);
        assert_eq!(
            orient2d_evaluated([minimum, minimum], [0.5, 0.5], [1.0, 1.0]),
            Orient2dEvaluation {
                orientation: Orientation::Collinear,
                path: Orient2dPath::Dyadic,
            }
        );
    }

    #[test]
    fn dyadic_accumulator_handles_extreme_limbs() {
        let minimum = f64::from_bits(1);
        let maximum = f64::MAX;
        let evaluated = orient2d_evaluated(
            [minimum, 0.0],
            [maximum * 0.5, maximum * 0.5],
            [maximum, maximum],
        );
        assert_eq!(evaluated.orientation, Orientation::Cw);
        assert_eq!(evaluated.path, Orient2dPath::Dyadic);
    }

    #[test]
    fn signed_zero_permutations_preserve_filter_result() {
        for mask in 0_u8..16 {
            let zero = |bit| {
                if mask & (1_u8 << bit) == 0_u8 {
                    0.0
                } else {
                    -0.0
                }
            };
            let evaluated = orient2d_evaluated([zero(0), zero(1)], [1.0, zero(2)], [zero(3), 1.0]);
            assert_eq!(evaluated.orientation, Orientation::Ccw, "mask={mask}");
            assert_eq!(evaluated.path, Orient2dPath::Filter, "mask={mask}");
        }
    }

    #[test]
    fn non_finite_inputs_use_the_explicit_standardized_sentinel() {
        for non_finite in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for coordinate in 0..6 {
                let mut points = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
                points[coordinate / 2][coordinate % 2] = non_finite;
                let [a, b, c] = points;
                assert_eq!(orient2d(a, b, c), Orientation::Collinear);
                assert_eq!(
                    orient2d_evaluated(a, b, c),
                    Orient2dEvaluation {
                        orientation: Orientation::Collinear,
                        path: Orient2dPath::NonFiniteInput,
                    }
                );
            }
        }
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
    fn orient3d_stays_exact_through_underflow() {
        // Triple products of subnormal coordinates underflow to zero in
        // binary64, yet the tetrahedron is not flat.
        let tiny = f64::from_bits(1);
        let [a, b, c] = [[0.0, 0.0, 0.0], [tiny, 0.0, 0.0], [0.0, tiny, 0.0]];
        assert_eq!(orient3d(a, b, c, [0.0, 0.0, tiny]), Orientation3d::Above);
        assert_eq!(orient3d(a, b, c, [0.0, 0.0, -tiny]), Orientation3d::Below);
        assert_eq!(
            orient3d(a, b, c, [tiny, tiny, 0.0]),
            Orientation3d::Coplanar
        );
        let huge = f64::MAX / 2.0;
        let [a, b, c] = [[0.0, 0.0, 0.0], [huge, 0.0, 0.0], [0.0, huge, 0.0]];
        assert_eq!(orient3d(a, b, c, [0.0, 0.0, huge]), Orientation3d::Above);
        assert_eq!(
            orient3d(a, b, c, [f64::NAN, 0.0, 0.0]),
            Orientation3d::Coplanar
        );
    }

    #[test]
    fn orient3d_matches_an_independent_integer_oracle() {
        // Small integer coordinates admit an exact `i128` determinant; a small
        // cube makes exact coplanar ties common.
        let mut state = 0x2f6b_93d1_c4a8_0e57_u64;
        for round in 0..20_000 {
            let range = if round % 2 == 0 { 3 } else { 1 << 20 };
            let mut points = [[0_i128; 3]; 4];
            for coordinate in points.iter_mut().flatten() {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                *coordinate = i128::from((state >> 33) as i32 % range);
            }
            let [a, b, c, _] = points.map(|p| [0, 1, 2].map(|k| p[k] - points[3][k]));
            let det = a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]);
            let expected = match det.cmp(&0) {
                Ordering::Greater => Orientation3d::Below,
                Ordering::Less => Orientation3d::Above,
                Ordering::Equal => Orientation3d::Coplanar,
            };
            let [a, b, c, d] = points.map(|p| p.map(|v| v as f64));
            assert_eq!(orient3d(a, b, c, d), expected);
            assert_eq!(orient3d_exact(a, b, c, d), expected);
            if let Some(middle) = orient3d_exact_differences(a, b, c, d) {
                assert_eq!(middle, expected);
            }
        }
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
