// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact angular magnitudes and signed displacements.

const MICROARCSECONDS_PER_ARCSECOND_U64: u64 = 1_000_000;
const MICROARCSECONDS_PER_ARCMINUTE_U64: u64 = 60 * MICROARCSECONDS_PER_ARCSECOND_U64;
const MICROARCSECONDS_PER_DEGREE_U64: u64 = 60 * MICROARCSECONDS_PER_ARCMINUTE_U64;
const MICROARCSECONDS_PER_ARCSECOND_I64: i64 = 1_000_000;
const MICROARCSECONDS_PER_ARCMINUTE_I64: i64 = 60 * MICROARCSECONDS_PER_ARCSECOND_I64;
const MICROARCSECONDS_PER_DEGREE_I64: i64 = 60 * MICROARCSECONDS_PER_ARCMINUTE_I64;

/// An exact, nonnegative angular magnitude stored in microarcseconds.
///
/// Angles are not implicitly normalized: one turn and two turns remain
/// distinguishable. Use [`AngularOffset`] when direction may be negative.
#[derive(Copy, Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Angle(u64);

impl Angle {
    /// The zero angle.
    pub const ZERO: Self = Self(0);

    /// Creates an angle from an exact microarcsecond count.
    #[must_use]
    pub const fn from_microarcseconds(value: u64) -> Self {
        Self(value)
    }

