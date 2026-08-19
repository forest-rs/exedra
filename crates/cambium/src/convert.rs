// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit cross-domain conversion helpers.
//!
//! Cambium intentionally keeps canonical geometry domains separate. This module
//! exposes explicit, typed conversion seams rather than forcing analytic state
//! through the mesh-only [`crate::EditOperator`] trait.
//!
//! Current surface:
//! - planar analytic shell -> [`exedra::Mesh`]
//! - rectangular frame spike helper -> [`exedra::Mesh`]
//! - constructive recipe -> tessellated [`exedra::Mesh`] bodies with source
//!   maps and an honest report ([`constructive_recipe_to_mesh`])
//!
//! ```rust
//! use cambium::convert::{
//!     AnalyticFaceId, AnalyticRegionId, RectFrameParams, analytic_shell_to_mesh, rect_frame_xy,
//! };
//!
//! let shell = rect_frame_xy(&RectFrameParams {
//!     region: AnalyticRegionId(7),
//!     ..RectFrameParams::default()
//! })?;
//! let converted = analytic_shell_to_mesh(&shell)?;
//! assert_eq!(converted.mesh_faces_for(AnalyticFaceId::from_index(0)).len(), 8);
//! # Ok::<(), cambium::convert::AnalyticToMeshError>(())
//! ```

use alloc::vec::Vec;
use core::fmt;

use exedra::{FaceId, Mesh};
pub use exedra_analytic::{
    AnalyticFaceId, AnalyticShell, AnalyticShellBuilder, AnalyticVertexId,
    BuildError as AnalyticBuildError, RectFrameParams, RegionId as AnalyticRegionId,
    TessellateError as AnalyticTessellateError, TessellateParams, rect_frame_xy,
};

/// Parameters for explicit analytic-shell to mesh conversion.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct AnalyticToMeshParams {
    /// Downstream tessellation settings used by `exedra_analytic`.
    pub tessellate: TessellateParams,
}

/// Parameters for the rectangular frame convenience conversion.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct RectFrameToMeshParams {
    /// Analytic rectangular frame authoring parameters.
    pub frame: RectFrameParams,
    /// Downstream tessellation settings used by `exedra_analytic`.
    pub tessellate: TessellateParams,
}

/// One deterministic analytic-face to mesh-face provenance mapping.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct AnalyticFaceProvenance {
    /// Source analytic face.
    pub analytic_face: AnalyticFaceId,
    /// Resulting Exedra mesh face.
    pub mesh_face: FaceId,
}

/// Result of explicit analytic to mesh conversion.
#[derive(Clone, Debug)]
pub struct AnalyticToMeshOutput {
    /// Converted Exedra mesh.
    pub mesh: Mesh,
    /// Deterministic analytic-face provenance in source face order.
    pub face_provenance: Vec<AnalyticFaceProvenance>,
}

impl AnalyticToMeshOutput {
    /// Returns the mapped mesh faces for `analytic_face`, when present.
    #[must_use]
    pub fn mesh_faces_for(&self, analytic_face: AnalyticFaceId) -> Vec<FaceId> {
        self.face_provenance
            .iter()
            .filter(|mapping| mapping.analytic_face == analytic_face)
            .map(|mapping| mapping.mesh_face)
            .collect()
    }
}

/// Explicit analytic to mesh conversion failure.
#[derive(Clone, Debug, PartialEq)]
pub enum AnalyticToMeshError {
    /// Analytic authoring helper failed before tessellation.
    Build(AnalyticBuildError),
    /// Analytic tessellation into Exedra mesh failed.
    Tessellate(AnalyticTessellateError),
}

impl fmt::Display for AnalyticToMeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build(error) => write!(f, "analytic build failed: {error:?}"),
            Self::Tessellate(error) => write!(f, "analytic tessellation failed: {error:?}"),
        }
    }
}

impl core::error::Error for AnalyticToMeshError {}

impl From<AnalyticBuildError> for AnalyticToMeshError {
    fn from(error: AnalyticBuildError) -> Self {
        Self::Build(error)
    }
}

impl From<AnalyticTessellateError> for AnalyticToMeshError {
    fn from(error: AnalyticTessellateError) -> Self {
        Self::Tessellate(error)
    }
}

/// Converts one analytic shell into an Exedra mesh.
pub fn analytic_shell_to_mesh(
    shell: &AnalyticShell,
) -> Result<AnalyticToMeshOutput, AnalyticToMeshError> {
    analytic_shell_to_mesh_with(shell, &AnalyticToMeshParams::default())
}

