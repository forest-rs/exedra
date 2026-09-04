// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! End-to-end primitive/operator smoke tests.

#[cfg(test)]
mod tests {
    use exedra_mesh::ExtractParams;
    use exedra_ops::{OperatorRunner, UvBox, UvBoxParams, UvScope};
    use exedra_primitives::{BoxParams, box_primitive};

    #[test]
    fn box_primitive_runs_through_uv_box_operator() {
        let mut primitive = box_primitive(&BoxParams {
            size: [2.0, 3.0, 4.0],
            centered: true,
            segments: [1, 1, 1],
        });
        let mut runner = OperatorRunner::new();
        let plan = runner
            .compile(
                &primitive.mesh,
                &UvBox,
                &UvBoxParams {
                    scope: UvScope::WholeMesh,
                    scale: 1.0,
                    offset: [0.0, 0.0],
                    write_missing_only: false,
                    normal_epsilon: 1.0e-6,
                },
            )
            .expect("uv.box compile should succeed on primitive box");
        let result = runner
            .apply_in_place(&mut primitive.mesh, &UvBox, &plan)
            .expect("uv.box should succeed on primitive box");

        assert_eq!(result.report.stats.counters.faces_processed, 6);
        assert!(result.report.stats.counters.corners_written > 0);
        assert!(primitive.mesh.validate_fast().is_empty());

        let (tri, _) = primitive.mesh.to_trimesh(&ExtractParams::default());
        assert!(
            tri.uvs.iter().any(|uv| uv[0] != 0.0 || uv[1] != 0.0),
            "uv.box should write non-zero UV values for this primitive"
        );
    }
}
