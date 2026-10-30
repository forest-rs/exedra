// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Numbers a [`Formula`](crate::Formula) is evaluated in.
//!
//! A formula is written once, generically over [`Number`], and evaluated in
//! whichever number answers the question at hand: `f64` for values, [`Dual`]
//! for values with exact gradients, [`Interval`] for bounds over a box, and
//! `Dual<Interval>` for bounds on the gradient over a box.
//!
//! `Number` has no comparisons, so a formula cannot branch on a value. It
//! chooses with [`Number::min`], [`Number::max`], [`Number::abs`] and
//! [`Number::select`] instead, which stay meaningful for numbers that stand
//! for many values at once. Constants go on the right of an operator, as in
//! `x * 7.0`, or through [`Number::constant`].
//!
//! # Creases
//!
//! `min`, `max` and `abs` mark creases: where their two sides meet, the
//! surface may turn sharply, as along the edge where a box's faces meet or
//! the rim where a cut leaves a solid. Extraction finds such edges from the
//! marks and keeps them sharp. [`Number::select`] joins two pieces that meet
//! smoothly and marks nothing, as where a capsule's cylinder meets its caps.

use core::f64::consts::{FRAC_PI_2, PI, TAU};
use core::ops::{Add, Div, Mul, Neg, Sub};

use exedra_math::Real;

use crate::{Aabb, ScalarField, Tie, blend};

/// Sealing module: nameable so the `Number` bound is expressible, hidden so
/// it is not part of the documented surface.
#[doc(hidden)]
pub mod sealed {
    use super::Dual;
    use crate::ScalarField;

    /// Implemented for exactly the numbers in this module.
    pub trait Sealed: Sized {
        /// `field` at `p`, an opaque leaf of a formula, in this number.
        fn sample<F: ScalarField + ?Sized>(field: &F, p: [Self; 3]) -> Self;
        /// The least and greatest value this number stands for.
        fn span(self) -> super::Interval;
        /// The symmetric smooth union of `values` with blend `radius`.
        fn smooth_union(values: &[Self], radius: f64) -> Self;
        /// This number as the value of a part tagged `region`.
        fn tagged(self, _region: u32) -> Self {
            self
        }
    }

    /// How a dual over this number chooses at the kinks of `min`, `max`,
    /// `abs` and `select`, and samples opaque fields.
    pub trait DualBranch: Sized {
        /// The lesser operand, with a gradient valid wherever it is chosen.
        fn dual_min(a: Dual<Self>, b: Dual<Self>) -> Dual<Self>;
        /// The greater operand, with a gradient valid wherever it is chosen.
        fn dual_max(a: Dual<Self>, b: Dual<Self>) -> Dual<Self>;
        /// The absolute value, with a gradient valid on either side.
        fn dual_abs(a: Dual<Self>) -> Dual<Self>;
        /// `below` where `condition` is negative, otherwise `above`.
        fn dual_select(condition: Dual<Self>, below: Dual<Self>, above: Dual<Self>) -> Dual<Self>;
        /// The length of `(a, b)`, with a gradient bounded where it is zero.
        fn dual_hypot(a: Dual<Self>, b: Dual<Self>) -> Dual<Self>;
        /// `field` at `p`, by the chain rule through `p`'s gradients.
        fn dual_sample<F: ScalarField + ?Sized>(field: &F, p: [Dual<Self>; 3]) -> Dual<Self>;
        /// The symmetric smooth union of `values` with blend `radius`.
        fn dual_smooth_union(values: &[Dual<Self>], radius: f64) -> Dual<Self>;
    }
}

use sealed::DualBranch;

/// A number a [`Formula`](crate::Formula) can be evaluated in.
///
/// Implemented for `f64`, [`Interval`], and [`Dual`] over either; sealed, so
/// more numbers can join without breaking formulas.
pub trait Number:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
    + Add<f64, Output = Self>
    + Sub<f64, Output = Self>
    + Mul<f64, Output = Self>
    + Div<f64, Output = Self>
    + sealed::Sealed
{
    /// A number that does not vary.
    fn constant(value: f64) -> Self;

    /// The absolute value, marking a crease where it is zero.
    #[must_use]
    fn abs(self) -> Self;

    /// The square root.
    #[must_use]
    fn sqrt(self) -> Self;

    /// The sine and cosine together.
    fn sin_cos(self) -> (Self, Self);

    /// The lesser of two numbers, marking a crease where they meet.
    #[must_use]
    fn min(self, other: Self) -> Self;

    /// The greater of two numbers, marking a crease where they meet.
    #[must_use]
    fn max(self, other: Self) -> Self;

    /// The length of the vector `(self, other)`.
    ///
    /// Unlike `(a * a + b * b).sqrt()`, its gradient stays bounded where the
    /// length is zero, as it is everywhere inside a box for the length of
    /// the box's outside part.
    #[must_use]
    fn hypot(self, other: Self) -> Self;

    /// `below` where `self` is negative, otherwise `above`, marking no
    /// crease.
    ///
    /// For pieces that meet where `self` is zero, so that the result is
    /// continuous. A number standing for values on both sides takes both
    /// pieces' bounds.
    #[must_use]
    fn select(self, below: Self, above: Self) -> Self;

    /// The sine.
    #[must_use]
    fn sin(self) -> Self {
        self.sin_cos().0
    }

    /// The cosine.
    #[must_use]
    fn cos(self) -> Self {
        self.sin_cos().1
    }

    /// The square.
    #[must_use]
    fn square(self) -> Self {
        self * self
    }
}

