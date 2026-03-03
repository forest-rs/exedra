// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Region-tagging operators and helpers.

use alloc::vec;

use exedra::FaceId;

use crate::selection::{FaceSet, canonicalize_face_set};
use crate::{
    Artifacts, DiagCode, DiagLevel, Diagnostic, EditOperator, OpContext, OpError, OpErrorKind,
    OpReport, SmallCounters,
};

/// Default untagged face region.
pub const REGION_UNTAGGED: u32 = 0;

/// Parameters for [`TagFaceRegion`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TagFaceRegionParams {
    /// Region identifier to write.
    pub region_id: u32,
    /// Faces to tag.
    pub faces: FaceSet,
}

/// Edit operator that writes face-region tags.
#[derive(Copy, Clone, Debug, Default)]
pub struct TagFaceRegion;

impl EditOperator for TagFaceRegion {
    type Params = TagFaceRegionParams;

    fn name(&self) -> &'static str {
        "tag.face.region"
    }

    fn apply(
        &self,
        txn: &mut exedra::Txn<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<OpReport, OpError> {
        let mut faces = params.faces.clone();
        let canonicalized = canonicalize_face_set(&mut faces);
        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        if canonicalized {
            report.stats.counters.selections_canonicalized = 1;
        }
        if txn
            .mesh()
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .is_none()
        {
            return Err(op_error(
                ctx,
                OpErrorKind::MissingAttribute,
                DiagCode::MissingRequiredAttribute,
                "missing required dense face.region layer",
            ));
        }

        for face in faces {
            if face == FaceId::OUTSIDE {
                continue;
            }
            if !txn.set_face_region(face, params.region_id) {
                return Err(op_error(
                    ctx,
                    OpErrorKind::PreconditionFailed,
                    DiagCode::PreconditionFailed,
                    "face set contains invalid/stale face id",
                ));
            }
            report.stats.counters.faces_processed =
                report.stats.counters.faces_processed.saturating_add(1);
        }

        Ok(report)
    }
}

fn op_error(ctx: &OpContext, kind: OpErrorKind, code: DiagCode, message: &'static str) -> OpError {
    OpError::new(
        kind,
        vec![Diagnostic::new(DiagLevel::Error, code, message)],
        Artifacts::new(
            ctx.policy.limits.max_artifact_items,
            ctx.policy.limits.max_artifact_bytes,
        ),
    )
}

/// Deterministic face-selection query result.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RegionSelection {
    /// Canonical face IDs matching the requested region.
    pub faces: FaceSet,
    /// Query counters.
    pub counters: SmallCounters,
}

