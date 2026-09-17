// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::vec;

fn folded_faces() -> Mesh {
    Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
        ],
        &[&[0, 1, 2, 3], &[1, 0, 4, 5]],
    )
    .unwrap()
}

#[test]
fn prepared_normals_reject_neighbor_geometry_changes_before_any_write() {
    for mode in [
        NormalEdit::Derived(NormalParams::default()),
        NormalEdit::Smooth(NormalParams::default()),
    ] {
        let mut mesh = folded_faces();
        let face = mesh.faces().next().unwrap();
        let neighbor_vertex = mesh
            .vertices()
            .find(|&v| mesh.vertex_position(v) == Some(&[1.0, 0.0, 1.0]))
            .unwrap();
        let plan = NormalEditPlan::prepare(
            &mesh,
            &NormalEditParams {
                faces: vec![face],
                mode,
            },
        )
        .unwrap();
        let mut edit = mesh.edit();
        op::set_vertex_position(&mut edit, neighbor_vertex, [1.0, -0.5, 1.0]).unwrap();
        assert_eq!(
            plan.apply(&mut edit),
            Err(NormalEditError::StalePreparation)
        );
        assert!(
            edit.mesh()
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                .is_none()
        );
    }
}

#[test]
fn prepared_normals_apply_to_equivalent_clone_and_leave_other_faces_alone() {
    let mesh = folded_faces();
    let faces: Vec<_> = mesh.faces().collect();
    let params = NormalEditParams {
        faces: vec![faces[0], faces[0]],
        mode: NormalEdit::Derived(NormalParams::default()),
    };
    let expected = mesh.derive_corner_normals(&NormalParams::default());
    let plan = NormalEditPlan::prepare(&mesh, &params).unwrap();
    let mut clone = mesh.clone();
    let mut edit = clone.edit();
    let output = plan.apply(&mut edit).unwrap();
    let _: () = edit.finish();
    assert!(output.selections_canonicalized);
    assert_eq!(output.corners_written, 4);
    for face in clone.faces() {
        for corner in clone.face_loop(face) {
            let actual = clone
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                .unwrap()
                .get(corner.as_id())
                .copied();
            assert_eq!(
                actual,
                if face == faces[0] {
                    expected.get(corner)
                } else {
                    None
                }
            );
        }
    }
    assert!(
        mesh.attrs()
            .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
            .is_none()
    );
}
