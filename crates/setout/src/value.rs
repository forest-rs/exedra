// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact quantity domains.

use core::any::Any;
use core::fmt;
use core::num::NonZeroU128;
use core::str::FromStr;

use joto_constants::length::i64::{METER, MILLIMETER};

use crate::fingerprint::CanonicalEncoder;
use crate::key::{ChoiceDomainKey, ChoiceOptionKey};

/// A runtime tag for a statically typed quantity domain.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum DomainTag {
    /// Exact signed length in iota.
    Length,
    /// Exact unsigned count.
    Count,
    /// Reduced exact rational number.
    Rational,
    /// Boolean flag.
    Flag,
    /// Stable named choice.
    Choice,
    /// Exact three-dimensional point.
    Point3,
}

impl DomainTag {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Length => 0,
            Self::Count => 1,
            Self::Rational => 2,
            Self::Flag => 3,
            Self::Choice => 4,
            Self::Point3 => 5,
        }
    }
}

#[doc(hidden)]
pub mod private {
    pub trait Sealed {}
}

/// A closed set of values that can inhabit a typed setout quantity.
pub trait Domain: private::Sealed + Clone + fmt::Debug + Eq + 'static {
    /// Runtime domain tag retained by erased registries and fingerprints.
    const TAG: DomainTag;

    /// Appends the domain's canonical exact representation.
    #[doc(hidden)]
    fn encode(&self, encoder: &mut CanonicalEncoder);
}

/// Exact signed length stored in iota (one ninth of a nanometre).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Length(i64);

impl Length {
    /// Zero length.
    pub const ZERO: Self = Self(0);

    /// Creates a length directly from iota.
    #[must_use]
    pub const fn from_iota(value: i64) -> Self {
        Self(value)
    }

    /// Creates an exact whole-millimetre length.
    pub fn millimetres(value: i64) -> Result<Self, ArithmeticError> {
        value
            .checked_mul(MILLIMETER)
            .map(Self)
            .ok_or(ArithmeticError::Overflow)
    }

    /// Creates an exact whole-metre length.
    pub fn metres(value: i64) -> Result<Self, ArithmeticError> {
        value
            .checked_mul(METER)
            .map(Self)
            .ok_or(ArithmeticError::Overflow)
    }

    /// Quantizes a finite metre value to the nearest iota.
    ///
    /// This is an import boundary for legacy floating-point parameters. The
    /// returned [`RootQuantization`] makes any discarded fraction explicit;
    /// exact setout propagation never repeats this conversion downstream.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the finite rounded value is explicitly bounded to the i64 domain before conversion"
    )]
    pub fn quantize_metres(value: f64) -> Result<(Self, RootQuantization), ArithmeticError> {
        if !value.is_finite() {
            return Err(ArithmeticError::NonFinite);
        }
        let scaled = value * METER as f64;
        if scaled < i64::MIN as f64 || scaled > i64::MAX as f64 {
            return Err(ArithmeticError::Overflow);
        }
        let rounded = libm::round(scaled);
        let selected = rounded as i64;
        Ok((
            Self(selected),
            RootQuantization {
                source_bits: value.to_bits(),
                selected_iota: selected,
                error_iota_bits: (scaled - rounded).to_bits(),
            },
        ))
    }

    /// Returns the underlying signed iota count.
    #[must_use]
    pub const fn iota(self) -> i64 {
        self.0
    }

    /// Lowers the exact value to metres once at a geometry boundary.
    #[must_use]
    pub fn as_metres(self) -> f64 {
        self.0 as f64 / METER as f64
    }

    /// Checked exact addition.
    pub fn checked_add(self, other: Self) -> Result<Self, ArithmeticError> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(ArithmeticError::Overflow)
    }

    /// Checked exact subtraction.
    pub fn checked_sub(self, other: Self) -> Result<Self, ArithmeticError> {
        self.0
            .checked_sub(other.0)
            .map(Self)
            .ok_or(ArithmeticError::Overflow)
    }

    /// Checked negation.
    pub fn checked_neg(self) -> Result<Self, ArithmeticError> {
        self.0
            .checked_neg()
            .map(Self)
            .ok_or(ArithmeticError::Overflow)
    }

    /// Multiplies by a rational with the requested integral policy.
    pub fn checked_scale(
        self,
        factor: Rational,
        round: Round,
    ) -> Result<(Self, ExactnessTrace), ArithmeticError> {
        let exact = Rational::new(
            i128::from(self.0)
                .checked_mul(factor.numerator())
                .ok_or(ArithmeticError::Overflow)?,
            factor.denominator(),
        )?;
        let (selected, trace) = exact.quantize_i64(round)?;
        Ok((Self(selected), trace))
    }
}

