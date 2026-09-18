// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Private outward-rounded intervals for polynomial intersection isolation.

use core::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug)]
pub(super) struct I {
    pub(super) lo: f64,
    pub(super) hi: f64,
}
impl I {
    pub(super) const ZERO: Self = Self::p(0.0);
    pub(super) const ONE: Self = Self::p(1.0);
    pub(super) const UNIT: Self = Self { lo: 0.0, hi: 1.0 };
    pub(super) const fn p(x: f64) -> Self {
        Self { lo: x, hi: x }
    }
    pub(super) fn valid(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi
    }
    pub(super) fn width(self) -> f64 {
        self.hi - self.lo
    }
    pub(super) fn mid(self) -> f64 {
        self.lo * 0.5 + self.hi * 0.5
    }
    pub(super) fn contains(self, other: Self) -> bool {
        self.lo <= other.lo && other.hi <= self.hi
    }
    pub(super) fn interior(self, other: Self) -> bool {
        self.lo < other.lo && other.hi < self.hi
    }
    pub(super) fn intersect(self, other: Self) -> Option<Self> {
        let lo = self.lo.max(other.lo);
        let hi = self.hi.min(other.hi);
        (lo <= hi).then_some(Self { lo, hi })
    }
    pub(super) fn union(self, other: Self) -> Self {
        Self {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }
    pub(super) fn excludes_zero(self) -> bool {
        self.lo > 0.0 || self.hi < 0.0
    }
    pub(super) fn square(self) -> Self {
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
            return Self {
                lo: f64::NAN,
                hi: f64::NAN,
            };
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
