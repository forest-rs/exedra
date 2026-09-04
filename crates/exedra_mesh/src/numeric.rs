// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit numeric tolerance policy for geometry operations.

/// Canonical numeric tolerance policy for Exedra operations.
///
/// All tolerance-based geometric comparisons should flow through this policy
/// rather than using ad-hoc epsilon constants.
///
/// Pass this through higher-level operator/primitive APIs when you want mesh
/// construction and validation behavior to stay numerically consistent.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct NumericPolicy {
    /// Base epsilon used for near-equality checks.
    pub epsilon: f32,
    /// Distance threshold for merge/snap style operations.
    pub merge_tolerance: f32,
    /// Tolerance used for coplanarity checks.
    pub coplanar_tolerance: f32,
    /// Epsilon used for normal-related degeneracy checks.
    pub normal_epsilon: f32,
}

impl NumericPolicy {
    /// Default epsilon.
    pub const DEFAULT_EPSILON: f32 = 1.0e-6;
    /// Default merge tolerance.
    pub const DEFAULT_MERGE_TOLERANCE: f32 = 1.0e-5;
    /// Default coplanar tolerance.
    pub const DEFAULT_COPLANAR_TOLERANCE: f32 = 1.0e-5;
    /// Default normal epsilon.
    pub const DEFAULT_NORMAL_EPSILON: f32 = 1.0e-6;
}

impl Default for NumericPolicy {
    fn default() -> Self {
        Self {
            epsilon: Self::DEFAULT_EPSILON,
            merge_tolerance: Self::DEFAULT_MERGE_TOLERANCE,
            coplanar_tolerance: Self::DEFAULT_COPLANAR_TOLERANCE,
            normal_epsilon: Self::DEFAULT_NORMAL_EPSILON,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NumericPolicy;

    #[test]
    fn default_values_match_documented_constants() {
        let defaults = NumericPolicy::default();
        assert_eq!(defaults.epsilon, NumericPolicy::DEFAULT_EPSILON);
        assert_eq!(
            defaults.merge_tolerance,
            NumericPolicy::DEFAULT_MERGE_TOLERANCE
        );
        assert_eq!(
            defaults.coplanar_tolerance,
            NumericPolicy::DEFAULT_COPLANAR_TOLERANCE
        );
        assert_eq!(
            defaults.normal_epsilon,
            NumericPolicy::DEFAULT_NORMAL_EPSILON
        );
    }

    #[test]
    fn default_values_are_sensible() {
        let defaults = NumericPolicy::default();

        assert!(defaults.epsilon > 0.0);
        assert!(defaults.merge_tolerance >= defaults.epsilon);
        assert!(defaults.coplanar_tolerance >= defaults.epsilon);
        assert!(defaults.normal_epsilon > 0.0);
    }
}
