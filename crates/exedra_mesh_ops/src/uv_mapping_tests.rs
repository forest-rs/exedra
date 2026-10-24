// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::vec;
use exedra_mesh::{ChangeSetBuilder, ExtractParams, attr};
use exedra_testkit::fixtures::{unit_box, unit_quad};

fn params(projection: UvProjection) -> UvMappingParams {
    UvMappingParams {
        scope: UvScope::WholeMesh,
        mapping: UvMapping {
            projection,
            transform: UvTransform::IDENTITY,
        },
        options: UvMappingOptions::default(),
        write_missing_only: false,
    }
}

fn uv(mesh: &Mesh, corner: CornerId) -> Option<[f32; 2]> {
    mesh.attrs()
        .sparse(attr::CORNER_UV)
        .and_then(|layer| layer.get(corner.into()))
        .copied()
}

#[test]
fn box_mapping_preserves_topology_authors_seams_and_marks_dirty_corners() {
    let mut mesh = unit_box();
    let vertices: Vec<_> = mesh.vertices().collect();
    let faces: Vec<_> = mesh.faces().collect();
    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    let output = map_uvs(&mut edit, &params(UvProjection::Box(BoxUv::default()))).unwrap();
    let changes = edit.finish();
    assert_eq!(output.corners_written, 24);
    assert!(output.fallback_faces.is_empty());
    assert!(changes.dirty.has_dirty_corners());
    assert_eq!(mesh.vertices().collect::<Vec<_>>(), vertices);
    assert_eq!(mesh.faces().collect::<Vec<_>>(), faces);
    assert!(mesh.validate_deep().is_empty());
    for face in mesh.faces() {
        let (plane, _) = exedra_mesh::dominant_box_plane(&mesh, face, DEFAULT_BOX_NORMAL_EPSILON);
        for corner in mesh.face_loop(face) {
            let position = *mesh
                .vertex_position(mesh.to_vertex(corner).unwrap())
                .unwrap();
            assert_eq!(
                uv(&mesh, corner).unwrap(),
                exedra_mesh::project_box_position(position, plane, 1.0, [0.0; 2])
            );
        }
    }
    let (render, _) = mesh.to_trimesh(&ExtractParams::default());
    assert!(
        render.positions.len() > vertices.len(),
        "corner seams split render vertices"
    );
}

#[test]
fn imported_coordinates_are_transformed_without_reprojection() {
    let mut mesh = unit_quad();
    let corners: Vec<_> = mesh.face_loop(mesh.faces().next().unwrap()).collect();
    {
        let mut edit = mesh.edit();
        for (index, &corner) in corners.iter().enumerate() {
            let value = f32::from(u16::try_from(index).unwrap());
            op::set_corner_uv(&mut edit, corner, [value, -value]).unwrap();
        }
    }
    let mut parameters = params(UvProjection::Existing);
    parameters.mapping.transform =
        UvTransform::rotate_offset_scale(0.0, [1.0, 2.0], [-2.0, 3.0]).unwrap();
    map_uvs(&mut mesh.edit(), &parameters).unwrap();
    for (index, &corner) in corners.iter().enumerate() {
        let value = f32::from(u16::try_from(index).unwrap());
        assert_eq!(
            uv(&mesh, corner),
            Some([-2.0 * (value + 1.0), 3.0 * (2.0 - value)])
        );
    }
}

#[test]
fn selection_is_canonical_and_missing_only_keeps_authored_values() {
    let mut mesh = unit_box();
    let face = mesh.faces().next().unwrap();
    let corner = mesh.face_loop(face).next().unwrap();
    op::set_corner_uv(&mut mesh.edit(), corner, [20.0, -30.0]).unwrap();
    let mut parameters = params(UvProjection::Planar(UvFrame::XY));
    parameters.scope = UvScope::FaceSet(vec![face, face]);
    parameters.write_missing_only = true;
    let output = map_uvs(&mut mesh.edit(), &parameters).unwrap();
    assert!(output.selections_canonicalized);
    assert_eq!(output.faces, vec![face]);
    assert_eq!(output.corners_skipped_existing, 1);
    assert_eq!(output.corners_written, 3);
    assert_eq!(uv(&mesh, corner), Some([20.0, -30.0]));
    for other in mesh.faces().filter(|&f| f != face) {
        for corner in mesh.face_loop(other) {
            assert_eq!(uv(&mesh, corner), None);
        }
    }
}

#[test]
fn authored_corner_normals_choose_box_planes() {
    let mut mesh = unit_quad();
    let face = mesh.faces().next().unwrap();
    let corner = mesh.face_loop(face).next().unwrap();
    op::set_corner_normal_override(&mut mesh.edit(), corner, Some([1.0, 0.0, 0.0])).unwrap();
    map_uvs(
        &mut mesh.edit(),
        &params(UvProjection::Box(BoxUv::default())),
    )
    .unwrap();
    let position = *mesh
        .vertex_position(mesh.to_vertex(corner).unwrap())
        .unwrap();
    assert_eq!(uv(&mesh, corner), Some([-position[2], position[1]]));
    assert_eq!(
        mesh.attrs()
            .sparse(attr::CORNER_NORMAL_OVERRIDE)
            .unwrap()
            .get(corner.into()),
        Some(&[1.0, 0.0, 0.0])
    );
}