impl sealed::Sealed for f64 {
    fn sample<F: ScalarField + ?Sized>(field: &F, p: [Self; 3]) -> Self {
        let mut value = [0.0];
        field.eval_points(&[p], &mut value);
        value[0]
    }

    fn span(self) -> Interval {
        Interval::new(self, self)
    }

    fn smooth_union(values: &[Self], radius: f64) -> Self {
        blend::blend(values.iter().map(|v| (*v, [0.0; 3])), radius).0
    }
}

impl Number for f64 {
    fn constant(value: f64) -> Self {
        value
    }

    fn abs(self) -> Self {
        Real::abs(self)
    }

    fn sqrt(self) -> Self {
        Real::sqrt(self)
    }

    fn sin_cos(self) -> (Self, Self) {
        Real::sin_cos(self)
    }

    fn min(self, other: Self) -> Self {
        Self::min(self, other)
    }

    fn max(self, other: Self) -> Self {
        Self::max(self, other)
    }

    /// Scaled by the larger magnitude, so squares neither underflow nor
    /// overflow.
    fn hypot(self, other: Self) -> Self {
        let (a, b) = (self.abs(), other.abs());
        if a.is_nan() || b.is_nan() {
            return Self::NAN;
        }
        let (big, small) = if a >= b { (a, b) } else { (b, a) };
        if big == 0.0 || big.is_infinite() {
            return big;
        }
        let ratio = small / big;
        big * Real::sqrt(1.0 + ratio * ratio)
    }

    fn select(self, below: Self, above: Self) -> Self {
        if self < 0.0 { below } else { above }
    }
}

/// A closed range of `f64` values, rounded outward so it always contains
/// every value the exact operations could take.
///
/// An interval that cannot be bounded, as from a division by a range
/// holding zero, is the whole line.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Interval {
    low: f64,
    high: f64,
}

impl Interval {
    /// Every value.
    pub const ENTIRE: Self = Self {
        low: f64::NEG_INFINITY,
        high: f64::INFINITY,
    };

    /// The range from `low` to `high`; the whole line when either is NaN.
    ///
    /// # Panics
    ///
    /// Panics when `low` exceeds `high`.
    #[must_use]
    pub fn new(low: f64, high: f64) -> Self {
        if low.is_nan() || high.is_nan() {
            return Self::ENTIRE;
        }
        assert!(low <= high, "interval ends must be ordered");
        Self { low, high }
    }

    /// The least value.
    #[must_use]
    pub const fn low(self) -> f64 {
        self.low
    }

    /// The greatest value.
    #[must_use]
    pub const fn high(self) -> f64 {
        self.high
    }

    /// The smallest interval containing both.
    #[must_use]
    pub fn hull(self, other: Self) -> Self {
        Self::new(self.low.min(other.low), self.high.max(other.high))
    }

    /// The values in both, or `None` when they do not meet.
    #[must_use]
    pub fn intersect(self, other: Self) -> Option<Self> {
        let (low, high) = (self.low.max(other.low), self.high.min(other.high));
        (low <= high).then(|| Self::new(low, high))
    }

    /// Whether the interval holds `value`.
    #[must_use]
    pub fn contains(self, value: f64) -> bool {
        self.low <= value && value <= self.high
    }

    fn rounded(low: f64, high: f64) -> Self {
        Self::new(low.next_down(), high.next_up())
    }

    /// Bounds of a function with one maximum per period at `peak` and one
    /// minimum half a period on, from its values at the ends.
    fn periodic(self, function: impl Fn(f64) -> f64, peak: f64) -> Self {
        if !(self.low.is_finite() && self.high.is_finite()) || self.high - self.low >= TAU {
            return Self::new(-1.0, 1.0);
        }
        let (a, b) = (function(self.low), function(self.high));
        // Library sines are within an ulp or two; widen past that.
        let mut low = a.min(b).next_down().next_down();
        let mut high = a.max(b).next_up().next_up();
        if self.reaches_phase(peak) {
            high = 1.0;
        }
        if self.reaches_phase(peak + PI) {
            low = -1.0;
        }
        Self::new(low.max(-1.0), high.min(1.0))
    }

    /// Whether the interval may hold `phase + 2kπ` for an integer `k`,
    /// answering yes when rounding leaves it in doubt.
    fn reaches_phase(self, phase: f64) -> bool {
        let slack = 1.0e-9 * (1.0 + self.low.abs() + self.high.abs());
        let turns = (self.low - slack - phase) / TAU;
        // So many turns out, the slack spans whole periods.
        if turns.is_nan() || turns.abs() >= 1.0e15 {
            return true;
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "fewer whole turns than i64 holds"
        )]
        let whole = turns as i64;
        #[expect(clippy::cast_precision_loss, reason = "small whole turns")]
        let next = if (whole as f64) < turns {
            whole + 1
        } else {
            whole
        };
        #[expect(clippy::cast_precision_loss, reason = "small whole turns")]
        let at = phase + next as f64 * TAU;
        at <= self.high + slack
    }
}

