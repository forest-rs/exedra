// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Runtime adapter for direct UV projection.

use crate::{EditOperator, FaceSet, OpContext, OpError, OpReport};
pub use exedra_mesh_ops::uv::{CylinderAxis, UvCylinderParams};

/// Runtime adapter for [`exedra_mesh_ops::uv::project_cylinder`].
#[derive(Copy, Clone, Debug, Default)]
pub struct UvCylinder;

impl EditOperator for UvCylinder {
    type Params = UvCylinderParams;
    type Plan = UvCylinderParams;
    type Output = FaceSet;
    fn name(&self) -> &'static str {
        "uv.cylinder"
    }
    fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let output = {
            let _bucket = ctx.clock.bucket("project");
            exedra_mesh_ops::uv::project_cylinder(txn, params)
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

    use super::{CylinderAxis, UvCylinder, UvCylinderParams};
    use crate::{OperatorRunner, UvScope, test_support::commit};

    fn side_strip_mesh() -> exedra_mesh::Mesh {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 1.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 1.0]);
        builder.push_vertex([0.0, 0.0, 1.0]);
        builder
            .add_face(&[0, 1, 2, 3])
            .expect("quad should be valid");
        builder.build().expect("build should succeed").mesh
    }

    #[test]
    fn uv_cylinder_projects_with_configurable_axis() {
        let mut mesh = side_strip_mesh();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &UvCylinder,
            &UvCylinderParams {
                scope: UvScope::WholeMesh,
                axis: CylinderAxis::Y,
                seam_offset_radians: 0.0,
                scale: [1.0, 1.0],
                offset: [0.0, 0.0],
                write_missing_only: false,
            },
        )
        .expect("uv.cylinder should succeed");
        assert_eq!(result.report.stats.counters.faces_processed, 1);
        assert_eq!(result.report.stats.counters.corners_written, 4);
        let (tri, _) = mesh.to_trimesh(&ExtractParams::default());
        for (position, uv) in tri.positions.iter().zip(&tri.uvs) {
            assert_eq!(
                uv[1], position[1],
                "Y-axis projection must retain the vertex's height"
            );
            let expected_u = if position[2] == 1.0 { 0.25 } else { 0.0 };
            assert_eq!(uv[0], expected_u);
        }
    }

    #[test]
    fn uv_cylinder_seam_placement_is_deterministic() {
        let mut mesh_a = side_strip_mesh();
        let mut mesh_b = side_strip_mesh();
        let mut runner_a = OperatorRunner::new();
        let mut runner_b = OperatorRunner::new();
        let params = UvCylinderParams {
            scope: UvScope::WholeMesh,
            axis: CylinderAxis::Z,
            seam_offset_radians: 0.75,
            scale: [1.0, 1.0],
            offset: [0.0, 0.0],
            write_missing_only: false,
        };

        let _ =
            commit(&mut runner_a, &mut mesh_a, &UvCylinder, &params).expect("run should succeed");
        let _ =
            commit(&mut runner_b, &mut mesh_b, &UvCylinder, &params).expect("run should succeed");

        let (tri_a, _) = mesh_a.to_trimesh(&ExtractParams::default());
        let (tri_b, _) = mesh_b.to_trimesh(&ExtractParams::default());
        assert_eq!(tri_a.uvs, tri_b.uvs);
        assert!(tri_a.uvs.iter().all(|uv| uv[0] >= 0.0 && uv[0] < 1.0));
    }
}
