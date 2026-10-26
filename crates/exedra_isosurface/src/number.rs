// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Numbers a field formula is evaluated in.
//!
//! A [`Formula`](crate::function::Formula) is written once, generically over
//! [`Number`], and evaluated in whichever number answers the question at
//! hand: `f64` for values, [`Dual`] for values with exact gradients,
//! [`Interval`] for bounds over a box, and `Dual<Interval>` for bounds on
//! the gradient over a box.
//!
//! `Number` has no comparisons, so a formula cannot branch on a value. Use
//! [`Number::min`], [`Number::max`] and [`Number::abs`] instead: a formula
//! without branches stays meaningful for numbers that stand for many values
//! at once. Constants go on the right of an operator, as in `x * 7.0`, or
//! through [`Number::constant`].

use core::f64::consts::{FRAC_PI_2, PI, TAU};
use core::ops::{Add, Div, Mul, Neg, Sub};

use exedra_math::Real;

/// Sealing module: nameable so the `Number` bound is expressible, hidden so
/// it is not part of the documented surface.
#[doc(hidden)]
pub mod sealed {
    use super::Dual;

    /// Implemented for exactly the numbers in this module.
    pub trait Sealed {}
    impl Sealed for f64 {}
    impl Sealed for super::Interval {}
    impl Sealed for super::Creased {}
    impl<T: DualBranch> Sealed for Dual<T> {}

    /// How a dual over this number chooses at the kinks of `min`, `max`
    /// and `abs`.
    pub trait DualBranch: Sized {
        /// The lesser operand, with a gradient valid wherever it is chosen.
        fn dual_min(a: Dual<Self>, b: Dual<Self>) -> Dual<Self>;
        /// The greater operand, with a gradient valid wherever it is chosen.
        fn dual_max(a: Dual<Self>, b: Dual<Self>) -> Dual<Self>;
        /// The absolute value, with a gradient valid on either side.
        fn dual_abs(a: Dual<Self>) -> Dual<Self>;
    }
}

use sealed::DualBranch;

/// A number a [`Formula`](crate::function::Formula) can be evaluated in.
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

    /// The absolute value.
    #[must_use]
    fn abs(self) -> Self;

    /// The square root.
    #[must_use]
    fn sqrt(self) -> Self;

    /// The sine and cosine together.
    fn sin_cos(self) -> (Self, Self);

    /// The lesser of two numbers.
    #[must_use]
    fn min(self, other: Self) -> Self;

    /// The greater of two numbers.
    #[must_use]
    fn max(self, other: Self) -> Self;

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
        #[expect(
            clippy::cast_possible_truncation,
            reason = "finite intervals narrower than a period near the origin"
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
#[derive(Copy, Clone, Debug, PartialEq)]
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
}

/// `value` with a gradient bounding both `a`'s and `b`'s.
fn either(value: Interval, a: Dual<Interval>, b: Dual<Interval>) -> Dual<Interval> {
    Dual {
        value,
        gradient: core::array::from_fn(|i| a.gradient[i].hull(b.gradient[i])),
    }
}

/// A dual that also carries the nearest tie between the two sides of a
/// `min`, `max` or `abs` it passed through: a function, with its gradient,
/// that is zero where they meet. Where the field is zero too, the surface
/// has a crease.
///
/// A side a `min` or `max` does not choose takes its own ties with it, since
/// they do not shape the field there.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Creased {
    pub(crate) value: Dual,
    pub(crate) tie: Option<Dual>,
}

impl Creased {
    pub(crate) fn point(point: [f64; 3]) -> [Self; 3] {
        Dual::point(point).map(|value| Self { value, tie: None })
    }

    fn with(self, value: Dual) -> Self {
        Self {
            value,
            tie: self.tie,
        }
    }

    fn joined(self, other: Self, value: Dual) -> Self {
        Self {
            value,
            tie: nearer(self.tie, other.tie),
        }
    }

    /// The side `choose` picks of `self` and `other`, tied where they meet.
    fn kink(self, other: Self, choose: fn(Dual, Dual) -> Dual) -> Self {
        let value = choose(self.value, other.value);
        let kept = if value == self.value { self } else { other };
        Self {
            value,
            tie: nearer(kept.tie, Some(self.value - other.value)),
        }
    }
}

