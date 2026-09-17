// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Runtime adapter for direct UV projection.

use crate::{EditOperator, FaceSet, OpContext, OpError, OpReport};
pub use exedra_mesh_ops::uv::UvBoxParams;

/// Runtime adapter for [`exedra_mesh_ops::uv::project_box`].
#[derive(Copy, Clone, Debug, Default)]
pub struct UvBox;

impl EditOperator for UvBox {
    type Params = UvBoxParams;
    type Plan = UvBoxParams;
    type Output = FaceSet;
    fn name(&self) -> &'static str {
        "uv.box"
    }
    fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let output = {
            let _bucket = ctx.clock.bucket("project");
            exedra_mesh_ops::uv::project_box(txn, params)
        }
        .map_err(|error| crate::uv_common::projection_error(ctx, error))?;
        Ok(crate::uv_common::projection_report(
            ctx,
            self.name(),
            output,
        ))
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
