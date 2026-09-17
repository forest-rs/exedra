// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exedra Edit policy set and sub-policies.

pub use exedra_mesh::PropagatePolicy;

/// Hard runtime limits.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct LimitsPolicy {
    /// Maximum retained diagnostics.
    pub max_diagnostics: usize,
    /// Maximum retained artifact items.
    pub max_artifact_items: usize,
    /// Maximum retained artifact bytes (v0.1 estimate).
    pub max_artifact_bytes: usize,
}

impl Default for LimitsPolicy {
    fn default() -> Self {
        Self {
            max_diagnostics: 64,
            max_artifact_items: 16,
            max_artifact_bytes: 1 << 20,
        }
    }
}

/// Validation behavior policy.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ValidatePolicy {
    /// Run validation in preview mode.
    pub validate_on_preview: bool,
    /// Run validation in commit mode.
    pub validate_on_commit: bool,
    /// Treat validation errors as hard failures.
    pub fail_on_error: bool,
}

impl Default for ValidatePolicy {
    fn default() -> Self {
        Self {
            validate_on_preview: false,
            validate_on_commit: true,
            fail_on_error: false,
        }
    }
}

/// Top-level Exedra Edit policy set.
///
/// Stored on [`OpContext::policy`](crate::OpContext::policy) and consulted by
/// operators/runners.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct PolicySet {
    /// Edit propagation controls shared with Exedra edit kernels.
    pub propagate: PropagatePolicy,
    /// Runtime hard limits.
    pub limits: LimitsPolicy,
    /// Validation controls.
    pub validate: ValidatePolicy,
}

#[cfg(test)]
mod tests {
    use super::PolicySet;

    #[test]
    fn policy_set_defaults_are_sensible() {
        let policy = PolicySet::default();
        assert_eq!(policy.limits.max_diagnostics, 64);
        assert_eq!(policy.limits.max_artifact_items, 16);
        assert!(policy.validate.validate_on_commit);
        assert!(!policy.validate.validate_on_preview);
    }
}