/// Converts one analytic shell into an Exedra mesh with explicit parameters.
pub fn analytic_shell_to_mesh_with(
    shell: &AnalyticShell,
    params: &AnalyticToMeshParams,
) -> Result<AnalyticToMeshOutput, AnalyticToMeshError> {
    let converted = shell
        .to_exedra_mesh(&params.tessellate)
        .map_err(AnalyticToMeshError::from)?;
    Ok(AnalyticToMeshOutput {
        mesh: converted.mesh,
        face_provenance: converted
            .face_provenance
            .into_iter()
            .map(|(analytic_face, mesh_face)| AnalyticFaceProvenance {
                analytic_face,
                mesh_face,
            })
            .collect(),
    })
}

/// Builds the rectangular frame analytic spike and converts it into an Exedra
/// mesh in one step.
pub fn rect_frame_to_mesh(
    params: &RectFrameToMeshParams,
) -> Result<AnalyticToMeshOutput, AnalyticToMeshError> {
    let shell = rect_frame_xy(&params.frame).map_err(AnalyticToMeshError::from)?;
    analytic_shell_to_mesh_with(
        &shell,
        &AnalyticToMeshParams {
            tessellate: params.tessellate,
        },
    )
}

// --- Constructive recipe -> mesh --------------------------------------------

pub use exedra_constructive::evaluate::{
    EvalError as ConstructiveEvalError, GeometryReport, PlacedBody,
    Severity as ConstructiveSeverity,
};
pub use exedra_constructive::ir::{Fingerprint as RecipeFingerprint, Recipe};
pub use exedra_constructive::tessellate::EvalPolicy as ConstructiveEvalPolicy;

use crate::diag::{DiagCode, DiagLevel, Diagnostic};

/// Parameters for explicit constructive-recipe to mesh conversion.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct ConstructiveToMeshParams {
    /// Evaluation policy used by `exedra_constructive`.
    pub policy: ConstructiveEvalPolicy,
}

