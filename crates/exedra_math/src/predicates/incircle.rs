// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact-sign incircle evaluation.

/// Position of a point relative to an oriented circumcircle.
///
/// For counter-clockwise `a`, `b`, and `c`, [`Inside`](Self::Inside) means
/// `d` lies inside their circumcircle. Reversing the orientation of the first
/// three points swaps [`Inside`](Self::Inside) and [`Outside`](Self::Outside).
/// When the first three points are collinear, the result is only the exact
/// sign of the algebraic determinant; no geometric circle exists.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum InCircle {
    /// Positive determinant: inside for a counter-clockwise defining triple.
    Inside,
    /// Negative determinant: outside for a counter-clockwise defining triple.
    Outside,
    /// Exactly zero determinant.
    Cocircular,
}

/// Arithmetic path used to evaluate an [`incircle()`] query.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IncirclePath {
    /// The ordinary floating-point error-bound filter proved the sign.
    Filter,
    /// A fixed-size exact dyadic accumulator proved the degree-four sign.
    Dyadic,
    /// The query contained a non-finite coordinate, so no sign was evaluated.
    ///
    /// [`IncircleEvaluation::position`] is the deterministic
    /// [`InCircle::Cocircular`] sentinel for this path, not an exact geometric
    /// classification.
    NonFiniteInput,
}

/// Result and evaluated path for one [`incircle_evaluated`] query.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct IncircleEvaluation {
    /// Exact determinant sign for a finite query within
    /// [`crate::predicates::MAX_COORDINATE`].
    ///
    /// This is the deterministic [`InCircle::Cocircular`] sentinel when
    /// [`path`](Self::path) is [`IncirclePath::NonFiniteInput`].
    pub position: InCircle,
    /// Arithmetic path that established the sign, or an explicit invalid-input
    /// status when no sign was evaluated.
    pub path: IncirclePath,
}

/// Half machine epsilon: the unit roundoff `2^-53`.
const U: f64 = f64::EPSILON / 2.0;
/// Shewchuk's error bound for the incircle floating-point filter.
const INCIRCLE_ERRBOUND_A: f64 = (10.0 + 96.0 * U) * U;
/// Least exponent in a product of four finite binary64 values.
const DYADIC_MIN_EXPONENT: i32 = -4296;
/// Enough bits for the complete finite-f64 incircle determinant.
///
/// A four-significand product occupies at most 212 bits. Its largest base
/// exponent is 3884, placing its highest bit at 4095. Relative to the minimum
/// exponent -4296 that is bit 8391; summing 48 monomials can carry through bit
/// 8397, while 132 limbs store through bit 8447.
const DYADIC_LIMBS: usize = 132;