/// The product of two ends, taking zero times infinity as zero.
fn product(a: f64, b: f64) -> f64 {
    if a == 0.0 || b == 0.0 { 0.0 } else { a * b }
}

impl Add for Interval {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::rounded(self.low + other.low, self.high + other.high)
    }
}

impl Sub for Interval {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self::rounded(self.low - other.high, self.high - other.low)
    }
}

impl Mul for Interval {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        let ends = [
            product(self.low, other.low),
            product(self.low, other.high),
            product(self.high, other.low),
            product(self.high, other.high),
        ];
        let low = ends.iter().copied().fold(f64::INFINITY, f64::min);
        let high = ends.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Self::rounded(low, high)
    }
}

impl Div for Interval {
    type Output = Self;

    fn div(self, other: Self) -> Self {
        if other.contains(0.0) {
            return Self::ENTIRE;
        }
        self * Self::new((1.0 / other.high).next_down(), (1.0 / other.low).next_up())
    }
}

impl Neg for Interval {
    type Output = Self;

    fn neg(self) -> Self {
        Self::new(-self.high, -self.low)
    }
}

impl Add<f64> for Interval {
    type Output = Self;

    fn add(self, other: f64) -> Self {
        Self::rounded(self.low + other, self.high + other)
    }
}

impl Sub<f64> for Interval {
    type Output = Self;

    fn sub(self, other: f64) -> Self {
        Self::rounded(self.low - other, self.high - other)
    }
}

impl Mul<f64> for Interval {
    type Output = Self;

    fn mul(self, other: f64) -> Self {
        self * Self::constant(other)
    }
}

impl Div<f64> for Interval {
    type Output = Self;

    fn div(self, other: f64) -> Self {
        self / Self::constant(other)
    }
}

impl sealed::Sealed for Interval {
    fn sample<F: ScalarField + ?Sized>(field: &F, p: [Self; 3]) -> Self {
        region(p).map_or(Self::ENTIRE, |region| field.eval_interval(&region))
    }

    fn span(self) -> Interval {
        self
    }

    fn smooth_union(values: &[Self], radius: f64) -> Self {
        blend::blend_interval(values, radius)
    }
}

/// The box spanned by interval coordinates, when finite.
fn region(p: [Interval; 3]) -> Option<Aabb> {
    Aabb::new(p.map(Interval::low), p.map(Interval::high))
}

impl Number for Interval {
    fn constant(value: f64) -> Self {
        Self::new(value, value)
    }

    fn abs(self) -> Self {
        if self.low >= 0.0 {
            self
        } else if self.high <= 0.0 {
            -self
        } else {
            Self::new(0.0, self.high.max(-self.low))
        }
    }

    fn sqrt(self) -> Self {
        if self.high < 0.0 {
            return Self::ENTIRE;
        }
        let low = Real::sqrt(self.low.max(0.0)).next_down().max(0.0);
        Self::new(low, Real::sqrt(self.high).next_up())
    }

    fn sin_cos(self) -> (Self, Self) {
        (
            self.periodic(|x| Real::sin_cos(x).0, FRAC_PI_2),
            self.periodic(|x| Real::sin_cos(x).1, 0.0),
        )
    }

    fn min(self, other: Self) -> Self {
        Self::new(self.low.min(other.low), self.high.min(other.high))
    }

    fn max(self, other: Self) -> Self {
        Self::new(self.low.max(other.low), self.high.max(other.high))
    }

    fn hypot(self, other: Self) -> Self {
        (self.square() + other.square()).sqrt()
    }

    fn select(self, below: Self, above: Self) -> Self {
        if self.high < 0.0 {
            below
        } else if self.low >= 0.0 {
            above
        } else {
            below.hull(above)
        }
    }

    fn square(self) -> Self {
        let (near, far) = if self.low >= 0.0 {
            (self.low, self.high)
        } else if self.high <= 0.0 {
            (-self.high, -self.low)
        } else {
            (0.0, self.high.max(-self.low))
        };
        Self::new((near * near).next_down().max(0.0), (far * far).next_up())
    }
}

/// A value with its gradient: forward-mode automatic differentiation.
///
/// Evaluating a formula on duals seeded by [`Dual::point`] carries each
/// operation's derivative along with its value, so one evaluation yields
/// the gradient. Over `f64` it is the exact gradient; where the formula is
/// not differentiable, as at a [`Number::min`] tie or the apex of
/// [`Number::abs`], it is one side's, and at a zero [`Number::sqrt`] it is
/// not finite. Over [`Interval`] it bounds the gradient over a box, taking
/// both sides' where a kink may fall inside it.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Dual<T = f64> {
    /// The value.
    pub value: T,
    /// The value's partial derivatives along `x`, `y` and `z`.
    pub gradient: [T; 3],
}

impl<T: Number> Dual<T> {
    /// The coordinate `value` along `axis`, whose derivative along it is one.
    ///
    /// # Panics
    ///
    /// Panics when `axis` is not `0`, `1` or `2`.
    #[must_use]
    pub fn variable(value: T, axis: usize) -> Self {
        assert!(axis < 3, "axis must be 0, 1 or 2");
        Self {
            value,
            gradient: core::array::from_fn(|i| T::constant(if i == axis { 1.0 } else { 0.0 })),
        }
    }

