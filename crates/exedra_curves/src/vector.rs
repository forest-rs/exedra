// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Arithmetic on points and vectors of any dimension, and controls in
//! homogeneous form.

use exedra_math::Real;

/// A NURBS control in homogeneous form: its point scaled by its weight, and
/// the weight. Rational curves are evaluated in this form, where they are
/// polynomial, and divided by the weight at the end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeightedPoint<const N: usize> {
    /// The point times the weight.
    pub weighted: [f64; N],
    /// The weight, positive.
    pub weight: f64,
}

impl<const N: usize> WeightedPoint<N> {
    pub(crate) const ZERO: Self = Self {
        weighted: [0.0; N],
        weight: 0.0,
    };

    /// `point` with `weight`.
    #[must_use]
    pub fn new(point: [f64; N], weight: f64) -> Self {
        Self {
            weighted: point.map(|c| c * weight),
            weight,
        }
    }

    /// The point: the weighted point divided by the weight.
    #[must_use]
    pub fn point(&self) -> [f64; N] {
        self.weighted.map(|c| c / self.weight)
    }

    /// `(1 - u) a + u b`, in every coordinate and the weight.
    pub(crate) fn lerp(a: Self, b: Self, u: f64) -> Self {
        Self {
            weighted: core::array::from_fn(|c| (1.0 - u) * a.weighted[c] + u * b.weighted[c]),
            weight: (1.0 - u) * a.weight + u * b.weight,
        }
    }

    /// `a - b`, in every coordinate and the weight.
    pub(crate) fn minus(a: Self, b: Self) -> Self {
        Self {
            weighted: sub(a.weighted, b.weighted),
            weight: a.weight - b.weight,
        }
    }

    /// `a / s`, in every coordinate and the weight.
    pub(crate) fn divided(self, s: f64) -> Self {
        Self {
            weighted: self.weighted.map(|c| c / s),
            weight: self.weight / s,
        }
    }

    /// `s a`, in every coordinate and the weight.
    pub(crate) fn times(a: Self, s: f64) -> Self {
        Self {
            weighted: scale(a.weighted, s),
            weight: a.weight * s,
        }
    }
}

pub(crate) fn add<const N: usize>(a: [f64; N], b: [f64; N]) -> [f64; N] {
    core::array::from_fn(|i| a[i] + b[i])
}

pub(crate) fn sub<const N: usize>(a: [f64; N], b: [f64; N]) -> [f64; N] {
    core::array::from_fn(|i| a[i] - b[i])
}

pub(crate) fn scale<const N: usize>(a: [f64; N], s: f64) -> [f64; N] {
    a.map(|c| c * s)
}

pub(crate) fn dot<const N: usize>(a: [f64; N], b: [f64; N]) -> f64 {
    let mut sum = 0.0;
    for i in 0..N {
        sum += a[i] * b[i];
    }
    sum
}

pub(crate) fn norm<const N: usize>(a: [f64; N]) -> f64 {
    Real::sqrt(dot(a, a))
}
