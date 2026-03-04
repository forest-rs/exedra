// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Structured Cambium operator errors.

use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;
use core::fmt;
use exedra::{BuildError, ValidationError};

use crate::{Artifacts, DiagCode, DiagLevel, Diagnostic};

/// Classifies high-level operator failure cause for [`OpError`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum OpErrorKind {
    /// A required precondition failed before mutation.
    PreconditionFailed,
    /// Mesh topology/structure is invalid for this operator.
    InvalidMesh,
    /// Required attribute is missing or has incompatible domain/type.
    MissingAttribute,
    /// Numeric issue (degeneracy, NaN/Inf, tolerance mismatch).
    NumericFailure,
    /// Execution budget was exceeded.
    BudgetExceeded,
    /// Execution was cancelled by user/orchestrator.
    Cancelled,
    /// Internal invariant violated (bug).
    InternalInvariantViolation,
}

impl fmt::Display for OpErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::PreconditionFailed => "precondition failed",
            Self::InvalidMesh => "invalid mesh",
            Self::MissingAttribute => "missing attribute",
            Self::NumericFailure => "numeric failure",
            Self::BudgetExceeded => "budget exceeded",
            Self::Cancelled => "cancelled",
            Self::InternalInvariantViolation => "internal invariant violation",
        };
        f.write_str(text)
    }
}

impl core::error::Error for OpErrorKind {}

/// Structured operator error payload with context.
///
/// Returned by operator application and runner execution paths
/// ([`OperatorRunner::run_commit`](crate::OperatorRunner::run_commit),
/// [`OperatorRunner::run_preview`](crate::OperatorRunner::run_preview)).
#[derive(Clone, Debug)]
pub struct OpError {
    /// Error classification.
    pub kind: OpErrorKind,
    /// Attached diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// Attached bounded artifacts.
    pub artifacts: Artifacts,
    /// Optional committed change summary when failure happens post-commit.
    pub change_set: Option<Box<exedra::ChangeSet>>,
}

impl OpError {
    /// Creates a new error payload.
    #[must_use]
    pub fn new(kind: OpErrorKind, diagnostics: Vec<Diagnostic>, artifacts: Artifacts) -> Self {
        Self {
            kind,
            diagnostics,
            artifacts,
            change_set: None,
        }
    }

    /// Attaches a committed change summary.
    #[must_use]
    pub fn with_change_set(mut self, change_set: exedra::ChangeSet) -> Self {
        self.change_set = Some(Box::new(change_set));
        self
    }

    /// Wraps an Exedra build error into Cambium operator space.
    #[must_use]
    pub fn from_build_error(
        error: BuildError,
        diagnostics: Vec<Diagnostic>,
        artifacts: Artifacts,
    ) -> Self {
        let kind = match error {
            BuildError::InvalidWeldTolerance => OpErrorKind::NumericFailure,
            BuildError::IndexOutOfBounds { .. } => OpErrorKind::PreconditionFailed,
            BuildError::DegenerateTriangle { .. } => OpErrorKind::NumericFailure,
            BuildError::NonManifoldEdge { .. } => OpErrorKind::InvalidMesh,
            BuildError::BoundaryStitchFailed { .. } => OpErrorKind::InvalidMesh,
            BuildError::InvalidFaceLoop { .. } => OpErrorKind::InvalidMesh,
        };
        Self::new(kind, diagnostics, artifacts)
    }

    /// Wraps Exedra validation errors into Cambium operator space.
    #[must_use]
    pub fn from_validation_errors(
        errors: &[ValidationError],
        mut diagnostics: Vec<Diagnostic>,
        artifacts: Artifacts,
    ) -> Self {
        diagnostics.extend(errors.iter().map(|error| {
            let code = match error {
                ValidationError::EdgeMultiplicity { .. } => DiagCode::NonManifoldInput,
                _ => DiagCode::InternalInvariantViolation,
            };
            Diagnostic::new(
                DiagLevel::Error,
                code,
                format!("validation error: {error:?}"),
            )
        }));
        Self::new(OpErrorKind::InvalidMesh, diagnostics, artifacts)
    }
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (diagnostics: {}, artifacts: {})",
            self.kind,
            self.diagnostics.len(),
            self.artifacts.len()
        )
    }
}

impl core::error::Error for OpError {}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;
    use alloc::vec;
    use exedra::{BuildError, ValidationError};

    use super::{OpError, OpErrorKind};
    use crate::{Artifacts, DiagCode, DiagLevel, Diagnostic};

    #[test]
    fn build_error_mapping_is_classified() {
        let diagnostics = vec![Diagnostic::new(
            DiagLevel::Error,
            DiagCode::PreconditionFailed,
            "oops",
        )];
        let error = OpError::from_build_error(
            BuildError::InvalidWeldTolerance,
            diagnostics,
            Artifacts::new(2, 128),
        );
        assert_eq!(error.kind, OpErrorKind::NumericFailure);
    }

    #[test]
    fn validation_error_mapping_is_invalid_mesh() {
        let error = OpError::from_validation_errors(
            &[ValidationError::FaceLoopNotClosed { face: 3 }],
            vec![],
            Artifacts::new(2, 128),
        );
        assert_eq!(error.kind, OpErrorKind::InvalidMesh);
        assert_eq!(error.diagnostics.len(), 1);
        assert_eq!(error.diagnostics[0].level, DiagLevel::Error);
        assert_eq!(
            error.diagnostics[0].code,
            DiagCode::InternalInvariantViolation
        );
    }

    #[test]
    fn validation_error_mapping_preserves_existing_diagnostics() {
        let seed = Diagnostic::new(
            DiagLevel::Warn,
            DiagCode::PreconditionFailed,
            "seed diagnostic",
        );
        let error = OpError::from_validation_errors(
            &[
                ValidationError::EdgeMultiplicity {
                    a: 1,
                    b: 2,
                    count: 3,
                },
                ValidationError::FaceLoopNotClosed { face: 1 },
            ],
            vec![seed],
            Artifacts::new(2, 128),
        );
        assert_eq!(error.diagnostics.len(), 3);
        assert_eq!(error.diagnostics[0].message, "seed diagnostic");
        assert_eq!(error.diagnostics[1].code, DiagCode::NonManifoldInput);
    }

    #[test]
    fn change_set_can_be_attached_to_errors() {
        let changes = exedra::ChangeSet::default();
        let error = OpError::new(OpErrorKind::InvalidMesh, vec![], Artifacts::new(2, 128))
            .with_change_set(changes.clone());
        assert_eq!(
            error.change_set.as_ref().map(|v| v.deleted_faces.len()),
            Some(0)
        );
    }

    #[test]
    fn display_contains_kind_and_counts() {
        let error = OpError::new(
            OpErrorKind::Cancelled,
            vec![Diagnostic::new(
                DiagLevel::Warn,
                DiagCode::Cancelled,
                "cancelled",
            )],
            Artifacts::new(4, 256),
        );
        assert!(error.to_string().contains("cancelled"));
        assert!(error.to_string().contains("diagnostics: 1"));
    }
}
