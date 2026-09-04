// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Small, deterministic 3-vector helpers over plain arrays.
//!
//! The Exedra workspace speaks `[f32; 3]` and `[f64; 3]` at every public
//! boundary (Exedra ADR-0001). This crate is the one place those arrays get
//! their arithmetic — [`add`], [`sub`], [`scale`], [`dot`], [`cross`],
//! [`norm`], [`normalize`], [`distance_squared`], [`lerp`], [`det3`] — plus
//! the [`promote`]/[`narrow`] pair kernels use at their single `f32` ↔ `f64`
//! boundary and the [`finite`], [`is_unit`], and [`is_orthogonal_frame`]
//! predicates. Mesh storage (`f32`) and kernel arithmetic (`f64`) call the
//! same functions; the arrays choose the precision.
//!
//! It owns nothing else: no vector or point types, no placements, bounding
//! boxes, or matrices, and no transcendental functions.
//!
//! Design rules:
//!
//! - **Plain arrays, free functions.** No newtype and no operator overloading;
//!   call sites read `exedra_math::dot(a, b)` and nothing else changes.
//! - **Correctly rounded only.** Every operation is a fixed sequence of
//!   additions, multiplications, divisions, and IEEE `sqrt`, all of which
//!   round identically under `std` and `libm`. Transcendentals are not
//!   offered here; a crate that needs `acos` keeps its own backend choice.
//! - **Fixed operation order.** The expression each helper evaluates is part
//!   of its contract, because golden fixtures downstream depend on it.
//! - **Explicit tolerances.** Predicates that need a tolerance take one; the
//!   crate has no hidden epsilons.
//!
//! # Features
//!
//! - `std` (default): `sqrt` through the standard library.
//! - `libm`: `sqrt` through the `libm` crate for `no_std` builds. When both are
//!   enabled `std` is used; the results are identical either way.
//!
//! # Example
//!
//! ```
//! use exedra_math::{cross, dot, normalize};
//!
//! let x = [1.0_f64, 0.0, 0.0];
//! let y = [0.0_f64, 1.0, 0.0];
//! assert_eq!(cross(x, y), [0.0, 0.0, 1.0]);
//! assert_eq!(dot(x, y), 0.0);
//!
//! let direction = normalize([3.0_f64, 0.0, 4.0]).expect("non-degenerate vector");
//! assert!((direction[0] - 0.6).abs() < 1.0e-12);
//! assert!((direction[2] - 0.8).abs() < 1.0e-12);
//! ```

#![no_std]

#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra_math requires either the `std` or `libm` feature");

use core::ops::{Add, Div, Mul, Sub};

/// Sealing module: nameable so the `Real` bound is expressible, hidden so it
/// is not part of the documented surface.
#[doc(hidden)]
pub mod sealed {
    /// Implemented for exactly `f32` and `f64`.
    pub trait Sealed {}
    impl Sealed for f32 {}
    impl Sealed for f64 {}
}

/// A real scalar the helpers are generic over: `f32` or `f64`.
///
/// The trait is sealed. It exists only so that one set of functions serves
/// both mesh-storage (`f32`) and kernel (`f64`) precision without a macro.
pub trait Real:
    Copy
    + PartialOrd
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + sealed::Sealed
{
    /// Additive identity.
    const ZERO: Self;
    /// Multiplicative identity.
    const ONE: Self;
    /// The smallest positive normal value.
    const MIN_POSITIVE: Self;

    /// Absolute value.
    #[must_use]
    fn abs(self) -> Self;

    /// Whether the value is neither infinite nor NaN.
    #[must_use]
    fn is_finite(self) -> bool;

    /// Correctly rounded square root.
    #[must_use]
    fn sqrt(self) -> Self;
}

macro_rules! impl_real {
    ($t:ty, $sqrt:path) => {
        impl Real for $t {
            const ZERO: Self = 0.0;
            const ONE: Self = 1.0;
            const MIN_POSITIVE: Self = <$t>::MIN_POSITIVE;

            #[inline]
            fn abs(self) -> Self {
                // Sign-bit manipulation, available in `core`.
                <$t>::abs(self)
            }

            #[inline]
            fn is_finite(self) -> bool {
                <$t>::is_finite(self)
            }

            #[inline]
            fn sqrt(self) -> Self {
                $sqrt(self)
            }
        }
    };
}

#[cfg(feature = "std")]
mod backend {
    extern crate std;

    #[inline]
    pub(crate) fn sqrt_f32(value: f32) -> f32 {
        value.sqrt()
    }