impl fmt::Display for Length {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rendered = joto_format::length::i64::format_dim(
            self.0,
            joto_parse::length::Unit::Meter,
            joto_format::length::LengthFormat::new(),
        );
        formatter.write_str(rendered.as_str())
    }
}

impl FromStr for Length {
    type Err = ParseLengthError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        joto_parse::length::i64::parse_dim_diagnostic(value)
            .map(Self)
            .map_err(|_| ParseLengthError)
    }
}

/// A dimension string could not be parsed as an exact iota length.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ParseLengthError;

impl fmt::Display for ParseLengthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid or inexact length dimension")
    }
}

impl core::error::Error for ParseLengthError {}

/// Exact unsigned count.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Count(u64);

impl Count {
    /// Creates a count.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Reduced exact rational number with a positive denominator.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Rational {
    numerator: i128,
    denominator: NonZeroU128,
}

impl Rational {
    /// The additive identity.
    pub const ZERO: Self = Self {
        numerator: 0,
        denominator: NonZeroU128::MIN,
    };

    /// The multiplicative identity.
    pub const ONE: Self = Self {
        numerator: 1,
        denominator: NonZeroU128::MIN,
    };

    /// Creates and eagerly reduces a rational.
    pub fn new(numerator: i128, denominator: u128) -> Result<Self, ArithmeticError> {
        let denominator = NonZeroU128::new(denominator).ok_or(ArithmeticError::ZeroDenominator)?;
        if numerator == 0 {
            return Ok(Self::ZERO);
        }
        let divisor = gcd(numerator.unsigned_abs(), denominator.get());
        let reduced_numerator =
            numerator / i128::try_from(divisor).map_err(|_| ArithmeticError::Overflow)?;
        let reduced_denominator = denominator.get() / divisor;
        Ok(Self {
            numerator: reduced_numerator,
            denominator: NonZeroU128::new(reduced_denominator).expect("non-zero divided by gcd"),
        })
    }

    /// Returns the signed numerator.
    #[must_use]
    pub const fn numerator(self) -> i128 {
        self.numerator
    }

    /// Returns the positive denominator.
    #[must_use]
    pub const fn denominator(self) -> u128 {
        self.denominator.get()
    }

    /// Returns the reciprocal, preserving the sign in the numerator.
    pub fn checked_reciprocal(self) -> Result<Self, ArithmeticError> {
        if self.numerator == 0 {
            return Err(ArithmeticError::DivisionByZero);
        }
        let numerator = if self.numerator.is_negative() {
            -i128::try_from(self.denominator.get()).map_err(|_| ArithmeticError::Overflow)?
        } else {
            i128::try_from(self.denominator.get()).map_err(|_| ArithmeticError::Overflow)?
        };
        Self::new(numerator, self.numerator.unsigned_abs())
    }

    /// Checked multiplication with cross-cancellation before products.
    pub fn checked_mul(self, other: Self) -> Result<Self, ArithmeticError> {
        let left_cancel = gcd(self.numerator.unsigned_abs(), other.denominator());
        let right_cancel = gcd(other.numerator.unsigned_abs(), self.denominator());
        let left_num =
            self.numerator / i128::try_from(left_cancel).map_err(|_| ArithmeticError::Overflow)?;
        let right_num = other.numerator
            / i128::try_from(right_cancel).map_err(|_| ArithmeticError::Overflow)?;
        let left_den = self.denominator() / right_cancel;
        let right_den = other.denominator() / left_cancel;
        let numerator = left_num
            .checked_mul(right_num)
            .ok_or(ArithmeticError::Overflow)?;
        let denominator = left_den
            .checked_mul(right_den)
            .ok_or(ArithmeticError::Overflow)?;
        Self::new(numerator, denominator)
    }

    /// Checked division.
    pub fn checked_div(self, other: Self) -> Result<Self, ArithmeticError> {
        self.checked_mul(other.checked_reciprocal()?)
    }

