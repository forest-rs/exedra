// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exedra Ops policy set and sub-policies.

pub use exedra_mesh::PropagatePolicy;

/// Operator quality mode.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum QualityMode {
    /// Budgeted/interactive behavior.
    Preview,
    /// Full deterministic commit behavior.
    Commit,
}

/// Deterministic work budget for preview paths.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct WorkBudget {
    /// Maximum faces to process.
    pub max_faces: u64,
    /// Maximum corners to process.
    pub max_corners: u64,
    /// Advisory wall-clock budget in milliseconds.
    ///
    /// This value is cooperative: Exedra Ops does not enforce it centrally.
    /// Operators are responsible for checking elapsed time and respecting it.
    pub max_millis: u64,
}

impl Default for WorkBudget {
    fn default() -> Self {
        Self {
            max_faces: 100_000,
            max_corners: 400_000,
            max_millis: 16,
        }
    }
}

/// Quality configuration.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct QualityPolicy {
    /// Selected quality mode.
    pub mode: QualityMode,
    /// Optional deterministic work budget.
    pub budget: Option<WorkBudget>,
}

impl Default for QualityPolicy {
    fn default() -> Self {
        Self {
            mode: QualityMode::Commit,
            budget: None,
        }
    }
}

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

/// UV-related defaults.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct UvPolicy {
    /// Default UV scale.
    pub default_scale: [f32; 2],
    /// Default UV offset.
    pub default_offset: [f32; 2],
    /// Whether existing UV values may be overwritten.
    pub allow_overwrite_existing: bool,
}

impl Default for UvPolicy {
    fn default() -> Self {
        Self {
            default_scale: [1.0, 1.0],
            default_offset: [0.0, 0.0],
            allow_overwrite_existing: false,
        }
    }
}

/// Placeholder boolean params until Exedra exposes stable boolean params.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BooleanParams {
    /// Geometric epsilon.
    pub epsilon: f32,
}

impl Default for BooleanParams {
    fn default() -> Self {
        Self { epsilon: 1.0e-6 }
    }
}

/// Boolean-stage policy.
///
/// Uses [`PartialEq`] because [`BooleanParams`] contains `f32` fields.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BooleanPolicy {
    /// Preview boolean params.
    pub preview_params: BooleanParams,
    /// Commit boolean params.
    pub commit_params: BooleanParams,
}

/// Top-level Exedra Ops policy set.
///
/// Uses [`PartialEq`] (not `Eq`) because float-bearing sub-policies (UV/boolean)
/// intentionally avoid total equality semantics.
///
/// Stored on [`OpContext::policy`](crate::OpContext::policy) and consulted by
/// operators/runners.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct PolicySet {
    /// Quality/budget controls.
    pub quality: QualityPolicy,
    /// UV controls.
    pub uv: UvPolicy,
    /// Boolean controls.
    pub boolean: BooleanPolicy,
    /// Edit propagation controls shared with Exedra edit kernels.
    pub propagate: PropagatePolicy,
    /// Runtime hard limits.
    pub limits: LimitsPolicy,
    /// Validation controls.
    pub validate: ValidatePolicy,
}

#[cfg(test)]
mod tests {
    use super::{PolicySet, QualityMode};

    #[test]
    fn policy_set_defaults_are_sensible() {
        let policy = PolicySet::default();
        assert_eq!(policy.quality.mode, QualityMode::Commit);
        assert!(policy.quality.budget.is_none());
        assert_eq!(policy.limits.max_diagnostics, 64);
        assert_eq!(policy.limits.max_artifact_items, 16);
        assert!(policy.validate.validate_on_commit);
        assert!(!policy.validate.validate_on_preview);
        assert_eq!(policy.uv.default_scale, [1.0, 1.0]);
    }
}