    #[inline]
    pub(crate) fn sqrt_f64(value: f64) -> f64 {
        value.sqrt()
    }
}

#[cfg(all(not(feature = "std"), feature = "libm"))]
mod backend {
    #[inline]
    pub(crate) fn sqrt_f32(value: f32) -> f32 {
        libm::sqrtf(value)
    }

    #[inline]
    pub(crate) fn sqrt_f64(value: f64) -> f64 {
        libm::sqrt(value)
    }
}

impl_real!(f32, backend::sqrt_f32);
impl_real!(f64, backend::sqrt_f64);

/// Componentwise sum `a + b`.
#[inline]
#[must_use]
pub fn add<T: Real>(a: [T; 3], b: [T; 3]) -> [T; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Componentwise difference `a - b`.
#[inline]
#[must_use]
pub fn sub<T: Real>(a: [T; 3], b: [T; 3]) -> [T; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Uniform scaling `a * factor`.
#[inline]
#[must_use]
pub fn scale<T: Real>(a: [T; 3], factor: T) -> [T; 3] {
    [a[0] * factor, a[1] * factor, a[2] * factor]
}

/// Dot product, evaluated as `(a0·b0 + a1·b1) + a2·b2`.
#[inline]
#[must_use]
pub fn dot<T: Real>(a: [T; 3], b: [T; 3]) -> T {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Cross product `a × b`, each component evaluated as one subtraction of two
/// products.
#[inline]
#[must_use]
pub fn cross<T: Real>(a: [T; 3], b: [T; 3]) -> [T; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Euclidean length `sqrt(dot(a, a))`.
#[inline]
#[must_use]
pub fn norm<T: Real>(a: [T; 3]) -> T {
    dot(a, a).sqrt()
}

/// The unit vector along `a`, or `None` when `a` is degenerate.
///
/// Degenerate means the length is not finite, or is at or below
/// `sqrt(MIN_POSITIVE)`: below that, `dot(a, a)` has already lost precision
/// to underflow and the direction is not trustworthy. The result is
/// `scale(a, 1 / length)`; a non-finite result is also reported as `None`.
#[inline]
#[must_use]
pub fn normalize<T: Real>(a: [T; 3]) -> Option<[T; 3]> {
    let length = norm(a);
    if !length.is_finite() || length <= T::MIN_POSITIVE.sqrt() {
        return None;
    }
    let unit = scale(a, T::ONE / length);
    finite(unit).then_some(unit)
}

/// Squared distance between two points, `dot(b - a, b - a)`.
#[inline]
#[must_use]
pub fn distance_squared<T: Real>(a: [T; 3], b: [T; 3]) -> T {
    let d = sub(b, a);
    dot(d, d)
}

/// Linear interpolation `a + (b - a) * t`, componentwise.
#[inline]
#[must_use]
pub fn lerp<T: Real>(a: [T; 3], b: [T; 3], t: T) -> [T; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Determinant of a 3×3 matrix given as rows, by cofactor expansion along
/// the first row. Negative means the matrix reflects.
#[inline]
#[must_use]
pub fn det3<T: Real>(r: [[T; 3]; 3]) -> T {
    r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
        - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
        + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0])
}

/// Widens mesh-storage `f32` coordinates to `f64` for kernel arithmetic.
#[inline]
#[must_use]
pub fn promote(p: [f32; 3]) -> [f64; 3] {
    [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]
}

/// Narrows kernel `f64` coordinates to mesh-storage `f32`.
///
/// Kernels construct geometry in `f64` and narrow exactly once, where new
/// vertices materialize; this is the function to call at that one point.
#[inline]
#[must_use]
pub fn narrow(p: [f64; 3]) -> [f32; 3] {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the documented single f64 -> f32 narrowing point"
    )]
    {
        [p[0] as f32, p[1] as f32, p[2] as f32]
    }
}

/// Whether every component is finite.
#[inline]
#[must_use]
pub fn finite<T: Real>(a: [T; 3]) -> bool {
    a[0].is_finite() && a[1].is_finite() && a[2].is_finite()
}

/// Whether `a` is finite and its length is within `tolerance` of one.
#[inline]
#[must_use]
pub fn is_unit<T: Real>(a: [T; 3], tolerance: T) -> bool {
    finite(a) && (norm(a) - T::ONE).abs() < tolerance
}

/// Whether three axes are pairwise orthogonal: every pairwise dot product has
/// magnitude at most `tolerance`. Says nothing about length or handedness.
#[inline]
#[must_use]
pub fn is_orthogonal_frame<T: Real>(axes: [[T; 3]; 3], tolerance: T) -> bool {
    dot(axes[0], axes[1]).abs() <= tolerance
        && dot(axes[0], axes[2]).abs() <= tolerance
        && dot(axes[1], axes[2]).abs() <= tolerance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_is_componentwise() {
        assert_eq!(add([1.0, 2.0, 3.0], [10.0, 20.0, 30.0]), [11.0, 22.0, 33.0]);
        assert_eq!(
            sub([1.0, 2.0, 3.0], [10.0, 20.0, 30.0]),
            [-9.0, -18.0, -27.0]
        );
        assert_eq!(scale([1.0_f32, 2.0, 3.0], 2.0), [2.0, 4.0, 6.0]);
        assert_eq!(dot([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]), 32.0);
    }

    #[test]
    fn cross_follows_the_right_hand_rule() {
        assert_eq!(cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
        assert_eq!(cross([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]), [0.0, 0.0, -1.0]);
        assert_eq!(
            cross([1.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0]),
            [0.0, 0.0, 0.0],
            "parallel vectors have no cross product"
        );
    }

    #[test]
    fn norm_and_normalize_agree() {
        assert_eq!(norm([3.0, 4.0, 0.0]), 5.0);
        assert_eq!(normalize([0.0, 3.0, 0.0]), Some([0.0, 1.0, 0.0]));
        assert_eq!(normalize([0.0_f32, 0.0, 2.0]), Some([0.0, 0.0, 1.0]));
        assert!(is_unit(
            normalize([1.0, 1.0, 1.0]).expect("non-degenerate"),
            1.0e-12
        ));
    }

    #[test]
    fn normalize_reports_degenerate_input() {
        assert_eq!(normalize([0.0; 3]), None, "zero length");
        assert_eq!(normalize([f64::NAN, 0.0, 0.0]), None, "NaN");
        assert_eq!(normalize([f64::INFINITY, 0.0, 0.0]), None, "infinite");
        let tiny = f64::MIN_POSITIVE.sqrt() * 0.5;
        assert_eq!(
            normalize([tiny, 0.0, 0.0]),
            None,
            "below the underflow floor"
        );
        let small = f64::MIN_POSITIVE.sqrt() * 4.0;
        assert_eq!(
            normalize([small, 0.0, 0.0]),
            Some([1.0, 0.0, 0.0]),
            "above the floor the direction is trustworthy"
        );
        assert_eq!(normalize([0.0_f32; 3]), None, "zero length f32");
    }

    #[test]
    fn predicates_use_explicit_tolerances() {
        assert!(finite([1.0, 2.0, 3.0]));
        assert!(!finite([1.0, f64::NAN, 3.0]));
        assert!(!finite([f32::INFINITY, 0.0, 0.0]));
        assert!(is_unit([0.6, 0.8, 0.0], 1.0e-12));
        assert!(!is_unit([0.6, 0.8, 0.1], 1.0e-12));
        assert!(!is_unit([f64::NAN, 0.0, 0.0], 1.0e-12), "NaN is never unit");
        let frame = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]];
        assert!(
            is_orthogonal_frame(frame, 0.0),
            "handedness is not orthogonality"
        );
        let skew = [[1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        assert!(!is_orthogonal_frame(skew, 1.0e-9));
    }

    #[test]
    fn derived_helpers_match_their_definitions() {
        let a = [1.0, 2.0, 3.0];
        let b = [4.0, 6.0, 8.0];
        assert_eq!(distance_squared(a, b), 9.0 + 16.0 + 25.0);
        assert_eq!(lerp(a, b, 0.5), [2.5, 4.0, 5.5]);
        assert_eq!(lerp(a, b, 0.0), a);
        assert_eq!(lerp(a, b, 1.0), b);
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(det3(identity), 1.0);
        let reflection = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]];
        assert_eq!(det3(reflection), -1.0);
        let rotation = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(det3(rotation), 1.0);
        assert_eq!(promote([1.5_f32, -2.0, 0.25]), [1.5, -2.0, 0.25]);
        assert_eq!(narrow([1.5, -2.0, 0.25]), [1.5_f32, -2.0, 0.25]);
    }

    #[test]
    fn operation_order_is_fixed() {
        // These values make `(a0·b0 + a1·b1) + a2·b2` differ from other
        // associations by one ulp, so the test pins the documented order.
        let a = [1.0e16, 1.0, -1.0e16];
        let b = [1.0, 1.0, 1.0];
        assert_eq!(dot(a, b), (a[0] * b[0] + a[1] * b[1]) + a[2] * b[2]);
    }
}
