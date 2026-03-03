// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Mesh validation operator for debug/inspection workflows.

use alloc::format;
use alloc::vec::Vec;

use exedra::ValidationError;

use crate::{
    Artifacts, DiagCode, DiagLevel, Diagnostic, EditOperator, OpContext, OpError, OpReport,
};

/// Validation pass configuration.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ValidateMeshMode {
    /// Run `validate_fast`.
    Fast,
    /// Run `validate_deep`.
    Deep,
    /// Run both passes (`fast` then `deep`).
    FastAndDeep,
}

/// Parameters for [`ValidateMesh`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ValidateMeshParams {
    /// Validation pass mode.
    pub mode: ValidateMeshMode,
}

impl Default for ValidateMeshParams {
    fn default() -> Self {
        Self {
            mode: ValidateMeshMode::FastAndDeep,
        }
    }
}

/// Debug operator that validates mesh invariants and reports diagnostics.
#[derive(Copy, Clone, Debug, Default)]
pub struct ValidateMesh;

impl EditOperator for ValidateMesh {
    type Params = ValidateMeshParams;

    fn name(&self) -> &'static str {
        "inspect.validate.mesh"
    }

    fn apply(
        &self,
        txn: &mut exedra::Txn<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<OpReport, OpError> {
        let mesh = txn.mesh();
        let mut report = OpReport::new(self.name(), Artifacts::default());
        report.stats.elements_touched.vertices = mesh.vertices().count() as u64;
        report.stats.elements_touched.faces = mesh.faces().count() as u64;

        let mut diagnostics = Vec::new();
        match params.mode {
            ValidateMeshMode::Fast => {
                diagnostics.extend(validation_diagnostics("fast", &mesh.validate_fast()));
            }
            ValidateMeshMode::Deep => {
                diagnostics.extend(validation_diagnostics("deep", &mesh.validate_deep()));
            }
            ValidateMeshMode::FastAndDeep => {
                diagnostics.extend(validation_diagnostics("fast", &mesh.validate_fast()));
                diagnostics.extend(validation_diagnostics("deep", &mesh.validate_deep()));
            }
        }

        for diagnostic in diagnostics {
            ctx.diagnostics.push(diagnostic);
        }

        Ok(report)
    }
}

fn validation_diagnostics(pass: &'static str, errors: &[ValidationError]) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|error| {
            let code = match error {
                ValidationError::EdgeMultiplicity { .. } => DiagCode::NonManifoldInput,
                _ => DiagCode::InternalInvariantViolation,
            };
            Diagnostic::new(
                DiagLevel::Error,
                code,
                format!("validation ({pass}): {error:?}"),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use exedra::Mesh;

    use super::{ValidateMesh, ValidateMeshMode, ValidateMeshParams, validation_diagnostics};
    use crate::{DiagCode, DiagLevel, EditOperator, OpContext};

    #[test]
    fn validate_mesh_reports_no_diagnostics_for_valid_mesh() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.begin();
        let op = ValidateMesh;
        let mut ctx = OpContext::default();
        let report = op
            .apply(
                &mut txn,
                &ValidateMeshParams {
                    mode: ValidateMeshMode::FastAndDeep,
                },
                &mut ctx,
            )
            .expect("validation operator should succeed");
        assert_eq!(report.name, "inspect.validate.mesh");
        assert_eq!(report.stats.elements_touched.vertices, 0);
        assert_eq!(report.stats.elements_touched.faces, 0);
        assert!(ctx.diagnostics.is_empty());
    }

    #[test]
    fn validate_mesh_fast_mode_runs_without_errors_for_valid_mesh() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.begin();
        let op = ValidateMesh;
        let mut ctx = OpContext::default();
        let _ = op
            .apply(
                &mut txn,
                &ValidateMeshParams {
                    mode: ValidateMeshMode::Fast,
                },
                &mut ctx,
            )
            .expect("validation operator should succeed");
        assert!(ctx.diagnostics.is_empty());
    }

    #[test]
    fn validation_diagnostics_maps_invalid_errors() {
        let diagnostics = validation_diagnostics(
            "fast",
            &[exedra::ValidationError::EdgeMultiplicity {
                a: 1,
                b: 2,
                count: 3,
            }],
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].level, DiagLevel::Error);
        assert_eq!(diagnostics[0].code, DiagCode::NonManifoldInput);
        assert!(diagnostics[0].message.contains("validation (fast):"));
    }
}