    fn quantize_i64(self, round: Round) -> Result<(i64, ExactnessTrace), ArithmeticError> {
        let denominator =
            i128::try_from(self.denominator()).map_err(|_| ArithmeticError::Overflow)?;
        let quotient = self.numerator / denominator;
        let remainder = self.numerator % denominator;
        if remainder == 0 {
            return Ok((
                i64::try_from(quotient).map_err(|_| ArithmeticError::Overflow)?,
                ExactnessTrace::Exact,
            ));
        }
        let selected = match round {
            Round::Exact => return Err(ArithmeticError::NonIntegral { exact: self }),
            Round::Down => self.numerator.div_euclid(denominator),
            Round::Up => {
                let floor = self.numerator.div_euclid(denominator);
                floor.checked_add(1).ok_or(ArithmeticError::Overflow)?
            }
            Round::Nearest => {
                let floor = self.numerator.div_euclid(denominator);
                let floor_remainder = self.numerator.rem_euclid(denominator);
                let doubled = floor_remainder
                    .checked_mul(2)
                    .ok_or(ArithmeticError::Overflow)?;
                if doubled >= denominator {
                    floor.checked_add(1).ok_or(ArithmeticError::Overflow)?
                } else {
                    floor
                }
            }
        };
        let selected = i64::try_from(selected).map_err(|_| ArithmeticError::Overflow)?;
        Ok((
            selected,
            ExactnessTrace::RationalQuantization {
                exact: self,
                selected: i128::from(selected),
                policy: round,
            },
        ))
    }
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let next = left % right;
        left = right;
        right = next;
    }
    left
}

/// Exact boolean flag.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Flag(bool);

impl Flag {
    /// Creates a flag.
    #[must_use]
    pub const fn new(value: bool) -> Self {
        Self(value)
    }

    /// Returns the boolean value.
    #[must_use]
    pub const fn get(self) -> bool {
        self.0
    }
}

/// A selected stable option in a named choice domain.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ChoiceValue {
    /// Stable identity of the choice domain.
    pub domain: ChoiceDomainKey,
    /// Stable identity of the selected option.
    pub option: ChoiceOptionKey,
}

/// An exact point in a Z-up, metre-based coordinate system.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Point3 {
    /// X coordinate.
    pub x: Length,
    /// Y coordinate.
    pub y: Length,
    /// Z coordinate.
    pub z: Length,
}

impl Point3 {
    /// Creates a point from exact components.
    #[must_use]
    pub const fn new(x: Length, y: Length, z: Length) -> Self {
        Self { x, y, z }
    }
}

/// Policy for turning a non-integral exact result into an integer domain.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Round {
    /// Reject a non-integral result.
    Exact,
    /// Select the mathematical floor.
    Down,
    /// Select the mathematical ceiling.
    Up,
    /// Select the nearest integer, with exact halves away from negative infinity.
    Nearest,
}

impl Round {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Exact => 0,
            Self::Down => 1,
            Self::Up => 2,
            Self::Nearest => 3,
        }
    }
}

/// Exact description of any quantization performed by a relation or root import.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExactnessTrace {
    /// No information was discarded.
    Exact,
    /// An exact rational did not inhabit the integer target domain.
    RationalQuantization {
        /// Exact result before selection.
        exact: Rational,
        /// Selected integer value.
        selected: i128,
        /// Selection policy.
        policy: Round,
    },
    /// An integer square root was not exact.
    RootQuantization(RootRounding),
    /// A legacy floating root was quantized once on import.
    ImportedFloat(RootQuantization),
}

/// Certificate for an integer square-root result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootRounding {
    /// Exact squared radicand.
    pub radicand: u128,
    /// Integer floor of the root.
    pub floor_root: u128,
    /// `radicand - floor_root²`.
    pub remainder: u128,
    /// Selected integer root.
    pub selected_root: u128,
    /// Selection policy.
    pub policy: Round,
}

/// Audit record for a one-time floating-point root import.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RootQuantization {
    /// Original IEEE-754 bits.
    pub source_bits: u64,
    /// Selected exact iota count.
    pub selected_iota: i64,
    /// IEEE-754 bits of the discarded iota fraction.
    pub error_iota_bits: u64,
}

/// Failure of checked exact arithmetic.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ArithmeticError {
    /// Integer arithmetic overflowed its domain.
    Overflow,
    /// A rational denominator was zero.
    ZeroDenominator,
    /// Division by zero was requested.
    DivisionByZero,
    /// A floating import was NaN or infinite.
    NonFinite,
    /// Exact integer output was requested for a non-integral rational.
    NonIntegral {
        /// The exact result that could not inhabit the target domain.
        exact: Rational,
    },
    /// A square-root radicand was negative.
    NegativeRadicand,
}

