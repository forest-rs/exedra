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

/// Typed output from [`ValidateMesh`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ValidateMeshOutput {
    /// Number of emitted validation diagnostics.
    pub diagnostics_emitted: u64,
}

impl EditOperator for ValidateMesh {
    type Params = ValidateMeshParams;
    type Plan = ValidateMeshParams;
    type Output = ValidateMeshOutput;

    fn name(&self) -> &'static str {
        "inspect.validate.mesh"
    }

    fn apply<S: exedra::ChangeSink>(
        &self,
        txn: &mut exedra::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
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

        let emitted = diagnostics.len();
        for diagnostic in diagnostics {
            ctx.diagnostics.push(diagnostic);
        }

        let output = ValidateMeshOutput {
            diagnostics_emitted: u64::try_from(emitted).expect("diagnostic count should fit u64"),
        };
        Ok((report, output))
    }

    fn compile(
        &self,
        _mesh: &exedra::Mesh,
        params: &Self::Params,
        _ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        Ok(*params)
    }

    fn apply_plan<S: exedra::ChangeSink>(
        &self,
        txn: &mut exedra::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        self.apply(txn, plan, ctx)
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
    use alloc::string::String;
    use alloc::vec;

    use exedra::Mesh;
    use exedra_ops_testkit::{GoldenSnapshot, GoldenStep, parse_snapshot, render_snapshot};

    use super::{ValidateMesh, ValidateMeshMode, ValidateMeshParams, validation_diagnostics};
    use crate::{DiagCode, DiagLevel, EditOperator, OpContext};

    #[test]
    fn validate_mesh_reports_no_diagnostics_for_valid_mesh() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.edit();
        let op = ValidateMesh;
        let mut ctx = OpContext::default();
        let (report, output) = op
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
        assert_eq!(output.diagnostics_emitted, 0);
        assert!(ctx.diagnostics.is_empty());
    }

    #[test]
    fn validate_mesh_fast_mode_runs_without_errors_for_valid_mesh() {
        let mut mesh = Mesh::new();
        let mut txn = mesh.edit();
        let op = ValidateMesh;
        let mut ctx = OpContext::default();
        let (_, output) = op
            .apply(
                &mut txn,
                &ValidateMeshParams {
                    mode: ValidateMeshMode::Fast,
                },
                &mut ctx,
            )
            .expect("validation operator should succeed");
        assert_eq!(output.diagnostics_emitted, 0);
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

    #[test]
    fn exedra_ops_tests_can_use_exedra_ops_testkit_snapshot_format() {
        let snapshot = GoldenSnapshot {
            scenario: String::from("validate-op"),
            steps: vec![GoldenStep {
                label: String::from("step_1"),
                op: String::from("inspect.validate.mesh"),
                faces_processed: 0,
                corners_written: 0,
                selections_canonicalized: 0,
                diagnostics: vec![],
            }],
        };
        let encoded = render_snapshot(&snapshot);
        let decoded = parse_snapshot(&encoded).expect("roundtrip should parse");
        assert_eq!(decoded, snapshot);
    }
}
