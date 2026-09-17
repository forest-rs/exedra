// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::vec;

fn quad() -> Mesh {
    Mesh::from_polygons(
        &[
            [1.0, 0.0, 0.0],
            [1.0, 2.0, 0.0],
            [0.0, 2.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
        &[&[0, 1, 2, 3]],
    )
    .unwrap()
}

#[test]
fn projection_checks_all_pending_values_before_writing() {
    let mut mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1e20, 0.0, 0.0],
            [1e20, 1.0, 0.0],
            [1e20, 0.0, 1.0],
        ],
        &[&[0, 1, 2], &[3, 4, 5]],
    )
    .unwrap();
    let mut edit = mesh.edit();
    assert_eq!(
        project_planar(
            &mut edit,
            &UvPlanarParams {
                scale: 1e20,
                ..Default::default()
            }
        ),
        Err(UvError::NumericLimit)
    );
    for face in edit.mesh().faces() {
        for corner in edit.mesh().face_loop(face) {
            assert_eq!(edit.corner_uv(corner), None);
        }
    }
    let _: () = edit.finish();
}

#[test]
fn projection_refuses_invalid_parameters_and_outside_selection() {
    let mut mesh = quad();
    let face = mesh.faces().next().unwrap();
    let mut edit = mesh.edit();
    assert_eq!(
        project_box(
            &mut edit,
            &UvBoxParams {
                normal_epsilon: -1.0,
                ..Default::default()
            }
        ),
        Err(UvError::InvalidParameters)
    );
    assert_eq!(
        project_cylinder(
            &mut edit,
            &UvCylinderParams {
                seam_offset_radians: f32::NAN,
                ..Default::default()
            }
        ),
        Err(UvError::InvalidParameters)
    );
    assert_eq!(
        project_planar(
            &mut edit,
            &UvPlanarParams {
                scope: UvScope::FaceSet(vec![face, FaceId::OUTSIDE]),
                ..Default::default()
            }
        ),
        Err(UvError::InvalidFace {
            face: FaceId::OUTSIDE
        })
    );
    for corner in edit.mesh().face_loop(face) {
        assert_eq!(edit.corner_uv(corner), None);
    }
    let _: () = edit.finish();
}

#[test]
fn cylinder_retains_authored_uvs_and_reports_only_actual_writes() {
    let mut mesh = quad();
    let face = mesh.faces().next().unwrap();
    let kept = mesh.face_loop(face).next().unwrap();
    let mut edit = mesh.edit();
    op::set_corner_uv(&mut edit, kept, [9.0, 8.0]).unwrap();
    let output = project_cylinder(
        &mut edit,
        &UvCylinderParams {
            scope: UvScope::FaceSet(vec![face, face]),
            write_missing_only: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(output.selections_canonicalized);
    assert_eq!(output.faces, vec![face]);
    assert_eq!(output.corners_written, 3);
    assert_eq!(output.corners_skipped_existing, 1);
    assert!(output.fallback_faces.is_empty());
    for corner in edit.mesh().face_loop(face) {
        let p = corner_position(edit.mesh(), corner).unwrap();
        let expected = if corner == kept {
            [9.0, 8.0]
        } else {
            [if p[2] == 1.0 { 0.25 } else { 0.0 }, p[1]]
        };
        assert_eq!(edit.corner_uv(corner), Some(expected));
    }
    let _: () = edit.finish();
}