    /// Duals for the coordinates of `point`.
    #[must_use]
    pub fn point(point: [T; 3]) -> [Self; 3] {
        core::array::from_fn(|axis| Self::variable(point[axis], axis))
    }

    fn chain(self, value: T, slope: T) -> Self {
        Self {
            value,
            gradient: self.gradient.map(|component| component * slope),
        }
    }

    /// A function of `p` with `value` and partial derivatives `slope` along
    /// `p`'s coordinates, differentiated through `p`.
    fn through(value: T, slope: [T; 3], p: &[Self; 3]) -> Self {
        Self {
            value,
            gradient: core::array::from_fn(|i| {
                slope[0] * p[0].gradient[i]
                    + slope[1] * p[1].gradient[i]
                    + slope[2] * p[2].gradient[i]
            }),
        }
    }
}

impl<T: Number> Add for Dual<T> {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            value: self.value + other.value,
            gradient: core::array::from_fn(|i| self.gradient[i] + other.gradient[i]),
        }
    }
}

impl<T: Number> Sub for Dual<T> {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            value: self.value - other.value,
            gradient: core::array::from_fn(|i| self.gradient[i] - other.gradient[i]),
        }
    }
}

impl<T: Number> Mul for Dual<T> {
    type Output = Self;

    #[expect(clippy::suspicious_arithmetic_impl, reason = "the product rule")]
    fn mul(self, other: Self) -> Self {
        Self {
            value: self.value * other.value,
            gradient: core::array::from_fn(|i| {
                self.gradient[i] * other.value + self.value * other.gradient[i]
            }),
        }
    }
}

impl<T: Number> Div for Dual<T> {
    type Output = Self;

    #[expect(clippy::suspicious_arithmetic_impl, reason = "the quotient rule")]
    fn div(self, other: Self) -> Self {
        let value = self.value / other.value;
        Self {
            value,
            gradient: core::array::from_fn(|i| {
                (self.gradient[i] - value * other.gradient[i]) / other.value
            }),
        }
    }
}

impl<T: Number> Neg for Dual<T> {
    type Output = Self;

    fn neg(self) -> Self {
        Self {
            value: -self.value,
            gradient: self.gradient.map(|component| -component),
        }
    }
}

impl<T: Number> Add<f64> for Dual<T> {
    type Output = Self;

    fn add(self, other: f64) -> Self {
        Self {
            value: self.value + other,
            gradient: self.gradient,
        }
    }
}

impl<T: Number> Sub<f64> for Dual<T> {
    type Output = Self;

    fn sub(self, other: f64) -> Self {
        Self {
            value: self.value - other,
            gradient: self.gradient,
        }
    }
}

impl<T: Number> Mul<f64> for Dual<T> {
    type Output = Self;

    fn mul(self, other: f64) -> Self {
        Self {
            value: self.value * other,
            gradient: self.gradient.map(|component| component * other),
        }
    }
}

impl<T: Number> Div<f64> for Dual<T> {
    type Output = Self;

    fn div(self, other: f64) -> Self {
        Self {
            value: self.value / other,
            gradient: self.gradient.map(|component| component / other),
        }
    }
}

impl<T: Number + DualBranch> sealed::Sealed for Dual<T> {
    fn sample<F: ScalarField + ?Sized>(field: &F, p: [Self; 3]) -> Self {
        T::dual_sample(field, p)
    }

    fn span(self) -> Interval {
        self.value.span()
    }

    fn smooth_union(values: &[Self], radius: f64) -> Self {
        T::dual_smooth_union(values, radius)
    }
}

impl<T: Number + DualBranch> Number for Dual<T> {
    fn constant(value: f64) -> Self {
        Self {
            value: T::constant(value),
            gradient: [T::constant(0.0); 3],
        }
    }

    fn abs(self) -> Self {
        T::dual_abs(self)
    }

    fn sqrt(self) -> Self {
        let value = self.value.sqrt();
        self.chain(value, T::constant(0.5) / value)
    }

    fn sin_cos(self) -> (Self, Self) {
        let (sin, cos) = self.value.sin_cos();
        (self.chain(sin, cos), self.chain(cos, -sin))
    }

    fn min(self, other: Self) -> Self {
        T::dual_min(self, other)
    }

    fn max(self, other: Self) -> Self {
        T::dual_max(self, other)
    }

    fn hypot(self, other: Self) -> Self {
        T::dual_hypot(self, other)
    }

    fn select(self, below: Self, above: Self) -> Self {
        T::dual_select(self, below, above)
    }

    fn square(self) -> Self {
        self.chain(self.value.square(), self.value * 2.0)
    }
}

impl DualBranch for f64 {
    fn dual_min(a: Dual<Self>, b: Dual<Self>) -> Dual<Self> {
        if b.value < a.value { b } else { a }
    }

    fn dual_max(a: Dual<Self>, b: Dual<Self>) -> Dual<Self> {
        if b.value > a.value { b } else { a }
    }

    fn dual_abs(a: Dual<Self>) -> Dual<Self> {
        if a.value < 0.0 { -a } else { a }
    }

