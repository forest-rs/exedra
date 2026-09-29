// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Section scale and twist laws along a sweep path.
//!
//! A [`SectionLaw`] varies a sweep's section with normalized arc length along
//! its path: `scale` multiplies the section about its datum (the point placed
//! on the path) and `twist` rotates it about the path tangent, in radians,
//! right-handed (section X toward section Y). The identity law reproduces a
//! constant-section sweep exactly.

use alloc::vec::Vec;

/// A scalar function of normalized arc length `t` in `[0, 1]`.
///
/// Equality compares values, so `-0.0` equals `0.0`; the canonical encoding
/// behind fingerprints also writes both as `+0.0`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Law {
    /// The same value everywhere.
    Constant(f64),
    /// Piecewise-linear interpolation of `[t, value]` keys.
    ///
    /// Keys are finite, at least two, with `t` strictly increasing from
    /// exactly `0.0` to exactly `1.0`.
    Linear(Vec<[f64; 2]>),
}

impl Law {
    /// Evaluates the law at `t`, clamped to `[0, 1]`.
    #[must_use]
    pub fn eval(&self, t: f64) -> f64 {
        match self {
            Self::Constant(value) => *value,
            Self::Linear(keys) => {
                let t = t.clamp(0.0, 1.0);
                let upper = keys
                    .iter()
                    .position(|key| key[0] >= t)
                    .unwrap_or(keys.len() - 1)
                    .max(1);
                let [t0, v0] = keys[upper - 1];
                let [t1, v1] = keys[upper];
                if t >= t1 {
                    return v1;
                }
                v0 + (v1 - v0) * ((t - t0) / (t1 - t0))
            }
        }
    }

    /// Values at the keys, or the constant.
    fn values(&self) -> impl Iterator<Item = f64> + '_ {
        let (constant, keys) = match self {
            Self::Constant(value) => (Some(*value), &[][..]),
            Self::Linear(keys) => (None, keys.as_slice()),
        };
        constant.into_iter().chain(keys.iter().map(|key| key[1]))
    }

    fn validate(&self) -> Result<(), SectionLawError> {
        match self {
            Self::Constant(value) if value.is_finite() => Ok(()),
            Self::Constant(_) => Err(SectionLawError::NonFinite),
            Self::Linear(keys) => {
                if keys.len() < 2 {
                    return Err(SectionLawError::TooFewKeys);
                }
                if keys.iter().flatten().any(|v| !v.is_finite()) {
                    return Err(SectionLawError::NonFinite);
                }
                let first = keys[0][0];
                let last = keys[keys.len() - 1][0];
                if first != 0.0 || last != 1.0 || keys.windows(2).any(|w| w[1][0] <= w[0][0]) {
                    return Err(SectionLawError::KeyOrder);
                }
                Ok(())
            }
        }
    }

    /// Appends a canonical encoding: a tag, then counts and `f64` bits.
    ///
    /// Adding `0.0` maps `-0.0` to `+0.0`, so laws that are equal (and
    /// produce identical geometry) encode identically.
    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        let put = |out: &mut Vec<u8>, v: f64| {
            out.extend_from_slice(&(v + 0.0).to_bits().to_le_bytes());
        };
        match self {
            Self::Constant(value) => {
                out.push(0);
                put(out, *value);
            }
            Self::Linear(keys) => {
                out.push(1);
                out.extend_from_slice(&crate::len_u32(keys.len()).to_le_bytes());
                for v in keys.iter().flatten() {
                    put(out, *v);
                }
            }
        }
    }
}

/// Scale and twist of a sweep section along normalized arc length.
///
/// Arc length is the cumulative chord length between the sweep's section
/// stations (path points, or sampled stations of an analytic path), divided
/// by the total. `scale` must stay positive; `twist` is in radians about the
/// path tangent. On a closed path both laws must agree exactly at `t = 0`
/// and `t = 1`: the first and last stations share one seam ring. `turns`
/// adds an integral number of complete revolutions while retaining that
/// exact seam. The base twist law still has to agree exactly at both ends.
///
/// Laws are evaluated only at those stations and add none: sections between
/// stations interpolate linearly along each band. Author enough stations for
/// the intended variation; a controlled sweep refuses a band whose law change
/// collapses or folds its wall.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SectionLaw {
    /// Uniform section scale about the section datum.
    pub scale: Law,
    /// Section rotation about the path tangent, in radians.
    pub twist: Law,
    /// Complete right-handed revolutions along the path, in addition to `twist`.
    /// Negative values reverse the winding direction.
    pub turns: i32,
}

impl SectionLaw {
    /// A law from its `scale` and `twist` components.
    #[must_use]
    pub fn new(scale: Law, twist: Law) -> Self {
        Self {
            scale,
            twist,
            turns: 0,
        }
    }

    /// Adds signed complete revolutions along the path.
    #[must_use]
    pub const fn with_turns(mut self, turns: i32) -> Self {
        self.turns = turns;
        self
    }

    /// A constant section: scale 1, no twist.
    pub const IDENTITY: Self = Self {
        scale: Law::Constant(1.0),
        twist: Law::Constant(0.0),
        turns: 0,
    };

