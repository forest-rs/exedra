// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact positive lengths and signed linear displacements.

use core::num::NonZeroU64;

use joto_constants::length::{i64 as signed_iota, u64 as unsigned_iota};

/// An exact, strictly positive physical size stored in joto iotas.
///
/// The type excludes zero as well as every invalid floating-point state. Use
/// [`Offset`] for signed differences or coordinates.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Length(NonZeroU64);

impl Length {
    /// Creates a length from an exact iota count.
    ///
    /// Returns `None` when `value` is zero.
    #[must_use]
    pub const fn from_iota(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Creates an exact whole-millimeter length.
    ///
    /// Returns `None` for zero or when conversion to iotas overflows.
    #[must_use]
    pub const fn millimeters(value: u64) -> Option<Self> {
        match value.checked_mul(unsigned_iota::MILLIMETER) {
            Some(value) => Self::from_iota(value),
            None => None,
        }
    }

    /// Creates an exact positive whole-micrometer length.
    ///
    /// Returns `None` for zero or when conversion to iotas overflows.
    #[must_use]
    pub const fn micrometers(value: u64) -> Option<Self> {
        match value.checked_mul(unsigned_iota::MICROMETER) {
            Some(value) => Self::from_iota(value),
            None => None,
        }
    }

    /// Creates an exact whole-meter length.
    ///
    /// Returns `None` for zero or when conversion to iotas overflows.
    #[must_use]
    pub const fn meters(value: u64) -> Option<Self> {
        match value.checked_mul(unsigned_iota::METER) {
            Some(value) => Self::from_iota(value),
            None => None,
        }
    }

    /// Returns the exact underlying iota count.
    #[must_use]
    pub const fn iota(self) -> u64 {
        self.0.get()
    }

    /// Lowers this exact length to meters at a floating-point geometry boundary.
    #[must_use]
    pub fn as_meters(self) -> f64 {
        self.iota() as f64 / unsigned_iota::METER as f64
    }

    /// Adds two lengths without overflowing the exact representation.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.iota().checked_add(other.iota()) {
            Some(value) => Self::from_iota(value),
            None => None,
        }
    }

    /// Scales a length by a positive integer without overflowing.
    ///
    /// Returns `None` when `factor` is zero because zero is not a [`Length`].
    #[must_use]
    pub const fn checked_mul(self, factor: u64) -> Option<Self> {
        match self.iota().checked_mul(factor) {
            Some(value) => Self::from_iota(value),
            None => None,
        }
    }

    /// Converts this length to a positive signed [`Offset`] when representable.
    #[must_use]
    pub const fn checked_offset(self) -> Option<Offset> {
        if self.iota() <= i64::MAX.cast_unsigned() {
            Some(Offset::from_iota(self.iota().cast_signed()))
        } else {
            None
        }
    }
}

/// An exact signed displacement stored in joto iotas.
///
/// Offsets describe coordinates, translations, and differences. Use
/// [`Length`] when the value is intrinsically a positive physical size.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Offset(i64);

impl Offset {
    /// The zero displacement.
    pub const ZERO: Self = Self(0);

    /// Creates an offset from an exact signed iota count.
    #[must_use]
    pub const fn from_iota(value: i64) -> Self {
        Self(value)
    }