/// The tie nearer by its first-order distance, `|value| / |gradient|`.
fn nearer(a: Option<Dual>, b: Option<Dual>) -> Option<Dual> {
    let distance = |tie: &Dual| {
        let slope = Real::sqrt(tie.gradient.iter().map(|g| g * g).sum::<f64>());
        Real::abs(tie.value) / slope
    };
    match (a, b) {
        (Some(a), Some(b)) => Some(if distance(&b) < distance(&a) { b } else { a }),
        (a, b) => a.or(b),
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

impl Number for Creased {
    fn constant(value: f64) -> Self {
        Self {
            value: Dual::constant(value),
            tie: None,
        }
    }

    fn abs(self) -> Self {
        Self {
            value: self.value.abs(),
            tie: nearer(self.tie, Some(self.value)),
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

    fn square(self) -> Self {
        self.with(self.value.square())
    }
}

#[cfg(test)]
mod tests {
    use super::{Dual, Interval, Number};

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
            assert!(close(component, expected));
        }
    }

    #[test]
    fn quotients_and_roots_follow_the_chain_rule() {
        let [x, y, _] = Dual::point([3.0, 4.0, 0.0]);
        let radius = (x.square() + y.square()).sqrt();
        assert_eq!(radius.value, 5.0);
        for (component, expected) in radius.gradient.into_iter().zip([0.6, 0.8, 0.0]) {
            assert!(close(component, expected));
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
    }

    /// Points spread through a box, corners and interior.
    fn samples(low: [f64; 3], high: [f64; 3]) -> impl Iterator<Item = [f64; 3]> {
        (0..7_u32.pow(3)).map(move |index| {
            let step = [index % 7, index / 7 % 7, index / 49].map(f64::from);
            core::array::from_fn(|i| low[i] + (high[i] - low[i]) * step[i] / 6.0)
        })
    }

    #[test]
    fn intervals_bound_every_sampled_value_and_gradient() {
        let formula = |[x, y, z]: [Dual<Interval>; 3]| {
            let sheet = (x * 7.0).sin() * (y * 7.0).cos() / 7.0 - z.square().sqrt();
            (sheet.abs() - 0.05).max((x.square() + y.square()).sqrt() - 0.9)
        };
        let point = |[x, y, z]: [Dual<f64>; 3]| {
            let sheet = (x * 7.0).sin() * (y * 7.0).cos() / 7.0 - z.square().sqrt();
            (sheet.abs() - 0.05).max((x.square() + y.square()).sqrt() - 0.9)
        };
        for (low, high) in [
            ([0.1, -0.3, 0.2], [0.15, -0.25, 0.26]),
            ([-0.7, 0.4, -0.1], [-0.2, 0.9, 0.3]),
            ([0.0, 0.0, 0.0], [1.5, 1.5, 1.5]),
        ] {
            let bounds = formula(Dual::point(core::array::from_fn(|i| {
                Interval::new(low[i], high[i])
            })));
            for p in samples(low, high) {
                let sample = point(Dual::point(p));
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
    fn interval_squares_and_quotients_stay_sound() {
        assert_eq!(Interval::new(-2.0, 1.0).square().low(), 0.0);
        assert_eq!(
            Interval::new(1.0, 2.0) / Interval::new(-1.0, 1.0),
            Interval::ENTIRE
        );
        assert!((Interval::new(1.0, 2.0) * Interval::ENTIRE).contains(0.0));
    }

    #[test]
    fn creases_follow_the_chosen_side() {
        use super::Creased;
        // A slab cut by a ball: near the rim, the tie is the slab's face
        // against the ball; deep inside, the slab's mid-plane, from `abs`.
        let formula = |[x, y, z]: [Creased; 3]| {
            let slab = z.abs() - 0.1;
            let ball = (x.square() + y.square() + z.square()).sqrt() - 1.0;
            slab.max(ball)
        };
        let rim = formula(Creased::point([0.99, 0.0, 0.1])).tie.unwrap();
        // |z| - 0.1 - (r - 1) at the rim.
        assert!(rim.value.abs() < 0.02);
        assert!(rim.gradient[0] < 0.0 && rim.gradient[2] > 0.0);
        let middle = formula(Creased::point([0.0, 0.0, 0.01])).tie.unwrap();
        assert_eq!(middle.value, 0.01);
        assert_eq!(middle.gradient, [0.0, 0.0, 1.0]);
    }
}
