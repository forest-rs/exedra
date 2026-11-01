// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Outward-rounded intervals, shared by the crate's certified kernels.
//!
//! Every operation's result contains the exact result for every pair of
//! values its operands contain. An operation that cannot keep that promise
//! in finite `f64`, an overflow or a division by an interval holding zero,
//! returns an interval that is not [`I::valid`], which callers check.

use core::ops::{Add, Div, Mul, Neg, Sub};

/// The real numbers from `lo` to `hi`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct I {
    pub(crate) lo: f64,
    pub(crate) hi: f64,
}
impl I {
    pub(crate) const ZERO: Self = Self::p(0.0);
    pub(crate) const ONE: Self = Self::p(1.0);
    pub(crate) const UNIT: Self = Self { lo: 0.0, hi: 1.0 };
    const INVALID: Self = Self {
        lo: f64::NAN,
        hi: f64::NAN,
    };
    pub(crate) const fn p(x: f64) -> Self {
        Self { lo: x, hi: x }
    }
    pub(crate) fn valid(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi
    }
    pub(crate) fn width(self) -> f64 {
        self.hi - self.lo
    }
    pub(crate) fn mid(self) -> f64 {
        self.lo * 0.5 + self.hi * 0.5
    }
    pub(crate) fn contains(self, other: Self) -> bool {
        self.lo <= other.lo && other.hi <= self.hi
    }
    pub(crate) fn interior(self, other: Self) -> bool {
        self.lo < other.lo && other.hi < self.hi
    }
    pub(crate) fn intersect(self, other: Self) -> Option<Self> {
        let lo = self.lo.max(other.lo);
        let hi = self.hi.min(other.hi);
        (lo <= hi).then_some(Self { lo, hi })
    }
    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }
    pub(crate) fn excludes_zero(self) -> bool {
        self.lo > 0.0 || self.hi < 0.0
    }
    /// `1 / self` for an interval that excludes zero.
    pub(crate) fn recip(self) -> Self {
        Self {
            lo: (1.0 / self.hi).next_down(),
            hi: (1.0 / self.lo).next_up(),
        }
    }
    /// The square root of an interval of nonnegative values, rounded
    /// outward.
    pub(crate) fn sqrt(self) -> Self {
        Self {
            lo: libm::sqrt(self.lo.max(0.0)).next_down().max(0.0),
            hi: libm::sqrt(self.hi).next_up(),
        }
    }
    /// The Euclidean length of `v`, scaled by its largest magnitude first
    /// so that no square overflows or underflows on the way.
    pub(crate) fn norm<const N: usize>(v: [Self; N]) -> Self {
        let scale = v
            .iter()
            .map(|i| i.lo.abs().max(i.hi.abs()))
            .fold(0.0, f64::max);
        if scale == 0.0 {
            return Self::ZERO;
        }
        if !scale.is_finite() {
            return Self::INVALID;
        }
        let sum = v
            .into_iter()
            .fold(Self::ZERO, |sum, i| sum + (i / Self::p(scale)).square());
        sum.sqrt() * Self::p(scale)
    }
    /// The point at fraction `t`, within `[0, 1]`, of the way from `self` to
    /// `other`, held within the two: a convex combination cannot leave
    /// them, however wide `t` is.
    pub(crate) fn lerp(self, other: Self, t: Self) -> Self {
        debug_assert!(Self::UNIT.contains(t), "a fraction of the way");
        if !(self.valid() && other.valid()) {
            return Self::INVALID;
        }
        // This also spares two equal wide intervals the widening their
        // affine combination would add.
        if self.lo == other.lo && self.hi == other.hi {
            return self;
        }
        let combination = (Self::ONE - t) * self + t * other;
        if !combination.valid() {
            return Self::INVALID;
        }
        combination
            .intersect(self.union(other))
            .unwrap_or(Self::INVALID)
    }
    pub(crate) fn square(self) -> Self {
        let a = self.lo * self.lo;
        let b = self.hi * self.hi;
        Self {
            lo: if self.excludes_zero() {
                a.min(b).next_down()
            } else {
                0.0
            },
            hi: a.max(b).next_up(),
        }
    }
}
impl Add for I {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self {
            lo: (self.lo + b.lo).next_down(),
            hi: (self.hi + b.hi).next_up(),
        }
    }
}
impl Sub for I {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        self + -b
    }
}
impl Neg for I {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            lo: -self.hi,
            hi: -self.lo,
        }
    }
}
impl Div for I {
    type Output = Self;
    /// Invalid unless `b` excludes zero.
    fn div(self, b: Self) -> Self {
        if !b.excludes_zero() {
            return Self::INVALID;
        }
        let quotients = [
            self.lo / b.lo,
            self.lo / b.hi,
            self.hi / b.lo,
            self.hi / b.hi,
        ];
        if quotients.iter().any(|v| !v.is_finite()) {
            return Self::INVALID;
        }
        Self {
            lo: quotients
                .into_iter()
                .fold(f64::INFINITY, f64::min)
                .next_down(),
            hi: quotients
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max)
                .next_up(),
        }
    }
}
impl Mul for I {
    type Output = Self;
    fn mul(self, b: Self) -> Self {
        let products = [
            self.lo * b.lo,
            self.lo * b.hi,
            self.hi * b.lo,
            self.hi * b.hi,
        ];
        if products.iter().any(|v| !v.is_finite()) {
            return Self::INVALID;
        }
        Self {
            lo: products
                .into_iter()
                .fold(f64::INFINITY, f64::min)
                .next_down(),
            hi: products
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max)
                .next_up(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::I;

    #[test]
    fn blends_of_one_interval_keep_it_exactly() {
        for a in [
            I::ZERO,
            I::p(f64::from_bits(1)),
            I { lo: -3., hi: 7. },
            I {
                lo: 1e100,
                hi: 2e100,
            },
        ] {
            for t in [I::ZERO, I::ONE, I::p(0.37), I::UNIT] {
                let affine = ((I::ONE - t) * a + t * a).intersect(a).unwrap();
                let found = a.lerp(a, t);
                assert_eq!([found.lo, found.hi], [a.lo, a.hi]);
                assert_eq!([found.lo, found.hi], [affine.lo, affine.hi]);
            }
        }
        // The shortcut still refuses invalid intervals.
        let invalid = I { lo: 2., hi: 1. };
        assert!(!invalid.lerp(invalid, I::UNIT).valid());
        // And spares equal wide intervals an overflow.
        let large = I {
            lo: f64::MAX * 0.5,
            hi: f64::MAX,
        };
        assert!(!((I::ONE - I::UNIT) * large + I::UNIT * large).valid());
        let found = large.lerp(large, I::UNIT);
        assert_eq!([found.lo, found.hi], [large.lo, large.hi]);
    }

    #[test]
    fn norms_neither_overflow_nor_underflow() {
        for scale in [f64::from_bits(1), 1.0, 1e300] {
            let length = I::norm([I::p(3.0 * scale), I::p(-4.0 * scale)]);
            assert!(length.valid());
            assert!(length.lo <= 5.0 * scale && 5.0 * scale <= length.hi);
        }
        assert!(!I::norm([I::p(f64::INFINITY)]).valid());
    }

    #[test]
    fn division_by_an_interval_holding_zero_is_invalid() {
        let q = I::p(1.0) / I { lo: 2.0, hi: 4.0 };
        assert!(q.lo <= 0.25 && 0.5 <= q.hi);
        assert!(!(I::ONE / I { lo: -1.0, hi: 1.0 }).valid());
        assert!(!(I::p(f64::MAX) / I::p(0.5)).valid());
    }
}