/// Returns all non-OUTSIDE faces tagged with `region_id`.
pub fn select_faces_by_region(
    mesh: &exedra::Mesh,
    region_id: u32,
) -> Result<RegionSelection, OpError> {
    let layer = mesh
        .attrs()
        .dense(exedra::attr::FACE_REGION)
        .ok_or_else(|| {
            OpError::new(
                OpErrorKind::MissingAttribute,
                vec![Diagnostic::new(
                    DiagLevel::Error,
                    DiagCode::MissingRequiredAttribute,
                    "missing required dense face.region layer",
                )],
                Artifacts::default(),
            )
        })?;

    let mut result = RegionSelection::default();
    for face in mesh.faces() {
        if face == FaceId::OUTSIDE {
            continue;
        }
        result.counters.faces_processed = result.counters.faces_processed.saturating_add(1);
        if layer
            .get(face.as_id())
            .is_some_and(|value| *value == region_id)
        {
            result.faces.push(face);
        }
    }
    if canonicalize_face_set(&mut result.faces) {
        result.counters.selections_canonicalized = 1;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use exedra::{BuildParams, FaceId, Id, Mesh, MeshBuilder};

    use super::{REGION_UNTAGGED, TagFaceRegion, TagFaceRegionParams, select_faces_by_region};
    use crate::{EditOperator, OperatorRunner};

    fn one_quad_mesh() -> (Mesh, FaceId) {
        let mut builder = MeshBuilder::new();
        let _ = builder.push_vertex([0.0, 0.0, 0.0]);
        let _ = builder.push_vertex([1.0, 0.0, 0.0]);
        let _ = builder.push_vertex([1.0, 1.0, 0.0]);
        let _ = builder.push_vertex([0.0, 1.0, 0.0]);
        builder.add_face(&[0, 1, 2, 3]).expect("valid quad");
        let built = builder.build().expect("build should succeed");
        (built.mesh, built.face_ids[0])
    }

    #[test]
    fn tag_face_region_sets_dense_layer_and_marks_dirty() {
        let (mut mesh, face) = one_quad_mesh();
        let mut runner = OperatorRunner::new();
        let op = TagFaceRegion;
        let params = TagFaceRegionParams {
            region_id: 7,
            faces: vec![face, face],
        };

        let mut result = runner
            .run_commit(&mut mesh, &op, &params)
            .expect("tag operator should succeed");

        let region = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("face region layer must exist")
            .get(face.as_id())
            .copied();
        assert_eq!(region, Some(7));
        assert_eq!(result.report.name, op.name());
        assert_eq!(result.report.stats.counters.faces_processed, 1);
        assert_eq!(result.report.stats.counters.selections_canonicalized, 1);
        let mut dirty_faces = Vec::new();
        result.change_set.dirty.drain_faces_into(&mut dirty_faces);
        assert_eq!(dirty_faces, vec![face]);
    }

    #[test]
    fn tag_face_region_supports_explicit_untagged_region() {
        let (mut mesh, face) = one_quad_mesh();
        let mut runner = OperatorRunner::new();
        let op = TagFaceRegion;
        let set_non_zero = TagFaceRegionParams {
            region_id: 3,
            faces: vec![face],
        };
        runner
            .run_commit(&mut mesh, &op, &set_non_zero)
            .expect("tagging to non-zero should succeed");

        let set_untagged = TagFaceRegionParams {
            region_id: REGION_UNTAGGED,
            faces: vec![face],
        };
        runner
            .run_commit(&mut mesh, &op, &set_untagged)
            .expect("tagging to untagged should succeed");

        let region = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("face region layer must exist")
            .get(face.as_id())
            .copied();
        assert_eq!(region, Some(REGION_UNTAGGED));
    }

    #[test]
    fn face_region_default_is_untagged() {
        let mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let region = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("face region layer must exist")
            .get(face.as_id())
            .copied();
        assert_eq!(region, Some(REGION_UNTAGGED));
    }

    #[test]
    fn select_faces_by_region_returns_canonical_face_set() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let mut runner = OperatorRunner::new();

        let params = TagFaceRegionParams {
            region_id: 11,
            faces: vec![faces[1], faces[0], faces[1]],
        };
        let _ = runner
            .run_commit(&mut mesh, &TagFaceRegion, &params)
            .expect("tagging should succeed");

        let selected = select_faces_by_region(&mesh, 11).expect("selection should succeed");
        assert_eq!(selected.faces, vec![faces[0], faces[1]]);
        assert_eq!(selected.counters.faces_processed, 2);
        assert_eq!(selected.counters.selections_canonicalized, 0);
    }

    #[test]
    fn tag_face_region_rejects_stale_face_id() {
        let (mut mesh, _) = one_quad_mesh();
        let mut runner = OperatorRunner::new();
        let stale = FaceId::from(Id::new(999, NonZeroU32::MIN));
        let params = TagFaceRegionParams {
            region_id: 5,
            faces: vec![stale],
        };

        let error = runner
            .run_commit(&mut mesh, &TagFaceRegion, &params)
            .expect_err("stale face id should fail");
        assert_eq!(error.kind, crate::OpErrorKind::PreconditionFailed);
        let face = mesh.faces().next().expect("face should exist");
        assert!(
            mesh.attrs()
                .dense(exedra::attr::FACE_REGION)
                .expect("face region layer should exist")
                .get(face.as_id())
                .is_some_and(|v| *v == 0)
        );
    }
}
