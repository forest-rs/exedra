// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Region-tagging operators and helpers.

use alloc::vec;
use alloc::vec::Vec;

use exedra::FaceId;

use crate::{
    Artifacts, DiagCode, DiagLevel, Diagnostic, EditOperator, OpContext, OpError, OpErrorKind,
    OpReport,
};

/// Default untagged face region.
pub const REGION_UNTAGGED: u32 = 0;

/// Parameters for [`TagFaceRegion`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TagFaceRegionParams {
    /// Region identifier to write.
    pub region_id: u32,
    /// Faces to tag.
    pub faces: Vec<FaceId>,
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
        let canonicalized = canonicalize_faces(&mut faces);
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

        let mut tagged = Vec::with_capacity(faces.len());
        {
            let layer = txn
                .mesh_mut()
                .attrs_mut()
                .dense_mut(exedra::attr::FACE_REGION)
                .ok_or_else(|| {
                    op_error(
                        ctx,
                        OpErrorKind::MissingAttribute,
                        DiagCode::MissingRequiredAttribute,
                        "missing required dense face.region layer",
                    )
                })?;
            for face in faces {
                if face == FaceId::OUTSIDE {
                    continue;
                }
                if !layer.set(face.as_id(), params.region_id) {
                    return Err(op_error(
                        ctx,
                        OpErrorKind::PreconditionFailed,
                        DiagCode::PreconditionFailed,
                        "face set contains invalid/stale face id",
                    ));
                }
                tagged.push(face);
            }
        }

        for face in tagged {
            txn.mark_face_dirty(face);
            report.stats.counters.faces_processed =
                report.stats.counters.faces_processed.saturating_add(1);
        }

        Ok(report)
    }
}

fn canonicalize_faces(faces: &mut Vec<FaceId>) -> bool {
    let len_before = faces.len();
    faces.sort_unstable();
    faces.dedup();
    len_before != faces.len()
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

#[cfg(test)]
mod tests {
    use alloc::vec;

    use exedra::{BuildParams, Mesh, MeshBuilder};

    use super::{REGION_UNTAGGED, TagFaceRegion, TagFaceRegionParams};
    use crate::{EditOperator, OperatorRunner};

    fn one_quad_mesh() -> (Mesh, exedra::FaceId) {
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

        let result = runner
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
        assert_eq!(result.change_set.dirty.dirty_faces(), vec![face]);
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
}