impl fmt::Display for ArithmeticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overflow => formatter.write_str("exact arithmetic overflow"),
            Self::ZeroDenominator => formatter.write_str("rational denominator is zero"),
            Self::DivisionByZero => formatter.write_str("division by zero"),
            Self::NonFinite => formatter.write_str("floating root is not finite"),
            Self::NonIntegral { .. } => formatter.write_str("result is not integral"),
            Self::NegativeRadicand => formatter.write_str("square-root radicand is negative"),
        }
    }
}

impl core::error::Error for ArithmeticError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Value {
    Length(Length),
    Count(Count),
    Rational(Rational),
    Flag(Flag),
    Choice(ChoiceValue),
    Point3(Point3),
}

impl Value {
    pub(crate) fn from_domain<T: Domain>(value: T) -> Self {
        let value: &dyn Any = &value;
        match T::TAG {
            DomainTag::Length => Self::Length(
                *value
                    .downcast_ref::<Length>()
                    .expect("sealed Length domain matches its tag"),
            ),
            DomainTag::Count => Self::Count(
                *value
                    .downcast_ref::<Count>()
                    .expect("sealed Count domain matches its tag"),
            ),
            DomainTag::Rational => Self::Rational(
                *value
                    .downcast_ref::<Rational>()
                    .expect("sealed Rational domain matches its tag"),
            ),
            DomainTag::Flag => Self::Flag(
                *value
                    .downcast_ref::<Flag>()
                    .expect("sealed Flag domain matches its tag"),
            ),
            DomainTag::Choice => Self::Choice(
                value
                    .downcast_ref::<ChoiceValue>()
                    .expect("sealed Choice domain matches its tag")
                    .clone(),
            ),
            DomainTag::Point3 => Self::Point3(
                *value
                    .downcast_ref::<Point3>()
                    .expect("sealed Point3 domain matches its tag"),
            ),
        }
    }

    pub(crate) fn downcast<T: Domain>(&self) -> Option<&T> {
        let value: &dyn Any = match self {
            Self::Length(value) => value,
            Self::Count(value) => value,
            Self::Rational(value) => value,
            Self::Flag(value) => value,
            Self::Choice(value) => value,
            Self::Point3(value) => value,
        };
        value.downcast_ref()
    }

    pub(crate) fn tag(&self) -> DomainTag {
        match self {
            Self::Length(_) => DomainTag::Length,
            Self::Count(_) => DomainTag::Count,
            Self::Rational(_) => DomainTag::Rational,
            Self::Flag(_) => DomainTag::Flag,
            Self::Choice(_) => DomainTag::Choice,
            Self::Point3(_) => DomainTag::Point3,
        }
    }

    pub(crate) fn encode(&self, encoder: &mut CanonicalEncoder) {
        encoder.u8(self.tag().code());
        match self {
            Self::Length(value) => encoder.i64(value.iota()),
            Self::Count(value) => encoder.u64(value.get()),
            Self::Rational(value) => {
                encoder.i128(value.numerator());
                encoder.u128(value.denominator());
            }
            Self::Flag(value) => encoder.bool(value.get()),
            Self::Choice(value) => {
                encoder.str(value.domain.as_str());
                encoder.str(value.option.as_str());
            }
            Self::Point3(value) => {
                encoder.i64(value.x.iota());
                encoder.i64(value.y.iota());
                encoder.i64(value.z.iota());
            }
        }
    }
}

macro_rules! domain {
    ($type:ty, $tag:expr, $variant:ident) => {
        impl private::Sealed for $type {}

        impl Domain for $type {
            const TAG: DomainTag = $tag;

            fn encode(&self, encoder: &mut CanonicalEncoder) {
                Value::$variant(self.clone()).encode(encoder);
            }
        }
    };
}

domain!(Length, DomainTag::Length, Length);
domain!(Count, DomainTag::Count, Count);
domain!(Rational, DomainTag::Rational, Rational);
domain!(Flag, DomainTag::Flag, Flag);
domain!(ChoiceValue, DomainTag::Choice, Choice);
domain!(Point3, DomainTag::Point3, Point3);