/// Output of a constructive-recipe conversion.
///
/// The conversion is fingerprint-bound: `fingerprint` is the recipe's
/// content fingerprint under the evaluation schema version, so downstream
/// caches and plans can detect stale results exactly.
#[derive(Debug)]
pub struct ConstructiveToMeshOutput {
    /// Tessellated bodies in deterministic node order.
    pub bodies: Vec<PlacedBody>,
    /// The recipe's content fingerprint at conversion time.
    pub fingerprint: RecipeFingerprint,
    /// The full constructive evaluation report (fidelity, envelopes,
    /// counters, recorded policy).
    pub report: GeometryReport,
    /// The report's diagnostics, mapped into Cambium diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Converts a constructive recipe into tessellated Exedra meshes.
///
/// This is the explicit conversion seam for the constructive head (the
/// analogue of [`analytic_shell_to_mesh`]): deterministic for fixed
/// `(recipe, params)`, provenance-carrying via each body's source map, and
/// honest about anything not evaluable — those outcomes arrive as mapped
/// diagnostics and report fidelity, never as silently approximate geometry.
///
/// # Errors
///
/// Fails only when a supported body fails to tessellate.
pub fn constructive_recipe_to_mesh(
    recipe: &Recipe,
    params: &ConstructiveToMeshParams,
) -> Result<ConstructiveToMeshOutput, ConstructiveEvalError> {
    let evaluation = exedra_constructive::evaluate::evaluate(recipe, &params.policy)?;
    let diagnostics = evaluation
        .report
        .diagnostics
        .iter()
        .map(|d| {
            let level = match d.severity {
                ConstructiveSeverity::Note => DiagLevel::Note,
                ConstructiveSeverity::Warning => DiagLevel::Warn,
                ConstructiveSeverity::Error => DiagLevel::Error,
            };
            let code = match d.code {
                "eval.csg.unsupported" | "eval.unimplemented" => DiagCode::UnsupportedOperation,
                _ => DiagCode::InternalInvariantViolation,
            };
            let mut message = alloc::string::String::from(d.code);
            message.push_str(": ");
            message.push_str(&d.message);
            Diagnostic::new(level, code, message)
        })
        .collect();
    Ok(ConstructiveToMeshOutput {
        fingerprint: recipe.recipe_fingerprint(),
        bodies: evaluation.bodies,
        report: evaluation.report,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use crate::mesh_signature;

    use super::{
        AnalyticFaceId, AnalyticRegionId, AnalyticToMeshParams, RectFrameToMeshParams,
        analytic_shell_to_mesh, analytic_shell_to_mesh_with, rect_frame_to_mesh, rect_frame_xy,
    };

    #[test]
    fn analytic_shell_to_mesh_preserves_region_and_face_provenance() {
        let shell = rect_frame_xy(&super::RectFrameParams {
            region: AnalyticRegionId(7),
            ..super::RectFrameParams::default()
        })
        .expect("frame should build");

        let converted = analytic_shell_to_mesh(&shell).expect("conversion should succeed");
        assert_eq!(converted.mesh.faces().count(), 8);
        assert_eq!(converted.face_provenance.len(), 8);
        assert_eq!(
            converted
                .mesh_faces_for(AnalyticFaceId::from_index(0))
                .len(),
            8
        );
        for mapping in &converted.face_provenance {
            let region = converted
                .mesh
                .attrs()
                .dense(exedra::attr::FACE_REGION)
                .and_then(|layer| layer.get(mapping.mesh_face.as_id()).copied());
            assert_eq!(region, Some(7));
        }
    }

    #[test]
    fn analytic_shell_to_mesh_is_deterministic() {
        let shell = rect_frame_xy(&super::RectFrameParams::default()).expect("frame should build");

        let converted_a = analytic_shell_to_mesh_with(&shell, &AnalyticToMeshParams::default())
            .expect("conversion should succeed");
        let converted_b = analytic_shell_to_mesh_with(&shell, &AnalyticToMeshParams::default())
            .expect("conversion should succeed");

        assert_eq!(
            mesh_signature(&converted_a.mesh),
            mesh_signature(&converted_b.mesh)
        );
        assert_eq!(converted_a.face_provenance, converted_b.face_provenance);
    }

    #[test]
    fn rect_frame_to_mesh_matches_manual_two_step_path() {
        let params = RectFrameToMeshParams::default();

        let direct = rect_frame_to_mesh(&params).expect("direct conversion should succeed");
        let shell = rect_frame_xy(&params.frame).expect("frame should build");
        let manual = analytic_shell_to_mesh_with(
            &shell,
            &AnalyticToMeshParams {
                tessellate: params.tessellate,
            },
        )
        .expect("manual conversion should succeed");

        assert_eq!(mesh_signature(&direct.mesh), mesh_signature(&manual.mesh));
        assert_eq!(direct.face_provenance, manual.face_provenance);
    }

    #[test]
    fn constructive_recipe_conversion_is_fingerprint_bound() {
        use super::{ConstructiveToMeshParams, RecipeFingerprint, constructive_recipe_to_mesh};
        use exedra_constructive::builders;
        use exedra_constructive::ir::{CapMode, NodeKind, Placement3, RecipeBuilder};

        let build = |height: f64| {
            let mut b = RecipeBuilder::new();
            let p = b.add_profile(builders::rect(2.0, 1.0).expect("rect"));
            let n = b
                .add(NodeKind::Extrude {
                    profile: p,
                    placement: Placement3::IDENTITY,
                    height,
                    caps: CapMode::Both,
                })
                .expect("valid");
            b.finish(n).expect("valid recipe")
        };
        let recipe = build(3.0);
        let out = constructive_recipe_to_mesh(&recipe, &ConstructiveToMeshParams::default())
            .expect("converts");
        assert_eq!(out.bodies.len(), 1);
        assert!(out.diagnostics.is_empty());
        let fp: RecipeFingerprint = out.fingerprint;
        assert_eq!(fp, recipe.recipe_fingerprint());
        assert_ne!(fp, build(4.0).recipe_fingerprint());

        // Determinism across conversions.
        let again = constructive_recipe_to_mesh(&recipe, &ConstructiveToMeshParams::default())
            .expect("converts");
        assert_eq!(
            mesh_signature(&out.bodies[0].body.mesh),
            mesh_signature(&again.bodies[0].body.mesh)
        );
    }

    #[test]
    fn constructive_csg_maps_to_unsupported_diagnostic() {
        use super::{ConstructiveToMeshParams, constructive_recipe_to_mesh};
        use crate::diag::{DiagCode, DiagLevel};
        use exedra_constructive::builders;
        use exedra_constructive::ir::{CapMode, CsgOp, NodeKind, Placement3, RecipeBuilder};

        let mut b = RecipeBuilder::new();
        let p = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
        let e1 = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let e2 = b
            .add(NodeKind::Extrude {
                profile: p,
                placement: Placement3::translate(0.5, 0.0, 0.0),
                height: 1.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let csg = b
            .add(NodeKind::Csg {
                op: CsgOp::Union,
                operands: alloc::vec![e1, e2],
            })
            .expect("valid");
        let recipe = b.finish(csg).expect("valid recipe");

        let out = constructive_recipe_to_mesh(&recipe, &ConstructiveToMeshParams::default())
            .expect("converts");
        assert!(out.bodies.is_empty());
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].level, DiagLevel::Error);
        assert_eq!(out.diagnostics[0].code, DiagCode::UnsupportedOperation);
        assert!(
            out.diagnostics[0]
                .message
                .starts_with("eval.csg.unsupported")
        );
        assert_eq!(out.report.counters.envelope_only, 1);
    }
}