#[test]
fn zero_corner_normals_report_the_box_fallback() {
    let mut mesh = unit_quad();
    let face = mesh.faces().next().unwrap();
    let corner = mesh.face_loop(face).next().unwrap();
    op::set_corner_normal_override(&mut mesh.edit(), corner, Some([0.0; 3])).unwrap();
    let output = map_uvs(
        &mut mesh.edit(),
        &params(UvProjection::Box(BoxUv::default())),
    )
    .unwrap();
    assert_eq!(output.fallback_faces, vec![face]);
}

#[test]
fn previews_and_noops_are_inspectable_without_mutating_uvs() {
    let mut mesh = unit_quad();
    let parameters = params(UvProjection::Planar(UvFrame::XY));
    let mut scratch = UvAuthoringScratch::default();
    let revision = mesh.revision();
    let preview = preview_uvs(&mesh, &parameters, &mut scratch).unwrap();
    assert_eq!(preview.corners_written, 4);
    assert!(preview.mapped_bounds.is_some());
    assert_eq!(mesh.revision(), revision);
    assert_eq!(
        inspect_uvs(&mesh, &UvScope::WholeMesh, UvSet::Primary)
            .unwrap()
            .missing_corners,
        4
    );
    let mut edit = mesh.edit();
    let output = map_uvs_with_scratch(&mut edit, &parameters, &mut scratch).unwrap();
    let _: () = edit.finish();
    assert_eq!(preview, output);
    let capacity = scratch.retained_bytes();
    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    let noop = map_uvs_with_scratch(&mut edit, &parameters, &mut scratch).unwrap();
    assert_eq!(noop.corners_written, 0);
    assert_eq!(noop.corners_unchanged, 4);
    assert!(!edit.finish().dirty.has_dirty_corners());
    assert_eq!(scratch.retained_bytes(), capacity);
    assert_eq!(
        inspect_uvs(&mesh, &UvScope::WholeMesh, UvSet::Primary)
            .unwrap()
            .bounds,
        preview.mapped_bounds
    );
}

#[test]
fn existing_uvs_can_populate_a_secondary_channel_without_changing_the_primary() {
    let mut mesh = unit_quad();
    map_uvs(&mut mesh.edit(), &params(UvProjection::Planar(UvFrame::XY))).unwrap();
    let primary = inspect_uvs(&mesh, &UvScope::WholeMesh, UvSet::Primary).unwrap();
    let mut parameters = params(UvProjection::Existing);
    parameters.options.target = UvSet::Secondary;
    parameters.mapping.transform = UvTransform::translation([2.0, 3.0]).unwrap();
    let output = map_uvs(&mut mesh.edit(), &parameters).unwrap();
    assert_eq!(output.corners_written, 4);
    assert_eq!(
        inspect_uvs(&mesh, &UvScope::WholeMesh, UvSet::Primary).unwrap(),
        primary
    );
    let secondary = inspect_uvs(&mesh, &UvScope::WholeMesh, UvSet::Secondary).unwrap();
    assert_eq!(secondary.missing_corners, 0);
    for face in mesh.faces() {
        for corner in mesh.face_loop(face) {
            let p = corner_uv(&mesh, UvSet::Primary, corner).unwrap();
            assert_eq!(
                corner_uv(&mesh, UvSet::Secondary, corner),
                Some([p[0] + 2.0, p[1] + 3.0])
            );
        }
    }
}

#[test]
fn missing_uvs_bad_selection_and_late_overflow_do_not_write() {
    let mut mesh = unit_quad();
    let face = mesh.faces().next().unwrap();
    let first = mesh.face_loop(face).next().unwrap();
    op::set_corner_uv(&mut mesh.edit(), first, [4.0, 5.0]).unwrap();
    let mut cases = vec![params(UvProjection::Existing)];
    let mut bad_face = params(UvProjection::Planar(UvFrame::XY));
    bad_face.scope = UvScope::FaceSet(vec![face, FaceId::OUTSIDE]);
    cases.push(bad_face);
    let mut overflow = params(UvProjection::Planar(UvFrame::XY));
    overflow.mapping.transform.matrix[0][0] = f64::MAX;
    cases.push(overflow);
    for parameters in cases {
        let original = mesh.revision();
        let mut edit = mesh.edit_with(ChangeSetBuilder::new());
        assert!(map_uvs(&mut edit, &parameters).is_err());
        assert_eq!(edit.mesh().revision(), original);
        let changes = edit.finish();
        assert!(!changes.dirty.has_dirty_corners());
        for corner in mesh.face_loop(face) {
            assert_eq!(
                uv(&mesh, corner),
                if corner == first {
                    Some([4.0, 5.0])
                } else {
                    None
                }
            );
        }
    }
}

