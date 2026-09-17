// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Runtime adapter for direct UV projection.

use crate::{EditOperator, FaceSet, OpContext, OpError, OpReport};
pub use exedra_mesh_ops::uv::{UvPlanarParams, UvPlane, UvScope};

/// Runtime adapter for [`exedra_mesh_ops::uv::project_planar`].
#[derive(Copy, Clone, Debug, Default)]
pub struct UvPlanar;

impl EditOperator for UvPlanar {
    type Params = UvPlanarParams;
    type Plan = UvPlanarParams;
    type Output = FaceSet;
    fn name(&self) -> &'static str {
        "uv.planar"
    }
    fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let output = {
            let _bucket = ctx.clock.bucket("project");
            exedra_mesh_ops::uv::project_planar(txn, params)
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
    use exedra_mesh::{ExtractParams, MeshBuilder};

    use super::{UvPlanar, UvPlanarParams, UvPlane, UvScope};
    use crate::{OperatorRunner, test_support::commit};

    #[test]
    fn uv_planar_writes_uvs_and_extracts_trimesh() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 1.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face(&[0, 1, 2, 3])
            .expect("quad should be valid");
        let mut mesh = builder.build().expect("build should succeed").mesh;
        let mut runner = OperatorRunner::new();
        let op = UvPlanar;
        let result = commit(
            &mut runner,
            &mut mesh,
            &op,
            &UvPlanarParams {
                scope: UvScope::WholeMesh,
                plane: UvPlane::WorldXY,
                scale: 2.0,
                offset: [0.5, 1.0],
                write_missing_only: false,
                normal_epsilon: 1.0e-6,
            },
        )
        .expect("uv.planar should succeed");

        assert_eq!(result.report.stats.counters.faces_processed, 1);
        assert_eq!(result.report.stats.counters.corners_written, 4);
        let (tri, stats) = mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats.triangle_count, 2);
        assert_eq!(tri.indices.len(), 6);
        assert!(tri.uvs.contains(&[0.5, 1.0]));
        assert!(tri.uvs.contains(&[2.5, 3.0]));
        for (position, uv) in tri.positions.iter().zip(&tri.uvs) {
            assert_eq!(*uv, [2.0 * position[0] + 0.5, 2.0 * position[1] + 1.0]);
        }
    }

    #[test]
    fn uv_planar_write_missing_only_skips_existing_values() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face(&[0, 1, 2])
            .expect("triangle should be valid");
        let mut mesh = builder.build().expect("build should succeed").mesh;
        let face = mesh.faces().next().expect("face should exist");
        let corner = mesh.face_loop(face).next().expect("corner should exist");
        {
            let mut txn = mesh.edit();
            assert!(exedra_mesh::op::set_corner_uv(&mut txn, corner, [9.0, 9.0]).is_ok());
            let _: () = txn.finish();
        }

        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &UvPlanar,
            &UvPlanarParams {
                scope: UvScope::WholeMesh,
                plane: UvPlane::WorldXY,
                scale: 1.0,
                offset: [0.0, 0.0],
                write_missing_only: true,
                normal_epsilon: 1.0e-6,
            },
        )
        .expect("uv.planar should succeed");

        assert_eq!(result.report.stats.counters.faces_processed, 1);
        assert_eq!(result.report.stats.counters.corners_written, 2);
        assert_eq!(result.report.stats.counters.corners_skipped_existing, 1);
        let uv = mesh
            .attrs()
            .sparse(exedra_mesh::attr::CORNER_UV)
            .and_then(|layer| layer.get(corner.as_id()))
            .copied();
        assert_eq!(uv, Some([9.0, 9.0]));
    }

    #[test]
    fn uv_planar_per_face_degenerate_geometry_emits_warning() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([2.0, 0.0, 0.0]);
        builder
            .add_face(&[0, 1, 2])
            .expect("topology is valid even when geometry is degenerate");
        let mut mesh = builder.build().expect("build should succeed").mesh;
        let mut runner = OperatorRunner::new();
        let _ = commit(
            &mut runner,
            &mut mesh,
            &UvPlanar,
            &UvPlanarParams {
                scope: UvScope::WholeMesh,
                plane: UvPlane::PerFaceFromGeometry,
                scale: 1.0,
                offset: [0.0, 0.0],
                write_missing_only: false,
                normal_epsilon: 1.0e-6,
            },
        )
        .expect("uv.planar should succeed");

        assert!(runner.ctx.diagnostics.iter().any(|diag| {
            diag.level == crate::DiagLevel::Warn
                && diag.code == crate::DiagCode::NumericToleranceIssue
        }));
    }
}
