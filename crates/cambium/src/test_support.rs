// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared test fixtures for Cambium operator modules.

use exedra::{BuildParams, HalfEdgeId, Mesh};

pub(crate) fn shared_edge_mesh() -> (Mesh, HalfEdgeId, HalfEdgeId) {
    let mesh = Mesh::from_indexed_triangles(
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
    let shared = mesh
        .faces()
        .find_map(|face| {
            mesh.face_loop(face).find(|&half_edge| {
                mesh.from_vertex(half_edge).is_some_and(|v| v.index() == 0)
                    && mesh.to_vertex(half_edge).is_some_and(|v| v.index() == 2)
            })
        })
        .expect("shared edge should exist");
    let twin = mesh.twin(shared).expect("shared edge should have twin");
    (mesh, shared, twin)
}