    fn dual_select(condition: Dual<Self>, below: Dual<Self>, above: Dual<Self>) -> Dual<Self> {
        if condition.value < 0.0 { below } else { above }
    }

    fn dual_hypot(a: Dual<Self>, b: Dual<Self>) -> Dual<Self> {
        let value = a.value.hypot(b.value);
        if value == 0.0 {
            // Every direction is a subgradient's; take the one with no slope.
            return Dual::constant(0.0);
        }
        Dual {
            value,
            gradient: core::array::from_fn(|i| {
                (a.value * a.gradient[i] + b.value * b.gradient[i]) / value
            }),
        }
    }

    fn dual_sample<F: ScalarField + ?Sized>(field: &F, p: [Dual<Self>; 3]) -> Dual<Self> {
        let mut row = [Dual::constant(0.0)];
        field.eval_gradients(&[p.map(|c| c.value)], &mut row);
        Dual::through(row[0].value, row[0].gradient, &p)
    }

    fn dual_smooth_union(values: &[Dual<Self>], radius: f64) -> Dual<Self> {
        let (value, gradient) = blend::blend(values.iter().map(|v| (v.value, v.gradient)), radius);
        Dual { value, gradient }
    }
}

impl DualBranch for Interval {
    fn dual_min(a: Dual<Self>, b: Dual<Self>) -> Dual<Self> {
        if a.value.high < b.value.low {
            a
        } else if b.value.high < a.value.low {
            b
        } else {
            either(a.value.min(b.value), a, b)
        }
    }

    fn dual_max(a: Dual<Self>, b: Dual<Self>) -> Dual<Self> {
        if a.value.low > b.value.high {
            a
        } else if b.value.low > a.value.high {
            b
        } else {
            either(a.value.max(b.value), a, b)
        }
    }

    fn dual_abs(a: Dual<Self>) -> Dual<Self> {
        if a.value.low > 0.0 {
            a
        } else if a.value.high < 0.0 {
            -a
        } else {
            either(a.value.abs(), a, -a)
        }
    }

    fn dual_select(condition: Dual<Self>, below: Dual<Self>, above: Dual<Self>) -> Dual<Self> {
        if condition.value.high < 0.0 {
            below
        } else if condition.value.low >= 0.0 {
            above
        } else {
            either(below.value.hull(above.value), below, above)
        }
    }

    fn dual_hypot(a: Dual<Self>, b: Dual<Self>) -> Dual<Self> {
        let value = a.value.hypot(b.value);
        // Each partial derivative, `a / |(a, b)|`, lies in [-1, 1].
        let unit = Self::new(-1.0, 1.0);
        let partial = |side: Self| (side / value).intersect(unit).unwrap_or(unit);
        let (da, db) = (partial(a.value), partial(b.value));
        Dual {
            value,
            gradient: core::array::from_fn(|i| da * a.gradient[i] + db * b.gradient[i]),
        }
    }

    fn dual_sample<F: ScalarField + ?Sized>(field: &F, p: [Dual<Self>; 3]) -> Dual<Self> {
        let Some(region) = region(p.map(|c| c.value)) else {
            return Dual {
                value: Self::ENTIRE,
                gradient: [Self::ENTIRE; 3],
            };
        };
        let bounds = field.eval_gradient_interval(&region);
        Dual::through(bounds.value, bounds.gradient, &p)
    }

    fn dual_smooth_union(values: &[Dual<Self>], radius: f64) -> Dual<Self> {
        blend::blend_dual_interval(values, radius)
    }
}

/// `value` with a gradient bounding both `a`'s and `b`'s.
fn either(value: Interval, a: Dual<Interval>, b: Dual<Interval>) -> Dual<Interval> {
    Dual {
        value,
        gradient: core::array::from_fn(|i| a.gradient[i].hull(b.gradient[i])),
    }
}

/// The two creases nearest a point among those a [`Creased`] evaluation
/// passed through, nearest first, each a function zero where its two sides
/// meet, with how far away it counts as.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct Ties([Option<(Dual, f64)>; 2]);

impl Ties {
    /// The ties, nearest first.
    pub(crate) fn ties(self) -> [Option<Tie>; 2] {
        self.0
            .map(|tie| tie.map(|(function, distance)| Tie { function, distance }))
    }

    /// These ties with `tie` among them, if it is one of the two nearest.
    fn with(self, tie: Dual) -> Self {
        self.ranked(tie, reach(&tie))
    }

    /// These ties with `tie`, counted as `distance` away, among them if it
    /// is one of the two nearest.
    fn ranked(self, tie: Dual, distance: f64) -> Self {
        let [first, second] = self.0;
        if first.is_some_and(|f| f.0 == tie) || second.is_some_and(|s| s.0 == tie) {
            return self;
        }
        let entry = Some((tie, distance));
        match (first, second) {
            (Some(first), _) if distance < first.1 => Self([entry, Some(first)]),
            (Some(first), Some(second)) if distance < second.1 => Self([Some(first), entry]),
            (Some(first), None) => Self([Some(first), entry]),
            (None, _) => Self([entry, None]),
            _ => self,
        }
    }

