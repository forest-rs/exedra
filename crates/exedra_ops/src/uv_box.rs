// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic box UV projection operator.

use alloc::vec::Vec;

use exedra_mesh::{CornerId, DEFAULT_BOX_NORMAL_EPSILON, dominant_box_plane, project_corner_box};

use crate::{
    Artifact, Artifacts, DiagCode, DiagLevel, Diagnostic, EditOperator, OpContext, OpError,
    OpReport, UvScope,
    uv_common::{select_faces, stale_face_error},
};

/// Parameters for [`UvBox`].
#[derive(Clone, Debug, PartialEq)]
pub struct UvBoxParams {
    /// Face scope.
    pub scope: UvScope,
    /// Uniform UV scale multiplier.
    pub scale: f32,
    /// UV offset after scale.
    pub offset: [f32; 2],
    /// When true, only writes missing corner UV values.
    pub write_missing_only: bool,
    /// Epsilon used by dominant-axis tie-breaking.
    ///
    /// Compared between components of the unit face normal, so plane
    /// selection does not depend on face size. Defaults to
    /// [`DEFAULT_BOX_NORMAL_EPSILON`], the value render extraction uses under
    /// `UvSource::CustomOrBoxProjected`.
    pub normal_epsilon: f32,
}

impl Default for UvBoxParams {
    fn default() -> Self {
        Self {
            scope: UvScope::WholeMesh,
            scale: 1.0,
            offset: [0.0, 0.0],
            write_missing_only: false,
            normal_epsilon: DEFAULT_BOX_NORMAL_EPSILON,
        }
    }
}

/// Deterministic box projection UV operator.
///
/// The projection is `exedra_mesh`'s shared box projection
/// ([`dominant_box_plane`] and [`project_corner_box`]). For a mesh with no
/// authored UVs, authoring with this operator and extracting under the default
/// UV policy yields the same render buffers as extracting the unmodified mesh
/// under `UvSource::CustomOrBoxProjected` with the same scale and no offset.
/// Where authored UVs exist the two differ: extraction preserves them, while
/// this operator overwrites them unless `write_missing_only` is set.
#[derive(Copy, Clone, Debug, Default)]
pub struct UvBox;

impl EditOperator for UvBox {
    type Params = UvBoxParams;
    type Plan = UvBoxParams;
    type Output = crate::FaceSet;

    fn name(&self) -> &'static str {
        "uv.box"
    }

    fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        let faces = {
            let _bucket = ctx.clock.bucket("select");
            select_faces(txn.mesh(), &params.scope)
        };
        if matches!(params.scope, UvScope::FaceSet(_)) && faces.changed {
            report.stats.counters.selections_canonicalized = 1;
        }
        if faces.faces.is_empty() {
            return Ok((report, crate::FaceSet::new()));
        }
        let _ = report.artifacts.push(Artifact::FaceSet {
            name: "uv.box.affected_faces".into(),
            faces: faces.faces.clone(),
        });

        let mut pending = Vec::<(CornerId, [f32; 2])>::new();
        {
            let _bucket = ctx.clock.bucket("compute");
            for face in faces.faces.iter().copied() {
                if txn.mesh().face_edge(face).is_none() {
                    return Err(stale_face_error(
                        self.name(),
                        face,
                        report.artifacts.clone(),
                    ));
                }
                let (plane, fell_back) =
                    dominant_box_plane(txn.mesh(), face, params.normal_epsilon);
                if fell_back {
                    ctx.diagnostics.push(Diagnostic::new(
                        DiagLevel::Warn,
                        DiagCode::NumericToleranceIssue,
                        alloc::format!(
                            "uv.box face {} has degenerate normal; falling back to +Z plane",
                            face.index()
                        ),
                    ));
                }
                for corner in txn.mesh().face_loop(face) {
                    if params.write_missing_only && txn.corner_uv(corner).is_some() {
                        report.stats.counters.corners_skipped_existing = report
                            .stats
                            .counters
                            .corners_skipped_existing
                            .saturating_add(1);
                        continue;
                    }
                    let uv =
                        project_corner_box(txn.mesh(), corner, plane, params.scale, params.offset)
                            .expect("face loop corner must have destination vertex");
                    pending.push((corner, uv));
                }
                report.stats.counters.faces_processed =
                    report.stats.counters.faces_processed.saturating_add(1);
            }
        }

        {
            let _bucket = ctx.clock.bucket("attrs");
            for (corner, uv) in pending {
                if exedra_mesh::op::set_corner_uv(txn, corner, uv).is_ok() {
                    report.stats.counters.corners_written =
                        report.stats.counters.corners_written.saturating_add(1);
                }
            }
        }
        Ok((report, faces.faces))
    }

    fn compile(
        &self,
        _mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        _ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        Ok(params.clone())
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        self.apply(txn, plan, ctx)
    }
}

