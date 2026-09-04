// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Analytic workflow helpers over [`exedra_analytic`].
//!
//! These helpers are domain-native analytic edits. They intentionally do not
//! implement the mesh-only [`crate::EditOperator`] trait or run through
//! [`crate::OperatorRunner`]. Use them to mutate analytic state, then cross the
//! explicit conversion seam in [`crate::convert`] when you need a mesh.
//!
//! Current helpers cover analytic face regions plus opening add/remove lifecycle
//! on the planar analytic MVP.

use crate::{Artifacts, DiagCode, DiagLevel, Diagnostic, OpError, OpErrorKind};

pub use exedra_analytic::{
    AnalyticFaceId, AnalyticLoopId, AnalyticShell, EditError as AnalyticEditError,
    RectOpeningParams, RegionId as AnalyticRegionId,
};

/// Stable helper name for analytic face-region mutation.
pub const SET_FACE_REGION_NAME: &str = "analytic.face.set_region";

/// Stable helper name for adding a rectangular analytic opening.
pub const ADD_RECT_OPENING_XY_NAME: &str = "analytic.face.add_opening.rect_xy";

/// Stable helper name for removing an analytic opening.
pub const REMOVE_OPENING_NAME: &str = "analytic.face.remove_opening";

/// Parameters for [`set_face_region`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SetFaceRegionParams {
    /// Target face.
    pub face: AnalyticFaceId,
    /// New region value.
    pub region: AnalyticRegionId,
}

/// Output of [`set_face_region`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SetFaceRegionOutput {
    /// Updated face.
    pub face: AnalyticFaceId,
    /// Region written to `face`.
    pub region: AnalyticRegionId,
}

/// Parameters for [`add_rect_opening_xy`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AddRectOpeningParams {
    /// Target face.
    pub face: AnalyticFaceId,
    /// Opening rectangle in face XY coordinates.
    pub rect: RectOpeningParams,
}

/// Output of [`add_rect_opening_xy`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct AddRectOpeningOutput {
    /// Face that owns the new opening.
    pub face: AnalyticFaceId,
    /// Created opening loop.
    pub opening: AnalyticLoopId,
}

/// Parameters for [`remove_opening`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RemoveOpeningParams {
    /// Face expected to own `opening`.
    pub face: AnalyticFaceId,
    /// Opening loop to remove from `face`.
    pub opening: AnalyticLoopId,
}

/// Output of [`remove_opening`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RemoveOpeningOutput {
    /// Face that previously owned `opening`.
    pub face: AnalyticFaceId,
    /// Opening loop removed from `face`.
    pub opening: AnalyticLoopId,
}

/// Sets the semantic region carried by an analytic face.
pub fn set_face_region(
    shell: &mut AnalyticShell,
    params: &SetFaceRegionParams,
) -> Result<SetFaceRegionOutput, OpError> {
    shell
        .set_face_region(params.face, params.region)
        .map_err(map_analytic_edit_error)?;
    Ok(SetFaceRegionOutput {
        face: params.face,
        region: params.region,
    })
}

/// Adds a rectangular opening to an XY-aligned analytic face.
pub fn add_rect_opening_xy(
    shell: &mut AnalyticShell,
    params: &AddRectOpeningParams,
) -> Result<AddRectOpeningOutput, OpError> {
    let opening = shell
        .add_rect_opening_xy(params.face, &params.rect)
        .map_err(map_analytic_edit_error)?;
    Ok(AddRectOpeningOutput {
        face: params.face,
        opening,
    })
}

/// Removes one existing opening from an analytic face.
pub fn remove_opening(
    shell: &mut AnalyticShell,
    params: &RemoveOpeningParams,
) -> Result<RemoveOpeningOutput, OpError> {
    shell
        .remove_opening(params.face, params.opening)
        .map_err(map_analytic_edit_error)?;
    Ok(RemoveOpeningOutput {
        face: params.face,
        opening: params.opening,
    })
}