    /// The two nearest of both sets of ties.
    fn merged(self, other: Self) -> Self {
        other
            .0
            .into_iter()
            .flatten()
            .fold(self, |ties, (tie, distance)| ties.ranked(tie, distance))
    }
}

/// A tie's first-order distance, `|value| / |gradient|`; infinite when it
/// has no slope.
fn reach(tie: &Dual) -> f64 {
    let slope = Real::sqrt(tie.gradient.iter().map(|g| g * g).sum::<f64>());
    let reach = Real::abs(tie.value) / slope;
    if reach.is_nan() { f64::INFINITY } else { reach }
}

/// A dual that also carries the [`Ties`] nearest the point among the creases
/// of the `min`, `max` and `abs` it passed through, and the region of the
/// tagged part that shapes it.
///
/// A side a `min` or `max` does not choose shapes the field only beyond the
/// tie between the two, so its own ties count as no nearer than that tie:
/// where three faces meet at a corner, the two creases nearest it are both
/// found, whichever face the point lies on. The side a `select` does not
/// choose takes its ties with it.
///
/// The region follows the side `min`, `max` and `select` choose and the
/// least part of a smooth union; arithmetic on two numbers keeps the left
/// one's, untagged or not.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Creased {
    pub(crate) value: Dual,
    pub(crate) ties: Ties,
    pub(crate) region: u32,
}

impl Creased {
    pub(crate) fn point(point: [f64; 3]) -> [Self; 3] {
        Dual::point(point).map(|value| Self {
            value,
            ties: Ties::default(),
            region: 0,
        })
    }

    fn with(self, value: Dual) -> Self {
        Self { value, ..self }
    }

    fn joined(self, other: Self, value: Dual) -> Self {
        Self {
            value,
            ties: self.ties.merged(other.ties),
            region: self.region,
        }
    }

    /// The side `choose` picks of `self` and `other`, tied where they meet.
    fn kink(self, other: Self, choose: fn(Dual, Dual) -> Dual) -> Self {
        let value = choose(self.value, other.value);
        let (kept, lost) = if value == self.value {
            (self, other)
        } else {
            (other, self)
        };
        let tie = self.value - other.value;
        let beyond = reach(&tie);
        let ties = lost
            .ties
            .0
            .into_iter()
            .flatten()
            .fold(kept.ties.ranked(tie, beyond), |ties, (dual, distance)| {
                ties.ranked(dual, distance.max(beyond))
            });
        Self {
            value,
            ties,
            region: kept.region,
        }
    }
}

impl Add for Creased {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        self.joined(other, self.value + other.value)
    }
}

impl Sub for Creased {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        self.joined(other, self.value - other.value)
    }
}

impl Mul for Creased {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        self.joined(other, self.value * other.value)
    }
}

impl Div for Creased {
    type Output = Self;

    fn div(self, other: Self) -> Self {
        self.joined(other, self.value / other.value)
    }
}

impl Neg for Creased {
    type Output = Self;

    fn neg(self) -> Self {
        self.with(-self.value)
    }
}

impl Add<f64> for Creased {
    type Output = Self;

    fn add(self, other: f64) -> Self {
        self.with(self.value + other)
    }
}

impl Sub<f64> for Creased {
    type Output = Self;

    fn sub(self, other: f64) -> Self {
        self.with(self.value - other)
    }
}

impl Mul<f64> for Creased {
    type Output = Self;

    fn mul(self, other: f64) -> Self {
        self.with(self.value * other)
    }
}

impl Div<f64> for Creased {
    type Output = Self;

    fn div(self, other: f64) -> Self {
        self.with(self.value / other)
    }
}

impl sealed::Sealed for Creased {
    fn sample<F: ScalarField + ?Sized>(field: &F, p: [Self; 3]) -> Self {
        let coordinates = p.map(|c| c.value);
        let crease = field.eval_crease(coordinates.map(|c| c.value));
        let through = |row: Dual| Dual::through(row.value, row.gradient, &coordinates);
        let upstream = p
            .iter()
            .fold(Ties::default(), |ties, c| ties.merged(c.ties));
        let ties = crease
            .ties
            .into_iter()
            .flatten()
            .fold(upstream, |ties, tie| {
                let function = through(tie.function);
                ties.ranked(function, reach(&function).max(tie.distance))
            });
        Self {
            value: through(crease.field),
            ties,
            region: crease.region,
        }
    }

    fn span(self) -> Interval {
        self.value.value.span()
    }

    /// The blend of the values, keeping the ties of every part in the band,
    /// which still shape the field there, and the least part's region.
    fn smooth_union(values: &[Self], radius: f64) -> Self {
        let minimum = blend::least(values.iter().map(|v| v.value.value));
        let (value, gradient) = blend::blend(
            values.iter().map(|v| (v.value.value, v.value.gradient)),
            radius,
        );
        let ties = values
            .iter()
            .filter(|v| blend::in_band(v.value.value, minimum, radius))
            .fold(Ties::default(), |ties, v| ties.merged(v.ties));
        let region = values
            .iter()
            .find(|v| v.value.value == minimum)
            .map_or(0, |v| v.region);
        Self {
            value: Dual { value, gradient },
            ties,
            region,
        }
    }

    fn tagged(self, region: u32) -> Self {
        Self { region, ..self }
    }
}

