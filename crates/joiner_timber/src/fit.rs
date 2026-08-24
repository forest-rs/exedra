// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Typed fit allowance shared by timber rules.

use crate::Length;
use crate::length::default_micrometers;

/// How much larger the receiving profile is than the nominal interface.
///
/// `per_side` is a radial/profile offset: a rectangular mortise becomes
/// `2 * per_side` wider and deeper overall. Axial depth is unchanged. Rules
/// lower this exact allowance to meters only when constructing the profile.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FitClass {
    /// The receiving cut is exactly the nominal interface.
    LineToLine,
    /// The receiving profile is enlarged by `per_side` on every side.
    Clearance {
        /// Exact positive profile offset.
        per_side: Length,
    },
}

impl FitClass {
    /// A close, explicitly allowed fit with 0.5 mm per-side clearance.
    pub const CLOSE: Self = Self::Clearance {
        per_side: default_micrometers(500),
    };

    pub(crate) fn allowance_meters(self) -> f64 {
        match self {
            Self::LineToLine => 0.0,
            Self::Clearance { per_side } => per_side.as_meters(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_allowance_has_one_exact_canonical_representation() {
        // Line-to-line is the sole zero-clearance form. Every clearance value
        // is positive and exact before the profile builder needs meters.
        assert_eq!(FitClass::LineToLine.allowance_meters(), 0.0);
        assert_eq!(
            FitClass::CLOSE,
            FitClass::Clearance {
                per_side: Length::micrometers(500).unwrap()
            }
        );
        assert_eq!(FitClass::CLOSE.allowance_meters(), 0.000_5);
        assert!(Length::from_iota(0).is_none());
    }
}