#[test]
fn cross_face_batches_match_single_corner_mapping_and_late_refusal_is_atomic() {
    let mut mesh = exedra_testkit::fixtures::grid_mesh(17, 17);
    let mut parameters = params(UvProjection::Planar(
        UvFrame::from_normal([0.25, -0.5, 1.0], [1.0, 2.0, 3.0]).unwrap(),
    ));
    parameters.mapping.transform = UvTransform {
        matrix: [[1.8, -0.6], [0.35, 1.4]],
        offset: [0.25, -0.4],
    };
    let expected: Vec<_> = mesh
        .faces()
        .flat_map(|face| mesh.face_loop(face))
        .map(|corner| {
            let position = *mesh
                .vertex_position(mesh.to_vertex(corner).unwrap())
                .unwrap();
            (
                corner,
                parameters
                    .mapping
                    .map(&UvInput {
                        position,
                        normal: [0.0; 3],
                        existing: None,
                    })
                    .unwrap(),
            )
        })
        .collect();
    let mut scratch = UvAuthoringScratch::default();
    let mut edit = mesh.edit();
    let output = map_uvs_with_scratch(&mut edit, &parameters, &mut scratch).unwrap();
    let _: () = edit.finish();
    assert_eq!(
        usize::try_from(output.corners_written).unwrap(),
        expected.len()
    );
    for (corner, expected) in expected {
        assert_eq!(
            corner_uv(&mesh, UvSet::Primary, corner)
                .unwrap()
                .map(f32::to_bits),
            expected.map(f32::to_bits)
        );
    }
    let last = mesh.faces().last().unwrap();
    let bad_corner = mesh.face_loop(last).next().unwrap();
    // A late invalid source must not commit any of the already prepared batches.
    op::set_corner_uv(&mut mesh.edit(), bad_corner, [f32::INFINITY; 2]).unwrap();
    let snapshot = mesh.clone();
    parameters.mapping.projection = UvProjection::Existing;
    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    assert!(
        matches!(map_uvs_with_scratch(&mut edit, &parameters, &mut scratch), Err(UvError::Mapping { corner: Some(corner), error: UvMappingError::NumericLimit }) if corner == bad_corner)
    );
    let changes = edit.finish();
    assert!(!changes.dirty.has_dirty_corners());
    for face in mesh.faces() {
        for corner in mesh.face_loop(face) {
            assert_eq!(
                corner_uv(&mesh, UvSet::Primary, corner),
                corner_uv(&snapshot, UvSet::Primary, corner)
            );
        }
    }
}

#[test]
fn missing_only_angular_mapping_retains_neighbors_for_pole_recovery() {
    let mut mesh = Mesh::from_polygons(
        &[[1.0, -0.01, 0.0], [1.0, 0.01, 0.0], [0.0, 0.0, 1.0]],
        &[&[0, 1, 2]],
    )
    .unwrap();
    let corners: Vec<_> = mesh.face_loop(mesh.faces().next().unwrap()).collect();
    let inputs: Vec<_> = corners
        .iter()
        .map(|&corner| UvInput {
            position: *mesh
                .vertex_position(mesh.to_vertex(corner).unwrap())
                .unwrap(),
            normal: [0.0; 3],
            existing: None,
        })
        .collect();
    let mut parameters = params(UvProjection::Spherical(UvFrame::XY));
    let expected = parameters
        .mapping
        .map_face(&inputs, parameters.options.seams, parameters.options.poles)
        .unwrap();
    let pole = inputs
        .iter()
        .position(|input| input.position[2] == 1.0)
        .unwrap();
    for (index, &corner) in corners.iter().enumerate() {
        if index != pole {
            op::set_corner_uv(&mut mesh.edit(), corner, [17.0; 2]).unwrap();
        }
    }
    parameters.write_missing_only = true;
    let output = map_uvs(&mut mesh.edit(), &parameters).unwrap();
    assert_eq!(output.corners_written, 1);
    assert_eq!(output.corners_skipped_existing, 2);
    assert_eq!(output.pole_corners_repaired, 1);
    assert_eq!(
        corner_uv(&mesh, UvSet::Primary, corners[pole]),
        Some(expected.uvs[pole])
    );
    for (index, &corner) in corners.iter().enumerate() {
        if index != pole {
            assert_eq!(corner_uv(&mesh, UvSet::Primary, corner), Some([17.0; 2]));
        }
    }
    let output = map_uvs(&mut mesh.edit(), &parameters).unwrap();
    assert_eq!(output.corners_written, 0);
    assert_eq!(output.corners_skipped_existing, 3);
    assert_eq!(output.pole_corners_repaired, 0);
}