impl Number for Creased {
    fn constant(value: f64) -> Self {
        Self {
            value: Dual::constant(value),
            ties: Ties::default(),
            region: 0,
        }
    }

    fn abs(self) -> Self {
        Self {
            value: self.value.abs(),
            ties: self.ties.with(self.value),
            region: self.region,
        }
    }

    fn sqrt(self) -> Self {
        self.with(self.value.sqrt())
    }

    fn sin_cos(self) -> (Self, Self) {
        let (sin, cos) = self.value.sin_cos();
        (self.with(sin), self.with(cos))
    }

    fn min(self, other: Self) -> Self {
        self.kink(other, Number::min)
    }

    fn max(self, other: Self) -> Self {
        self.kink(other, Number::max)
    }

    fn hypot(self, other: Self) -> Self {
        self.joined(other, self.value.hypot(other.value))
    }

    fn select(self, below: Self, above: Self) -> Self {
        if self.value.value < 0.0 { below } else { above }
    }

    fn square(self) -> Self {
        self.with(self.value.square())
    }
}

#[cfg(test)]
mod tests {
    use super::{Creased, Dual, Interval, Number};

    fn gyroid<N: Number>([x, y, z]: [N; 3]) -> N {
        x.sin() * y.cos() + y.sin() * z.cos() + z.sin() * x.cos()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-15
    }

    #[test]
    fn duals_carry_exact_gradients() {
        let p = [0.3, -1.2, 2.5_f64];
        let dual = gyroid(Dual::point(p));
        assert_eq!(dual.value, gyroid(p));
        let [x, y, z] = p;
        let expected = [
            x.cos() * y.cos() - z.sin() * x.sin(),
            -x.sin() * y.sin() + y.cos() * z.cos(),
            -y.sin() * z.sin() + z.cos() * x.cos(),
        ];
        for (component, expected) in dual.gradient.into_iter().zip(expected) {
            assert!(close(component, expected), "{component} against {expected}");
        }
    }

    #[test]
    fn quotients_and_roots_follow_the_chain_rule() {
        let [x, y, _] = Dual::point([3.0, 4.0, 0.0]);
        let radius = (x.square() + y.square()).sqrt();
        assert_eq!(radius.value, 5.0);
        for (component, expected) in radius.gradient.into_iter().zip([0.6, 0.8, 0.0]) {
            assert!(close(component, expected), "{component} against {expected}");
        }
        let length = x.hypot(y);
        assert_eq!(length.value, 5.0);
        for (component, expected) in length.gradient.into_iter().zip([0.6, 0.8, 0.0]) {
            assert!(close(component, expected), "{component} against {expected}");
        }
        let ratio = x / y;
        assert_eq!(ratio.gradient, [0.25, -3.0 / 16.0, 0.0]);
    }

