// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::vec;

#[test]
fn bounds_distinguish_all_vertices_from_face_references() {
    let mut mesh = Mesh::from_polygons(
        &[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
        &[&[0, 1, 2]],
    )
    .unwrap();
    let face = mesh.faces().next().unwrap();
    let mut edit = mesh.edit();
    let _ = exedra_mesh::op::add_vertex(&mut edit, [10.0, 10.0, 10.0]);
    let _: () = edit.finish();
    let revision = mesh.revision();
    let all = bounds(&mesh, &BoundsParams::default()).unwrap();
    let selected = bounds(
        &mesh,
        &BoundsParams {
            scope: BoundsScope::FaceSet(vec![face, face]),
        },
    )
    .unwrap();
    assert_eq!(all.vertex_count, 4);
    assert_eq!(all.bounds.unwrap().max, [10.0; 3]);
    assert_eq!(selected.vertex_count, 3);
    assert_eq!(selected.face_count, 1);
    assert!(selected.selections_canonicalized);
    assert_eq!(selected.bounds.unwrap().max, [2.0, 2.0, 0.0]);
    assert_eq!(
        selected.bounds.unwrap().centroid,
        [2.0 / 3.0, 2.0 / 3.0, 0.0]
    );
    assert_eq!(mesh.revision(), revision);
}

#[test]
fn bounds_refuse_overflow_instead_of_returning_an_infinite_diagonal() {
    let mut mesh = Mesh::new();
    let mut edit = mesh.edit();
    let _ = exedra_mesh::op::add_vertex(&mut edit, [0.0; 3]);
    let _ = exedra_mesh::op::add_vertex(&mut edit, [1e20; 3]);
    let _: () = edit.finish();
    assert_eq!(
        bounds(&mesh, &BoundsParams::default()),
        Err(BoundsError::NumericLimit)
    );
}