#[cfg(test)]
mod tests {
    use exedra_mesh::{ExtractParams, MeshBuilder, UvSource};

    use super::{UvBox, UvBoxParams};
    use crate::{OperatorRunner, UvScope, test_support::commit};

    #[test]
    fn uv_box_matches_extraction_time_box_projection() {
        let mut builder = MeshBuilder::new();
        // Octahedron: eight faces on eight distinct dominant planes.
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([-1.0, 0.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder.push_vertex([0.0, -1.0, 0.0]);
        builder.push_vertex([0.0, 0.0, 1.0]);
        builder.push_vertex([0.0, 0.0, -1.0]);
        builder.add_face(&[0, 2, 4]).expect("face");
        builder.add_face(&[2, 1, 4]).expect("face");
        builder.add_face(&[1, 3, 4]).expect("face");
        builder.add_face(&[3, 0, 4]).expect("face");
        builder.add_face(&[2, 0, 5]).expect("face");
        builder.add_face(&[1, 2, 5]).expect("face");
        builder.add_face(&[3, 1, 5]).expect("face");
        builder.add_face(&[0, 3, 5]).expect("face");
        let pristine = builder.build().expect("build should succeed").mesh;

        for scale in [1.0_f32, 2.5] {
            let mut authored = pristine.clone();
            let mut runner = OperatorRunner::new();
            let _ = commit(
                &mut runner,
                &mut authored,
                &UvBox,
                &UvBoxParams {
                    scale,
                    ..UvBoxParams::default()
                },
            )
            .expect("uv.box should succeed");

            let (from_operator, operator_stats) = authored.to_trimesh(&ExtractParams::default());
            let (from_policy, policy_stats) = pristine.to_trimesh(&ExtractParams {
                uvs: UvSource::CustomOrBoxProjected { scale },
                ..ExtractParams::default()
            });
            assert_eq!(from_operator, from_policy, "scale {scale}");
            assert_eq!(operator_stats, policy_stats, "scale {scale}");
            assert!(
                policy_stats.uv_split_count > 0,
                "planes differ across faces"
            );
        }
    }

    #[test]
    fn uv_box_projects_axis_aligned_face() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 1.0]);
        builder.push_vertex([1.0, 0.0, 1.0]);
        builder.push_vertex([1.0, 1.0, 1.0]);
        builder.push_vertex([0.0, 1.0, 1.0]);
        builder
            .add_face(&[0, 1, 2, 3])
            .expect("quad should be valid");
        let mut mesh = builder.build().expect("build should succeed").mesh;

        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &UvBox,
            &UvBoxParams {
                scope: UvScope::WholeMesh,
                scale: 1.0,
                offset: [0.0, 0.0],
                write_missing_only: false,
                normal_epsilon: 1.0e-6,
            },
        )
        .expect("uv.box should succeed");
        assert_eq!(result.report.stats.counters.faces_processed, 1);
        assert_eq!(result.report.stats.counters.corners_written, 4);
        let (tri, _) = mesh.to_trimesh(&ExtractParams::default());
        for (position, uv) in tri.positions.iter().zip(&tri.uvs) {
            assert_eq!(*uv, [position[0], position[1]]);
        }
    }

    #[test]
    fn uv_box_is_deterministic_on_sphere_like_mesh() {
        let mut builder = MeshBuilder::new();
        // Octahedron-like mesh.
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([-1.0, 0.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder.push_vertex([0.0, -1.0, 0.0]);
        builder.push_vertex([0.0, 0.0, 1.0]);
        builder.push_vertex([0.0, 0.0, -1.0]);
        builder.add_face(&[0, 2, 4]).expect("face");
        builder.add_face(&[2, 1, 4]).expect("face");
        builder.add_face(&[1, 3, 4]).expect("face");
        builder.add_face(&[3, 0, 4]).expect("face");
        builder.add_face(&[2, 0, 5]).expect("face");
        builder.add_face(&[1, 2, 5]).expect("face");
        builder.add_face(&[3, 1, 5]).expect("face");
        builder.add_face(&[0, 3, 5]).expect("face");
        let mut mesh_a = builder.build().expect("build should succeed").mesh;
        let mut mesh_b = mesh_a.clone();

        let mut runner_a = OperatorRunner::new();
        let mut runner_b = OperatorRunner::new();
        let params = UvBoxParams::default();
        let _ = commit(&mut runner_a, &mut mesh_a, &UvBox, &params).expect("run");
        let _ = commit(&mut runner_b, &mut mesh_b, &UvBox, &params).expect("run");

        let (tri_a, stats_a) = mesh_a.to_trimesh(&ExtractParams::default());
        let (tri_b, stats_b) = mesh_b.to_trimesh(&ExtractParams::default());
        assert_eq!(stats_a, stats_b);
        assert_eq!(tri_a.indices, tri_b.indices);
        assert_eq!(tri_a.uvs, tri_b.uvs);
    }
}