    #[test]
    fn min_and_max_take_the_chosen_operand_gradient() {
        let [x, y, _] = Dual::point([1.0, 2.0, 0.0]);
        assert_eq!(x.max(y).gradient, [0.0, 1.0, 0.0]);
        assert_eq!(x.min(y).gradient, [1.0, 0.0, 0.0]);
        assert_eq!((-x).abs().gradient, [1.0, 0.0, 0.0]);
        assert_eq!((x - 2.0).select(x, y).gradient, [1.0, 0.0, 0.0]);
        assert_eq!((x - 1.0).select(x, y).gradient, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn lengths_of_zero_have_no_slope() {
        // The outside part of a box is zero throughout its inside.
        let [x, y, _] = Dual::point([0.2, -0.1, 0.0]);
        let zero = Dual::constant(0.0);
        let outside = (x - 0.5).max(zero).hypot((y - 0.5).max(zero));
        assert_eq!(outside, Dual::constant(0.0));
        let bounds = {
            let [x, y, _] = Dual::point([
                Interval::new(-0.1, 0.6),
                Interval::new(0.0, 0.1),
                Interval::constant(0.0),
            ]);
            let zero = Dual::constant(0.0);
            (x - 0.5).max(zero).hypot((y - 0.5).max(zero))
        };
        assert!(bounds.value.contains(0.0) && bounds.value.contains(0.1));
        assert!(bounds.gradient[0].contains(1.0) && bounds.gradient[0].contains(0.0));
        let slope = bounds.gradient[0];
        assert!(slope.high() <= 1.0 + 1e-15 && slope.low() >= -1.0 - 1e-15);
    }

    /// Points spread through a box, corners and interior.
    fn samples(low: [f64; 3], high: [f64; 3]) -> impl Iterator<Item = [f64; 3]> {
        (0..7_u32.pow(3)).map(move |index| {
            let step = [index % 7, index / 7 % 7, index / 49].map(f64::from);
            core::array::from_fn(|i| low[i] + (high[i] - low[i]) * step[i] / 6.0)
        })
    }

    fn rimmed_sheet<N: Number>([x, y, z]: [N; 3]) -> N {
        let sheet = (x * 7.0).sin() * (y * 7.0).cos() / 7.0 - z.square().sqrt();
        let rim = x.hypot(y) - 0.9;
        let cap = (z - 0.2).select(z.hypot(x) - 0.2, z - 0.2 + x.abs());
        (sheet.abs() - 0.05).max(rim).min(cap)
    }

    #[test]
    fn intervals_bound_every_sampled_value_and_gradient() {
        for (low, high) in [
            ([0.1, -0.3, 0.2], [0.15, -0.25, 0.26]),
            ([-0.7, 0.4, -0.1], [-0.2, 0.9, 0.3]),
            ([0.0, 0.0, 0.0], [1.5, 1.5, 1.5]),
        ] {
            let bounds = rimmed_sheet(Dual::point(core::array::from_fn(|i| {
                Interval::new(low[i], high[i])
            })));
            for p in samples(low, high) {
                let sample = rimmed_sheet(Dual::point(p));
                assert!(bounds.value.contains(sample.value), "{p:?}");
                // Square roots of zero have no gradient to bound.
                for axis in (0..3).filter(|&axis| sample.gradient[axis].is_finite()) {
                    assert!(
                        bounds.gradient[axis].contains(sample.gradient[axis]),
                        "{p:?} axis {axis}"
                    );
                }
            }
        }
    }

    #[test]
    fn lengths_keep_their_scale() {
        let near = |a: f64, b: f64| (a - b).abs() <= 4.0 * f64::EPSILON * b.abs();
        assert!(near(Number::hypot(3.0e-200_f64, 4.0e-200), 5.0e-200));
        assert!(near(Number::hypot(3.0e200_f64, -4.0e200), 5.0e200));
        assert_eq!(Number::hypot(0.0_f64, -0.0), 0.0);
        assert!(Number::hypot(f64::INFINITY, f64::NAN).is_nan());
    }

    #[test]
    fn interval_sines_reach_their_peaks() {
        let (sin, cos) = Interval::new(1.0, 2.0).sin_cos();
        assert_eq!(sin.high(), 1.0);
        assert!(sin.low() <= 1.0_f64.sin());
        assert!(cos.low() <= 2.0_f64.cos() && cos.high() >= 1.0_f64.cos());
        assert_eq!(Interval::new(-0.1, 0.1).cos().high(), 1.0);
        assert_eq!(Interval::new(3.0, 3.3).cos().low(), -1.0);
        assert_eq!(Interval::new(0.0, 7.0).sin(), Interval::new(-1.0, 1.0));
    }

    #[test]
    fn interval_sines_far_out_stay_sound() {
        for x in [1.0e20, -1.0e20, 1.0e15, 3.0e9] {
            let (sin, cos) = Interval::new(x, x).sin_cos();
            assert!(sin.contains(x.sin()) && cos.contains(x.cos()), "{x}");
        }
    }

    #[test]
    fn interval_squares_and_quotients_stay_sound() {
        assert_eq!(Interval::new(-2.0, 1.0).square().low(), 0.0);
        assert_eq!(
            Interval::new(1.0, 2.0) / Interval::new(-1.0, 1.0),
            Interval::ENTIRE
        );
        assert!((Interval::new(1.0, 2.0) * Interval::ENTIRE).contains(0.0));
        let picked =
            Interval::new(-1.0, 1.0).select(Interval::constant(2.0), Interval::constant(5.0));
        assert_eq!(picked, Interval::new(2.0, 5.0));
    }

    /// A slab cut by a ball and a half space: the rim where the slab's face
    /// meets the ball, and a corner where the half space meets both.
    fn cut_slab<N: Number>([x, y, z]: [N; 3]) -> N {
        let slab = z.abs() - 0.1;
        let ball = (x.square() + y.square() + z.square()).sqrt() - 1.0;
        slab.max(ball).max(y - 0.3)
    }

    #[test]
    fn creases_follow_the_chosen_side() {
        // Near the rim, the tie is the slab's face against the ball; deep
        // inside, the slab's mid-plane, from `abs`.
        let rim = cut_slab(Creased::point([0.99, 0.0, 0.1])).ties.ties()[0]
            .unwrap()
            .function;
        // |z| - 0.1 - (r - 1) at the rim.
        assert!(rim.value.abs() < 0.02);
        assert!(rim.gradient[0] < 0.0 && rim.gradient[2] > 0.0);
        let middle = cut_slab(Creased::point([0.0, 0.0, 0.01])).ties.ties()[0]
            .unwrap()
            .function;
        assert_eq!(middle.value, 0.01);
        assert_eq!(middle.gradient, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn corners_keep_both_ties() {
        // Where the rim meets the half space, the two nearest ties are the
        // rim's and the half space's against the slab and ball.
        let corner = [(1.0_f64 - 0.09 - 0.01).sqrt(), 0.3, 0.1];
        let [first, second] = cut_slab(Creased::point(corner)).ties.ties();
        let (first, second) = (first.unwrap().function, second.unwrap().function);
        assert!(first.value.abs() < 1e-12 && second.value.abs() < 1e-12);
        assert_ne!(first.gradient, second.gradient);
    }

    #[test]
    fn selects_mark_no_crease() {
        let [x, _, _] = Creased::point([0.5, 0.0, 0.0]);
        let joined = x.select(x.square(), x * 2.0);
        assert_eq!(joined.ties.ties(), [None, None]);
        assert_eq!(joined.value.gradient, [2.0, 0.0, 0.0]);
    }
}
