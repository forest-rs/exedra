// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact sums of signed binary64 products in fixed-size limb arrays.
//!
//! Every finite binary64 value is a signed integer significand times a power
//! of two, so a product of `FACTORS` values is exact as a multi-word integer
//! shifted by the sum of their exponents. Positive and negative terms
//! accumulate in separate magnitude arrays and are compared once at the end,
//! so no intermediate result is ever rounded.

use core::cmp::Ordering;

/// Exact accumulator for sums of `FACTORS`-fold binary64 products.
///
/// Bit 0 of the arrays is the least exponent a product of `FACTORS` finite
/// values can carry, `-1074 * FACTORS`. Each caller sizes `LIMBS` for the
/// largest product it can add plus the carry from its number of terms.
pub(super) struct Accumulator<const FACTORS: usize, const LIMBS: usize> {
    positive: [u64; LIMBS],
    negative: [u64; LIMBS],
}

impl<const FACTORS: usize, const LIMBS: usize> Accumulator<FACTORS, LIMBS> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "predicates multiply at most five factors"
    )]
    const MIN_EXPONENT: i32 = -1074 * FACTORS as i32;

    pub(super) const fn new() -> Self {
        Self {
            positive: [0; LIMBS],
            negative: [0; LIMBS],
        }
    }

    /// Adds the exact product of `factors`, negated unless `positive`.
    ///
    /// Zero factors contribute nothing. Returns `false` when a factor is not
    /// finite or the product does not fit the limbs; the sum is then invalid.
    pub(super) fn add(&mut self, factors: [f64; FACTORS], positive: bool) -> bool {
        let mut significands = [0_u64; FACTORS];
        let mut exponent = 0;
        let mut negative = !positive;
        for (significand, factor) in significands.iter_mut().zip(factors) {
            match decode(factor) {
                Decoded::Zero => return true,
                Decoded::NonFinite => return false,
                Decoded::Value(value) => {
                    *significand = value.significand;
                    exponent += value.exponent;
                    negative ^= value.negative;
                }
            }
        }
        let Ok(shift) = usize::try_from(exponent - Self::MIN_EXPONENT) else {
            return false;
        };
        let words = multiply_significands(significands);
        let limbs = if negative {
            &mut self.negative
        } else {
            &mut self.positive
        };
        let (word, bits) = (shift / 64, shift % 64);
        words
            .iter()
            .enumerate()
            .all(|(offset, &value)| add_shifted_word(limbs, word + offset, value, bits))
    }

    /// Sign of the exact sum.
    pub(super) fn sign(&self) -> Ordering {
        for (positive, negative) in self.positive.iter().zip(&self.negative).rev() {
            match positive.cmp(negative) {
                Ordering::Equal => {}
                unequal => return unequal,
            }
        }
        Ordering::Equal
    }
}

#[derive(Copy, Clone)]
struct Dyadic {
    negative: bool,
    significand: u64,
    exponent: i32,
}

enum Decoded {
    Zero,
    NonFinite,
    Value(Dyadic),
}

fn decode(value: f64) -> Decoded {
    let bits = value.to_bits();
    let magnitude = bits & 0x7fff_ffff_ffff_ffff;
    if magnitude == 0 {
        return Decoded::Zero;
    }
    let encoded_exponent = ((magnitude >> 52) & 0x7ff) as i32;
    let fraction = magnitude & 0x000f_ffff_ffff_ffff;
    if encoded_exponent == 0x7ff {
        return Decoded::NonFinite;
    }
    let (significand, exponent) = if encoded_exponent == 0 {
        (fraction, -1074)
    } else {
        ((1_u64 << 52) | fraction, encoded_exponent - 1075)
    };
    Decoded::Value(Dyadic {
        negative: bits >> 63 != 0,
        significand,
        exponent,
    })
}

/// Exact product of `N` significands of at most 53 bits, in `N` words.
fn multiply_significands<const N: usize>(factors: [u64; N]) -> [u64; N] {
    let mut words = [0_u64; N];
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
            debug_assert!(len < N, "N binary64 significands fit N limbs");
            words[len] = u64::try_from(carry).expect("multiplication carry fits u64");
            len += 1;
        }
    }
    words
}

fn add_shifted_word<const LIMBS: usize>(
    limbs: &mut [u64; LIMBS],
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

fn add_limb<const LIMBS: usize>(limbs: &mut [u64; LIMBS], mut word: usize, mut value: u64) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn limb_addition_propagates_carry() {
        let mut limbs = [0_u64; 4];
        limbs[0] = u64::MAX;
        limbs[1] = u64::MAX;
        assert!(add_limb(&mut limbs, 0, 1));
        assert_eq!(limbs, [0, 0, 1, 0]);
        assert!(!add_limb(&mut limbs, 4, 1));
    }

    #[test]
    fn cancels_exactly_across_the_exponent_range() {
        // A tiny product survives the cancellation of two huge ones, and
        // subnormal factors keep their exact value.
        let mut sum = Accumulator::<2, 66>::new();
        assert!(sum.add([f64::MAX, 1.0], true));
        assert!(sum.add([f64::MAX, 1.0], false));
        assert!(sum.add([f64::from_bits(1), 1.0], false));
        assert_eq!(sum.sign(), Ordering::Less);
        assert!(!sum.add([f64::NAN, 1.0], true));
    }
}