    /// A linear taper of the section scale from `start` to `end`.
    #[must_use]
    pub fn taper(start: f64, end: f64) -> Self {
        Self {
            scale: Law::Linear(alloc::vec![[0.0, start], [1.0, end]]),
            twist: Law::Constant(0.0),
            turns: 0,
        }
    }

    /// True when this law leaves every section unchanged.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        let constant = |law: &Law, value: f64| law.values().all(|v| v == value);
        self.turns == 0 && constant(&self.scale, 1.0) && constant(&self.twist, 0.0)
    }

    /// Checks key structure, finiteness and positive scale.
    ///
    /// # Errors
    ///
    /// Returns the first [`SectionLawError`] found.
    pub fn validate(&self) -> Result<(), SectionLawError> {
        self.scale.validate()?;
        self.twist.validate()?;
        if self.scale.values().any(|v| v <= 0.0) {
            return Err(SectionLawError::NonPositiveScale);
        }
        Ok(())
    }

    /// Checks that both laws agree at the ends of a closed path.
    pub(crate) fn validate_closed(&self) -> Result<(), SectionLawError> {
        let seam = |law: &Law| law.eval(0.0) == law.eval(1.0);
        if seam(&self.scale) && seam(&self.twist) {
            Ok(())
        } else {
            Err(SectionLawError::OpenSeam)
        }
    }

    /// Appends a canonical encoding of both laws.
    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        self.scale.encode(out);
        self.twist.encode(out);
        if self.turns != 0 {
            out.push(2);
            out.extend_from_slice(&self.turns.to_le_bytes());
        }
    }
}

impl Default for SectionLaw {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Invalid [`SectionLaw`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SectionLawError {
    /// A key or constant is not finite.
    NonFinite,
    /// A linear law has fewer than two keys.
    TooFewKeys,
    /// Keys do not run strictly increasing from `t = 0` to `t = 1`.
    KeyOrder,
    /// A scale value is zero or negative.
    NonPositiveScale,
    /// A closed path's laws differ at `t = 0` and `t = 1`.
    OpenSeam,
}

impl core::fmt::Display for SectionLawError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NonFinite => "section law values must be finite",
            Self::TooFewKeys => "a linear section law needs at least two keys",
            Self::KeyOrder => "section law keys must increase strictly from t = 0 to t = 1",
            Self::NonPositiveScale => "section scale must be positive",
            Self::OpenSeam => "a closed sweep's section law must agree at both ends",
        })
    }
}

impl core::error::Error for SectionLawError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_laws_interpolate_and_clamp() {
        let law = Law::Linear(alloc::vec![[0.0, 2.0], [0.25, 1.0], [1.0, 0.5]]);
        assert_eq!(law.eval(0.0), 2.0);
        assert_eq!(law.eval(0.125), 1.5);
        assert_eq!(law.eval(0.25), 1.0);
        assert_eq!(law.eval(1.0), 0.5);
        assert_eq!(law.eval(-1.0), 2.0);
        assert_eq!(law.eval(2.0), 0.5);
        assert_eq!(Law::Constant(3.0).eval(0.7), 3.0);
    }

    #[test]
    fn validation_rejects_malformed_laws() {
        let law = |scale| SectionLaw {
            scale,
            twist: Law::Constant(0.0),
            turns: 0,
        };
        assert_eq!(SectionLaw::IDENTITY.validate(), Ok(()));
        assert_eq!(SectionLaw::taper(1.0, 0.25).validate(), Ok(()));
        for (scale, error) in [
            (Law::Constant(f64::NAN), SectionLawError::NonFinite),
            (Law::Constant(0.0), SectionLawError::NonPositiveScale),
            (
                Law::Linear(alloc::vec![[0.0, 1.0]]),
                SectionLawError::TooFewKeys,
            ),
            (
                Law::Linear(alloc::vec![[0.1, 1.0], [1.0, 1.0]]),
                SectionLawError::KeyOrder,
            ),
            (
                Law::Linear(alloc::vec![[0.0, 1.0], [0.5, 1.0], [0.5, 2.0], [1.0, 1.0]]),
                SectionLawError::KeyOrder,
            ),
            (
                Law::Linear(alloc::vec![[0.0, 1.0], [1.0, -1.0]]),
                SectionLawError::NonPositiveScale,
            ),
        ] {
            assert_eq!(law(scale).validate(), Err(error));
        }
    }

    #[test]
    fn identity_and_closed_seams_are_recognized() {
        assert!(SectionLaw::IDENTITY.is_identity());
        assert!(SectionLaw::taper(1.0, 1.0).is_identity());
        assert!(!SectionLaw::taper(1.0, 0.5).is_identity());
        assert_eq!(
            SectionLaw::taper(1.0, 0.5).validate_closed(),
            Err(SectionLawError::OpenSeam)
        );
        let pulse = SectionLaw {
            scale: Law::Linear(alloc::vec![[0.0, 1.0], [0.5, 2.0], [1.0, 1.0]]),
            twist: Law::Constant(0.3),
            turns: 0,
        };
        assert_eq!(pulse.validate_closed(), Ok(()));
    }
}