    /// Creates an exact whole-millimeter offset.
    ///
    /// Returns `None` when conversion to iotas overflows.
    #[must_use]
    pub const fn millimeters(value: i64) -> Option<Self> {
        match value.checked_mul(signed_iota::MILLIMETER) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Creates an exact whole-micrometer offset.
    ///
    /// Returns `None` when conversion to iotas overflows.
    #[must_use]
    pub const fn micrometers(value: i64) -> Option<Self> {
        match value.checked_mul(signed_iota::MICROMETER) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Creates an exact whole-meter offset.
    ///
    /// Returns `None` when conversion to iotas overflows.
    #[must_use]
    pub const fn meters(value: i64) -> Option<Self> {
        match value.checked_mul(signed_iota::METER) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the exact underlying signed iota count.
    #[must_use]
    pub const fn iota(self) -> i64 {
        self.0
    }

    /// Lowers this exact offset to meters at a floating-point geometry boundary.
    #[must_use]
    pub fn as_meters(self) -> f64 {
        self.iota() as f64 / signed_iota::METER as f64
    }

    /// Adds two offsets without overflowing the exact representation.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.iota().checked_add(other.iota()) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Subtracts two offsets without overflowing the exact representation.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.iota().checked_sub(other.iota()) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Negates this offset without overflowing the exact representation.
    #[must_use]
    pub const fn checked_neg(self) -> Option<Self> {
        match self.iota().checked_neg() {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns this offset as a [`Length`] when it is strictly positive.
    #[must_use]
    pub const fn positive_length(self) -> Option<Length> {
        if self.iota() > 0 {
            Length::from_iota(self.iota() as u64)
        } else {
            None
        }
    }

    /// Returns the nonzero magnitude as a [`Length`].
    ///
    /// Zero has no length under the crate's strictly positive definition.
    #[must_use]
    pub const fn magnitude(self) -> Option<Length> {
        Length::from_iota(self.iota().unsigned_abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_excludes_non_positive_and_overflowed_sizes() {
        // A physical size cannot carry the invalid states that formerly
        // required finite-and-positive checks at each geometry call site.
        assert_eq!(Length::from_iota(0), None);
        assert_eq!(Length::millimeters(0), None);
        assert_eq!(
            Length::millimeters(u64::MAX / unsigned_iota::MILLIMETER + 1),
            None
        );
    }

    #[test]
    fn exact_values_lower_only_at_the_geometry_boundary() {
        // Construction and arithmetic remain integral until a caller asks for
        // the floating-point meters consumed by a geometry kernel.
        let length = Length::millimeters(25).unwrap();
        let offset = Offset::millimeters(-25).unwrap();

        assert_eq!(length.iota(), 25 * unsigned_iota::MILLIMETER);
        assert_eq!(offset.iota(), -25 * signed_iota::MILLIMETER);
        assert_eq!(length.as_meters(), 0.025);
        assert_eq!(offset.as_meters(), -0.025);
    }

    #[test]
    fn length_and_offset_conversions_preserve_their_domains() {
        // Only positive offsets are sizes, while magnitude intentionally turns
        // either displacement direction into a positive physical length.
        let positive = Offset::millimeters(25).unwrap();
        let negative = Offset::millimeters(-25).unwrap();

        assert_eq!(positive.positive_length(), Length::millimeters(25));
        assert_eq!(negative.positive_length(), None);
        assert_eq!(negative.magnitude(), Length::millimeters(25));
        assert_eq!(Offset::ZERO.magnitude(), None);
        // The signed minimum still has a representable unsigned magnitude,
        // while conversion in the other direction stops at signed maximum.
        assert_eq!(
            Offset::from_iota(i64::MIN).magnitude(),
            Length::from_iota(1_u64 << 63)
        );
        assert_eq!(
            Length::from_iota(i64::MAX.cast_unsigned())
                .unwrap()
                .checked_offset(),
            Some(Offset::from_iota(i64::MAX))
        );
        assert_eq!(
            Length::from_iota(i64::MAX.cast_unsigned() + 1)
                .unwrap()
                .checked_offset(),
            None
        );
    }

    #[test]
    fn checked_arithmetic_rejects_values_outside_the_target_domain() {
        // Overflow and multiplication by zero cannot manufacture an invalid
        // Length, and the most-negative Offset cannot be negated in i64.
        let one = Length::from_iota(1).unwrap();
        let largest = Length::from_iota(u64::MAX).unwrap();

        assert_eq!(largest.checked_add(one), None);
        assert_eq!(one.checked_mul(0), None);
        assert_eq!(Offset::from_iota(i64::MIN).checked_neg(), None);
    }
}