/// Returns the exact position of `d` relative to the oriented circumcircle
/// through `a`, `b`, and `c`.
///
/// For a counter-clockwise defining triple, the variants have their ordinary
/// geometric meaning. A clockwise triple reverses inside and outside. The
/// sign is exact for all finite inputs within
/// [`crate::predicates::MAX_COORDINATE`].
#[must_use]
#[inline]
pub fn incircle(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> InCircle {
    incircle_evaluated(a, b, c, d).position
}

/// Exact incircle sign plus the arithmetic path that established it.
///
/// This diagnostic form performs no global bookkeeping. Non-finite inputs use
/// a deterministic [`InCircle::Cocircular`] sentinel and report
/// [`IncirclePath::NonFiniteInput`]; no geometric classification is claimed.
#[must_use]
#[inline]
pub fn incircle_evaluated(
    a: [f64; 2],
    b: [f64; 2],
    c: [f64; 2],
    d: [f64; 2],
) -> IncircleEvaluation {
    if [a[0], a[1], b[0], b[1], c[0], c[1], d[0], d[1]]
        .iter()
        .any(|coordinate| !coordinate.is_finite())
    {
        return IncircleEvaluation {
            position: InCircle::Cocircular,
            path: IncirclePath::NonFiniteInput,
        };
    }
    if let Some(position) = incircle_filter(a, b, c, d) {
        return IncircleEvaluation {
            position,
            path: IncirclePath::Filter,
        };
    }
    IncircleEvaluation {
        position: sign_of_dyadic_incircle(a, b, c, d),
        path: IncirclePath::Dyadic,
    }
}

#[inline]
fn incircle_filter(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<InCircle> {
    let adx = a[0] - d[0];
    let ady = a[1] - d[1];
    let bdx = b[0] - d[0];
    let bdy = b[1] - d[1];
    let cdx = c[0] - d[0];
    let cdy = c[1] - d[1];

    let bdxcdy = bdx * cdy;
    let cdxbdy = cdx * bdy;
    let cdxady = cdx * ady;
    let adxcdy = adx * cdy;
    let adxbdy = adx * bdy;
    let bdxady = bdx * ady;
    let adx2 = adx * adx;
    let ady2 = ady * ady;
    let bdx2 = bdx * bdx;
    let bdy2 = bdy * bdy;
    let cdx2 = cdx * cdx;
    let cdy2 = cdy * cdy;
    if ![
        product_is_normal_or_exact_zero(bdx, cdy, bdxcdy),
        product_is_normal_or_exact_zero(cdx, bdy, cdxbdy),
        product_is_normal_or_exact_zero(cdx, ady, cdxady),
        product_is_normal_or_exact_zero(adx, cdy, adxcdy),
        product_is_normal_or_exact_zero(adx, bdy, adxbdy),
        product_is_normal_or_exact_zero(bdx, ady, bdxady),
        product_is_normal_or_exact_zero(adx, adx, adx2),
        product_is_normal_or_exact_zero(ady, ady, ady2),
        product_is_normal_or_exact_zero(bdx, bdx, bdx2),
        product_is_normal_or_exact_zero(bdy, bdy, bdy2),
        product_is_normal_or_exact_zero(cdx, cdx, cdx2),
        product_is_normal_or_exact_zero(cdy, cdy, cdy2),
    ]
    .into_iter()
    .all(core::convert::identity)
    {
        return None;
    }

    let alift = adx2 + ady2;
    let blift = bdx2 + bdy2;
    let clift = cdx2 + cdy2;
    let bcdet = bdxcdy - cdxbdy;
    let cadet = cdxady - adxcdy;
    let abdet = adxbdy - bdxady;
    let adet = alift * bcdet;
    let bdet = blift * cadet;
    let cdet = clift * abdet;
    let apermanent = (bdxcdy.abs() + cdxbdy.abs()) * alift;
    let bpermanent = (cdxady.abs() + adxcdy.abs()) * blift;
    let cpermanent = (adxbdy.abs() + bdxady.abs()) * clift;
    if ![
        product_is_normal_or_exact_zero(alift, bcdet, adet),
        product_is_normal_or_exact_zero(blift, cadet, bdet),
        product_is_normal_or_exact_zero(clift, abdet, cdet),
        product_is_normal_or_exact_zero(bdxcdy.abs() + cdxbdy.abs(), alift, apermanent),
        product_is_normal_or_exact_zero(cdxady.abs() + adxcdy.abs(), blift, bpermanent),
        product_is_normal_or_exact_zero(adxbdy.abs() + bdxady.abs(), clift, cpermanent),
    ]
    .into_iter()
    .all(core::convert::identity)
    {
        return None;
    }
    let det = adet + bdet + cdet;
    let permanent = apermanent + bpermanent + cpermanent;

    if !det.is_finite() || !permanent.is_finite() {
        return None;
    }
    let errbound = INCIRCLE_ERRBOUND_A * permanent;
    if errbound.is_normal() && (det > errbound || -det > errbound) {
        return Some(sign_of_det(det));
    }
    None
}

#[inline]
fn product_is_normal_or_exact_zero(left: f64, right: f64, product: f64) -> bool {
    left == 0.0 || right == 0.0 || product.is_normal()
}

#[derive(Copy, Clone)]
struct Dyadic {
    negative: bool,
    significand: u64,
    exponent: i32,
}

fn decode_dyadic(value: f64) -> Option<Dyadic> {
    let bits = value.to_bits();
    let magnitude = bits & 0x7fff_ffff_ffff_ffff;
    if magnitude == 0 {
        return None;
    }
    let encoded_exponent = ((magnitude >> 52) & 0x7ff) as i32;
    let fraction = magnitude & 0x000f_ffff_ffff_ffff;
    if encoded_exponent == 0x7ff {
        return None;
    }
    let (significand, exponent) = if encoded_exponent == 0 {
        (fraction, -1074)
    } else {
        ((1_u64 << 52) | fraction, encoded_exponent - 1075)
    };
    Some(Dyadic {
        negative: bits >> 63 != 0,
        significand,
        exponent,
    })
}

fn sign_of_dyadic_incircle(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> InCircle {
    let mut positive = [0_u64; DYADIC_LIMBS];
    let mut negative = [0_u64; DYADIC_LIMBS];

    // Homogeneous determinant, expanded along the squared-length column:
    //   a² det(b,c,d) - b² det(a,c,d)
    // + c² det(a,b,d) - d² det(a,b,c).
    // Expanding each squared length and 3×3 determinant produces 48 exact
    // four-factor monomials. Finite inputs are guaranteed to fit the reviewed
    // accumulator bound, so failure here is an internal invariant violation.
    let accumulated = add_lifted_determinant(&mut positive, &mut negative, a, b, c, d, true)
        && add_lifted_determinant(&mut positive, &mut negative, b, a, c, d, false)
        && add_lifted_determinant(&mut positive, &mut negative, c, a, b, d, true)
        && add_lifted_determinant(&mut positive, &mut negative, d, a, b, c, false);
    debug_assert!(accumulated, "reviewed incircle accumulator bound");

    for (&positive, &negative) in positive.iter().zip(&negative).rev() {
        if positive > negative {
            return InCircle::Inside;
        }
        if positive < negative {
            return InCircle::Outside;
        }
    }
    InCircle::Cocircular
}

fn add_lifted_determinant(
    positive: &mut [u64; DYADIC_LIMBS],
    negative: &mut [u64; DYADIC_LIMBS],
    lift: [f64; 2],
    p: [f64; 2],
    q: [f64; 2],
    r: [f64; 2],
    positive_cofactor: bool,
) -> bool {
    let determinant_terms = [
        (p[0], q[1], true),
        (q[0], r[1], true),
        (r[0], p[1], true),
        (p[0], r[1], false),
        (q[0], p[1], false),
        (r[0], q[1], false),
    ];
    for squared_coordinate in lift {
        for (left, right, positive_determinant_term) in determinant_terms {
            if !add_monomial(
                positive,
                negative,
                [squared_coordinate, squared_coordinate, left, right],
                positive_cofactor == positive_determinant_term,
            ) {
                return false;
            }
        }
    }
    true
}

fn add_monomial(
    positive: &mut [u64; DYADIC_LIMBS],
    negative: &mut [u64; DYADIC_LIMBS],
    factors: [f64; 4],
    positive_coefficient: bool,
) -> bool {
    let [Some(a), Some(b), Some(c), Some(d)] = factors.map(decode_dyadic) else {
        return true;
    };
    let words = multiply_significands([a.significand, b.significand, c.significand, d.significand]);
    let Some(shift) =
        usize::try_from(a.exponent + b.exponent + c.exponent + d.exponent - DYADIC_MIN_EXPONENT)
            .ok()
    else {
        return false;
    };
    let negative_term = !positive_coefficient ^ a.negative ^ b.negative ^ c.negative ^ d.negative;
    let magnitude = if negative_term { negative } else { positive };
    add_shifted_words(magnitude, &words, shift)
}

fn multiply_significands(factors: [u64; 4]) -> [u64; 4] {
    let mut words = [0_u64; 4];
    words[0] = 1;
    let mut len = 1;
    for factor in factors {
        let mut carry = 0_u128;
        for word in &mut words[..len] {
            let product = u128::from(*word) * u128::from(factor) + carry;
            *word = u64::try_from(product & u128::from(u64::MAX))
                .expect("masked product limb fits u64");
            carry = product >> 64;
        }
        if carry != 0 {
            debug_assert!(
                len < words.len(),
                "four binary64 significands fit four limbs"
            );
            words[len] = u64::try_from(carry).expect("multiplication carry fits u64");
            len += 1;
        }
    }
    words
}

fn add_shifted_words(limbs: &mut [u64; DYADIC_LIMBS], words: &[u64], shift: usize) -> bool {
    let word = shift / 64;
    let bits = shift % 64;
    words
        .iter()
        .enumerate()
        .all(|(offset, &value)| add_shifted_word(limbs, word + offset, value, bits))
}

fn add_shifted_word(
    limbs: &mut [u64; DYADIC_LIMBS],
    word: usize,
    value: u64,
    shift: usize,
) -> bool {
    if value == 0 {
        return true;
    }
    if !add_limb(limbs, word, value << shift) {
        return false;
    }
    shift == 0 || add_limb(limbs, word + 1, value >> (64 - shift))
}

fn add_limb(limbs: &mut [u64; DYADIC_LIMBS], mut word: usize, mut value: u64) -> bool {
    while value != 0 {
        let Some(limb) = limbs.get_mut(word) else {
            return false;
        };
        let (sum, carry) = limb.overflowing_add(value);
        *limb = sum;
        value = u64::from(carry);
        word += 1;
    }
    true
}

#[inline]
fn sign_of_det(det: f64) -> InCircle {
    if det > 0.0 {
        InCircle::Inside
    } else if det < 0.0 {
        InCircle::Outside
    } else {
        InCircle::Cocircular
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn power_of_two(exponent: i32) -> f64 {
        debug_assert!((-1022..=1023).contains(&exponent));
        f64::from_bits(((exponent + 1023) as u64) << 52)
    }

    #[test]
    fn classifies_clear_and_oriented_cases() {
        // Clear inside, outside, tie, and winding-reversal cases should stay
        // on the inexpensive filter whenever the sign is well separated.
        let a = [0.0, 0.0];
        let b = [1.0, 0.0];
        let c = [0.0, 1.0];

        assert_eq!(incircle(a, b, c, [0.25, 0.25]), InCircle::Inside);
        assert_eq!(incircle(a, b, c, [2.0, 2.0]), InCircle::Outside);
        assert_eq!(incircle(a, b, c, [1.0, 1.0]), InCircle::Cocircular);
        assert_eq!(incircle(a, c, b, [0.25, 0.25]), InCircle::Outside);
        assert_eq!(incircle(b, c, a, [0.25, 0.25]), InCircle::Inside);
        assert_eq!(
            incircle_evaluated(a, b, c, [0.25, 0.25]).path,
            IncirclePath::Filter
        );
    }

    #[test]
    fn resolves_one_ulp_around_an_exact_circle() {
        // Adjacent binary64 values around a cocircular query trap the exact
        // fallback's ability to distinguish a one-ULP displacement.
        let a = [0.0, 0.0];
        let b = [1.0, 0.0];
        let c = [0.0, 1.0];
        let one_below = f64::from_bits(1.0_f64.to_bits() - 1);
        let one_above = f64::from_bits(1.0_f64.to_bits() + 1);

        assert_eq!(incircle(a, b, c, [1.0, one_below]), InCircle::Inside);
        assert_eq!(incircle(a, b, c, [1.0, 1.0]), InCircle::Cocircular);
        assert_eq!(incircle(a, b, c, [1.0, one_above]), InCircle::Outside);
        assert_eq!(
            incircle_evaluated(a, b, c, [1.0, 1.0]).path,
            IncirclePath::Dyadic
        );
    }

    #[test]
    fn exact_path_handles_scaled_and_extreme_inputs() {
        // Exact ties and their immediate neighbors must retain their sign
        // across ordinary, tiny, and maximum supported coordinate scales.
        for scale in [power_of_two(-500), 1.0, power_of_two(300)] {
            let a = [0.0, 0.0];
            let b = [scale, 0.0];
            let c = [0.0, scale];
            assert_eq!(incircle(a, b, c, [scale, scale]), InCircle::Cocircular);
        }

        let extent = 1e100_f64;
        let below = f64::from_bits(extent.to_bits() - 1);
        let above = f64::from_bits(extent.to_bits() + 1);
        let a = [0.0, 0.0];
        let b = [extent, 0.0];
        let c = [0.0, extent];
        for (point, expected) in [
            ([extent, below], InCircle::Inside),
            ([extent, extent], InCircle::Cocircular),
            ([extent, above], InCircle::Outside),
        ] {
            let evaluated = incircle_evaluated(a, b, c, point);
            assert_eq!(evaluated.position, expected);
            assert_eq!(evaluated.path, IncirclePath::Dyadic);
        }
    }

    #[test]
    fn filter_defers_when_an_underflowed_cross_term_is_later_amplified() {
        // This fixed regression contains a product that underflows before a
        // large lift amplifies it; the filter must defer instead of guessing.
        let values = [
            0x327d_42cc_6270_baa5,
            0x952e_6483_d4ee_5290,
            0x9d49_2720_76eb_2a35,
            0x838a_8833_5c93_eeca,
            0x27e5_3c76_1426_3b97,
            0xd375_44eb_d2b2_7220,
            0xa69e_215f_da47_9ec4,
            0x0136_be3b_463e_20f9,
        ]
        .map(f64::from_bits);
        let evaluated = incircle_evaluated(
            [values[0], values[1]],
            [values[2], values[3]],
            [values[4], values[5]],
            [values[6], values[7]],
        );
        assert_eq!(evaluated.position, InCircle::Inside);
        assert_eq!(evaluated.path, IncirclePath::Dyadic);
    }

    #[test]
    fn degenerate_and_nonfinite_contracts_are_explicit() {
        // Degenerate finite input has an exact zero determinant, while
        // non-finite input must take the documented diagnostic sentinel path.
        let point = [2.0, -3.0];
        assert_eq!(
            incircle(point, point, point, [4.0, 5.0]),
            InCircle::Cocircular
        );

        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let evaluated = incircle_evaluated([invalid, 0.0], [1.0, 0.0], [0.0, 1.0], [0.0, 0.0]);
            assert_eq!(evaluated.position, InCircle::Cocircular);
            assert_eq!(evaluated.path, IncirclePath::NonFiniteInput);
        }
    }

    #[test]
    fn four_significand_product_propagates_every_carry() {
        // Four maximum-width significands exercise every output limb and the
        // carry propagation used by each exact determinant monomial.
        let factors = [(1_u64 << 53) - 1; 4];
        assert_eq!(
            multiply_significands(factors),
            [
                0xff80_0000_0000_0001,
                0x0000_17ff_ffff_ffff,
                0xffff_fffe_0000_0000,
                0x0000_0000_000f_ffff,
            ]
        );
    }

    #[test]
    fn matches_an_independent_integer_oracle() {
        // Small integer coordinates admit an independent exact `i128`
        // determinant, checking both the public dispatcher and dyadic path.
        fn exact(a: [i128; 2], b: [i128; 2], c: [i128; 2], d: [i128; 2]) -> InCircle {
            let adx = a[0] - d[0];
            let ady = a[1] - d[1];
            let bdx = b[0] - d[0];
            let bdy = b[1] - d[1];
            let cdx = c[0] - d[0];
            let cdy = c[1] - d[1];
            let det = (adx * adx + ady * ady) * (bdx * cdy - cdx * bdy)
                + (bdx * bdx + bdy * bdy) * (cdx * ady - adx * cdy)
                + (cdx * cdx + cdy * cdy) * (adx * bdy - bdx * ady);
            match det.cmp(&0) {
                core::cmp::Ordering::Greater => InCircle::Inside,
                core::cmp::Ordering::Less => InCircle::Outside,
                core::cmp::Ordering::Equal => InCircle::Cocircular,
            }
        }

        let mut state = 0x49df_212a_7f31_d6c5_u64;
        for _ in 0..10_000 {
            let mut points = [[0_i128; 2]; 4];
            for coordinate in points.iter_mut().flatten() {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                *coordinate = i128::from((state >> 48) as i16);
            }
            let as_f64 = |point: [i128; 2]| [point[0] as f64, point[1] as f64];
            let [a, b, c, d] = points.map(as_f64);
            let expected = exact(points[0], points[1], points[2], points[3]);
            assert_eq!(incircle(a, b, c, d), expected);
            assert_eq!(sign_of_dyadic_incircle(a, b, c, d), expected);
        }
    }
}