    /// Creates an exact whole-arcsecond angle.
    ///
    /// Returns `None` when conversion to microarcseconds overflows.
    #[must_use]
    pub const fn arcseconds(value: u64) -> Option<Self> {
        match value.checked_mul(MICROARCSECONDS_PER_ARCSECOND_U64) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Creates an exact whole-arcminute angle.
    ///
    /// Returns `None` when conversion to microarcseconds overflows.
    #[must_use]
    pub const fn arcminutes(value: u64) -> Option<Self> {
        match value.checked_mul(MICROARCSECONDS_PER_ARCMINUTE_U64) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Creates an exact whole-degree angle.
    ///
    /// Returns `None` when conversion to microarcseconds overflows.
    #[must_use]
    pub const fn degrees(value: u64) -> Option<Self> {
        match value.checked_mul(MICROARCSECONDS_PER_DEGREE_U64) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the exact underlying microarcsecond count.
    #[must_use]
    pub const fn microarcseconds(self) -> u64 {
        self.0
    }

    /// Lowers this exact angle to degrees at a floating-point geometry boundary.
    #[must_use]
    pub fn as_degrees(self) -> f64 {
        self.microarcseconds() as f64 / MICROARCSECONDS_PER_DEGREE_U64 as f64
    }

    /// Lowers this exact angle to radians at a floating-point geometry boundary.
    #[must_use]
    pub fn as_radians(self) -> f64 {
        self.as_degrees() * (core::f64::consts::PI / 180.0)
    }

    /// Adds two angles without overflowing the exact representation.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.microarcseconds().checked_add(other.microarcseconds()) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Scales an angle by an integer without overflowing.
    #[must_use]
    pub const fn checked_mul(self, factor: u64) -> Option<Self> {
        match self.microarcseconds().checked_mul(factor) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Converts this angle to a nonnegative [`AngularOffset`] when representable.
    #[must_use]
    pub const fn checked_offset(self) -> Option<AngularOffset> {
        if self.microarcseconds() <= i64::MAX.cast_unsigned() {
            Some(AngularOffset::from_microarcseconds(
                self.microarcseconds().cast_signed(),
            ))
        } else {
            None
        }
    }
}

/// An exact signed angular displacement stored in microarcseconds.
///
/// Angular offsets are not implicitly wrapped or normalized. Use [`Angle`]
/// for a nonnegative angular magnitude.
#[derive(Copy, Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AngularOffset(i64);

impl AngularOffset {
    /// The zero angular displacement.
    pub const ZERO: Self = Self(0);

    /// Creates an angular offset from an exact signed microarcsecond count.
    #[must_use]
    pub const fn from_microarcseconds(value: i64) -> Self {
        Self(value)
    }

    /// Creates an exact whole-arcsecond angular offset.
    ///
    /// Returns `None` when conversion to microarcseconds overflows.
    #[must_use]
    pub const fn arcseconds(value: i64) -> Option<Self> {
        match value.checked_mul(MICROARCSECONDS_PER_ARCSECOND_I64) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Creates an exact whole-arcminute angular offset.
    ///
    /// Returns `None` when conversion to microarcseconds overflows.
    #[must_use]
    pub const fn arcminutes(value: i64) -> Option<Self> {
        match value.checked_mul(MICROARCSECONDS_PER_ARCMINUTE_I64) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Creates an exact whole-degree angular offset.
    ///
    /// Returns `None` when conversion to microarcseconds overflows.
    #[must_use]
    pub const fn degrees(value: i64) -> Option<Self> {
        match value.checked_mul(MICROARCSECONDS_PER_DEGREE_I64) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the exact underlying signed microarcsecond count.
    #[must_use]
    pub const fn microarcseconds(self) -> i64 {
        self.0
    }

    /// Lowers this exact offset to degrees at a floating-point geometry boundary.
    #[must_use]
    pub fn as_degrees(self) -> f64 {
        self.microarcseconds() as f64 / MICROARCSECONDS_PER_DEGREE_I64 as f64
    }

    /// Lowers this exact offset to radians at a floating-point geometry boundary.
    #[must_use]
    pub fn as_radians(self) -> f64 {
        self.as_degrees() * (core::f64::consts::PI / 180.0)
    }

    /// Adds two angular offsets without overflowing the exact representation.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.microarcseconds().checked_add(other.microarcseconds()) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Subtracts two angular offsets without overflowing the exact representation.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.microarcseconds().checked_sub(other.microarcseconds()) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Negates this angular offset without overflowing the exact representation.
    #[must_use]
    pub const fn checked_neg(self) -> Option<Self> {
        match self.microarcseconds().checked_neg() {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the nonnegative magnitude as an [`Angle`].
    #[must_use]
    pub const fn magnitude(self) -> Angle {
        Angle::from_microarcseconds(self.microarcseconds().unsigned_abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angular_units_are_exact_and_do_not_wrap() {
        // Degrees, arcminutes, and arcseconds share one exact integer basis;
        // accumulated rotations remain distinct instead of wrapping silently.
        assert_eq!(Angle::arcseconds(3_600), Angle::degrees(1));
        assert_eq!(Angle::arcminutes(60), Angle::degrees(1));
        assert_ne!(Angle::degrees(360), Angle::degrees(720));
        assert_eq!(
            Angle::degrees(180).unwrap().as_radians(),
            core::f64::consts::PI
        );
    }

    #[test]
    fn angle_and_angular_offset_preserve_direction_and_bounds() {
        // Signed angular displacement lowers with its direction intact, and
        // even the signed minimum has a representable unsigned magnitude.
        let clockwise = AngularOffset::degrees(-45).unwrap();
        assert_eq!(clockwise.as_degrees(), -45.0);
        assert_eq!(clockwise.magnitude(), Angle::degrees(45).unwrap());
        assert_eq!(
            AngularOffset::from_microarcseconds(i64::MIN).magnitude(),
            Angle::from_microarcseconds(1_u64 << 63)
        );
        assert_eq!(
            Angle::from_microarcseconds(i64::MAX.cast_unsigned()).checked_offset(),
            Some(AngularOffset::from_microarcseconds(i64::MAX))
        );
        assert_eq!(
            Angle::from_microarcseconds(i64::MAX.cast_unsigned() + 1).checked_offset(),
            None
        );
    }

    #[test]
    fn angle_arithmetic_reports_overflow() {
        // Exact angular arithmetic never saturates or wraps at either the
        // storage limit or a full turn.
        let one = Angle::from_microarcseconds(1);
        let largest = Angle::from_microarcseconds(u64::MAX);
        assert_eq!(largest.checked_add(one), None);
        assert_eq!(largest.checked_mul(2), None);
        assert_eq!(
            AngularOffset::from_microarcseconds(i64::MIN).checked_neg(),
            None
        );
    }
}
