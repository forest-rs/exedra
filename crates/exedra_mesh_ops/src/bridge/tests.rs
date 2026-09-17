// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

#[test]
fn prepared_bridge_checks_unfinished_source_edits_and_accepts_equivalent_clone() {
    let mut mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ],
        &[&[0, 1, 2, 3], &[4, 7, 6, 5]],
    )
    .unwrap();
    let faces: Vec<_> = mesh.faces().collect();
    let edges = |face| {
        mesh.face_loop(face)
            .map(|edge| mesh.canonical_edge(edge).unwrap())
            .collect()
    };
    let params = BridgeBoundaryLoopsParams {
        loop_a: edges(faces[0]),
        loop_b: edges(faces[1]),
    };
    let plan = BridgeBoundaryLoopsPlan::prepare(&mesh, &params).unwrap();
    assert_eq!(plan.aligned_a_vertices().len(), 4);
    let mut clone = mesh.clone();
    let mut edit = clone.edit();
    let (stats, output) = plan.apply(&mut edit).unwrap();
    let _: () = edit.finish();
    assert_eq!(stats.boundary_edges, 8);
    assert_eq!(output.bridge_faces.len(), 4);
    assert!(clone.validate_deep().is_empty());
    assert!(clone.boundary_loops().unwrap().is_empty());
    let vertex = plan.aligned_a_vertices()[0];
    let mut edit = mesh.edit();
    op::set_vertex_position(&mut edit, vertex, [2.0, 2.0, 0.0]).unwrap();
    assert_eq!(
        plan.apply(&mut edit).unwrap_err(),
        BridgeError::StalePreparation
    );
    assert_eq!(edit.mesh().faces().count(), 2);
}