fn map_analytic_edit_error(error: AnalyticEditError) -> OpError {
    let (kind, code, message) = match error {
        AnalyticEditError::MissingFace { face } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            alloc::format!("analytic face does not exist: {}", face.index()),
        ),
        AnalyticEditError::InvalidFaceTopology { face } => (
            OpErrorKind::InternalInvariantViolation,
            DiagCode::InternalInvariantViolation,
            alloc::format!("analytic face has invalid topology: {}", face.index()),
        ),
        AnalyticEditError::InvalidRectBounds => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            alloc::string::String::from("rectangular analytic opening bounds are invalid"),
        ),
        AnalyticEditError::FaceNotXyAligned { face } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            alloc::format!("analytic face is not XY-aligned: {}", face.index()),
        ),
        AnalyticEditError::OpeningOutsideFace { face } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            alloc::format!("analytic opening lies outside face {}", face.index()),
        ),
        AnalyticEditError::OpeningOverlapsExisting { face } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            alloc::format!(
                "analytic opening overlaps an existing opening on face {}",
                face.index()
            ),
        ),
        AnalyticEditError::OpeningNotOnFace { face, opening } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            alloc::format!(
                "analytic opening {} is not owned by face {}",
                opening.index(),
                face.index()
            ),
        ),
    };
    OpError::new(
        kind,
        alloc::vec![Diagnostic::new(DiagLevel::Error, code, message)],
        Artifacts::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        ADD_RECT_OPENING_XY_NAME, AddRectOpeningParams, AnalyticRegionId, REMOVE_OPENING_NAME,
        RemoveOpeningParams, SET_FACE_REGION_NAME, SetFaceRegionParams, add_rect_opening_xy,
        remove_opening, set_face_region,
    };
    use crate::convert::analytic_shell_to_mesh;
    use exedra_analytic::{
        AnalyticFaceId, AnalyticLoopId, AnalyticShellBuilder, RectOpeningParams, RegionId,
    };

    #[test]
    fn analytic_edit_then_convert_workflow_preserves_region_and_opening() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([4.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([4.0, 3.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 3.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(1))
            .expect("outer face should build");
        let mut shell = builder.build();

        let region = set_face_region(
            &mut shell,
            &SetFaceRegionParams {
                face,
                region: AnalyticRegionId(12),
            },
        )
        .expect("region update should succeed");
        assert_eq!(region.region, AnalyticRegionId(12));

        let opening = add_rect_opening_xy(
            &mut shell,
            &AddRectOpeningParams {
                face,
                rect: RectOpeningParams {
                    min: [1.0, 1.0],
                    max: [3.0, 2.0],
                },
            },
        )
        .expect("opening edit should succeed");
        assert_eq!(opening.face, face);

        let converted = analytic_shell_to_mesh(&shell).expect("conversion should succeed");
        assert_eq!(
            converted
                .mesh_faces_for(AnalyticFaceId::from_index(0))
                .len(),
            8
        );
        assert!(
            converted
                .face_provenance
                .iter()
                .all(|mapping| mapping.analytic_face == face)
        );
        for mapping in &converted.face_provenance {
            let region = converted
                .mesh
                .attrs()
                .dense(exedra_mesh::attr::FACE_REGION)
                .and_then(|layer| layer.get(mapping.mesh_face.as_id()).copied());
            assert_eq!(region, Some(12));
        }
    }

    #[test]
    fn analytic_add_rect_opening_reports_precondition_failure() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([4.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([4.0, 3.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 3.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(1))
            .expect("outer face should build");
        let mut shell = builder.build();

        let error = add_rect_opening_xy(
            &mut shell,
            &AddRectOpeningParams {
                face,
                rect: RectOpeningParams {
                    min: [3.5, 1.0],
                    max: [4.5, 2.0],
                },
            },
        )
        .expect_err("out-of-bounds opening should fail");
        assert_eq!(error.kind, crate::OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn analytic_add_rect_opening_accepts_clockwise_xy_face() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([4.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([4.0, 3.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 3.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v3, v2, v1], RegionId(4))
            .expect("clockwise outer face should build");
        let mut shell = builder.build();

        let opening = add_rect_opening_xy(
            &mut shell,
            &AddRectOpeningParams {
                face,
                rect: RectOpeningParams {
                    min: [1.0, 1.0],
                    max: [3.0, 2.0],
                },
            },
        )
        .expect("opening edit should succeed on clockwise XY face");
        assert_eq!(opening.face, face);

        let converted = analytic_shell_to_mesh(&shell).expect("conversion should succeed");
        assert_eq!(converted.mesh_faces_for(face).len(), 8);
        for mapping in &converted.face_provenance {
            let region = converted
                .mesh
                .attrs()
                .dense(exedra_mesh::attr::FACE_REGION)
                .and_then(|layer| layer.get(mapping.mesh_face.as_id()).copied());
            assert_eq!(region, Some(4));
        }
    }

    #[test]
    fn analytic_remove_opening_updates_shell_and_conversion() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([8.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([8.0, 4.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 4.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(9))
            .expect("outer face should build");
        let mut shell = builder.build();

        let opening = add_rect_opening_xy(
            &mut shell,
            &AddRectOpeningParams {
                face,
                rect: RectOpeningParams {
                    min: [1.0, 1.0],
                    max: [3.0, 2.5],
                },
            },
        )
        .expect("opening edit should succeed");

        let removed = remove_opening(
            &mut shell,
            &RemoveOpeningParams {
                face,
                opening: opening.opening,
            },
        )
        .expect("opening removal should succeed");
        assert_eq!(removed.face, face);
        assert_eq!(removed.opening, opening.opening);
        assert!(
            shell
                .face_opening_vertices(face)
                .expect("face should still be readable")
                .is_empty()
        );

        let converted = analytic_shell_to_mesh(&shell).expect("conversion should succeed");
        let mesh_faces = converted.mesh_faces_for(face);
        assert_eq!(mesh_faces.len(), 1);
        let region = converted
            .mesh
            .attrs()
            .dense(exedra_mesh::attr::FACE_REGION)
            .and_then(|layer| layer.get(mesh_faces[0].as_id()).copied());
        assert_eq!(region, Some(9));
    }

    #[test]
    fn analytic_remove_opening_reports_precondition_failure() {
        let mut builder = AnalyticShellBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([4.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([4.0, 3.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 3.0, 0.0]);
        let face = builder
            .add_planar_face(&[v0, v1, v2, v3], RegionId(1))
            .expect("outer face should build");
        let mut shell = builder.build();

        let error = remove_opening(
            &mut shell,
            &RemoveOpeningParams {
                face,
                opening: AnalyticLoopId::from_index(99),
            },
        )
        .expect_err("missing opening should fail");
        assert_eq!(error.kind, crate::OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn analytic_helper_names_are_stable() {
        assert_eq!(SET_FACE_REGION_NAME, "analytic.face.set_region");
        assert_eq!(
            ADD_RECT_OPENING_XY_NAME,
            "analytic.face.add_opening.rect_xy"
        );
        assert_eq!(REMOVE_OPENING_NAME, "analytic.face.remove_opening");
    }
}
