// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use exedra_mesh::{BuildParams, ExtractParams, NormalParams, NormalsSource, op};

fn step(distance: f64, length: f64) -> VertexStretchStep {
    VertexStretchStep {
        plane: Plane3 {
            normal: [1.0, 0.0, 0.0],
            distance,
        },
        length,
    }
}
fn triangle(points: [[f32; 3]; 3]) -> Mesh {
    Mesh::from_indexed_triangles(&points, &[[0, 1, 2]], &BuildParams::default()).unwrap()
}
fn points(mesh: &Mesh) -> Vec<[f32; 3]> {
    mesh.vertices()
        .map(|v| *mesh.vertex_position(v).unwrap())
        .collect()
}

#[test]
fn open_oblique_triangle_moves_vertices_without_a_band_or_topology_change() {
    let source = triangle([[0.0, 0.0, 0.0], [2.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    let before = exedra_testkit::dump_mesh_topology(&source);
    let result = stretch_vertices(&source, &[step(1.0, 1.0)], &Placement3::IDENTITY).unwrap();
    assert_eq!(
        points(&result),
        [[0.0, 0.0, 0.0], [3.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    );
    assert_eq!(
        source.faces().collect::<Vec<_>>(),
        result.faces().collect::<Vec<_>>()
    );
    for face in source.faces() {
        assert_eq!(
            source.face_loop(face).collect::<Vec<_>>(),
            result.face_loop(face).collect::<Vec<_>>()
        );
    }
    assert!(result.validate_deep().is_empty());
    assert_eq!(exedra_testkit::dump_mesh_topology(&source), before);
}

#[test]
fn contraction_classifies_original_positions_and_accumulates_once() {
    let source = triangle([[0.45, 0.0, 0.0], [0.45, 1.0, 0.0], [0.45, 0.0, 1.0]]);
    let result = stretch_vertices(
        &source,
        &[step(0.4, -0.3), step(0.2, -0.3)],
        &Placement3::IDENTITY,
    )
    .unwrap();
    for p in points(&result) {
        assert!((p[0] + 0.15).abs() < 1e-7);
    }
    // A contraction slab would consume this entirely positive triangle.
    assert_eq!(result.faces().count(), 1);
}

#[test]
fn on_plane_vertices_stay_put_and_plane_normalization_keeps_distance() {
    let source = triangle([[1.0, 0.0, 0.0], [2.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    let scaled = VertexStretchStep {
        plane: Plane3 {
            normal: [2.0, 0.0, 0.0],
            distance: 2.0,
        },
        length: 1.0,
    };
    let result = stretch_vertices(&source, &[scaled], &Placement3::IDENTITY).unwrap();
    assert_eq!(
        points(&result),
        [[1.0, 0.0, 0.0], [3.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    );
}

#[test]
fn affine_transport_selects_in_local_space_and_moves_in_the_forward_direction() {
    // Rotation, reflection, nonuniform scale and shear in one invertible frame.
    let placement = Placement3::from_axes(
        [0.0, -2.0, 0.0],
        [-3.0, 0.0, 0.0],
        [0.5, 0.0, 4.0],
        [7.0, 9.0, 11.0],
    );
    let local = triangle([[0.0, 0.0, 0.0], [2.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    let placed = crate::transform::transform(&local, &placement).unwrap();
    let actual = stretch_vertices(&placed, &[step(1.0, 0.5)], &placement).unwrap();
    let deformed = stretch_vertices(&local, &[step(1.0, 0.5)], &Placement3::IDENTITY).unwrap();
    let expected = crate::transform::transform(&deformed, &placement).unwrap();
    assert_eq!(points(&actual), points(&expected));
}

#[test]
fn deformed_normals_are_derived_while_rigid_overrides_uvs_and_regions_survive() {
    // Two triangles share an edge: one stays rigid, the other changes slope.
    let mut source = Mesh::from_indexed_triangles(
        &[
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [2.0, 0.0, 1.0],
        ],
        &[[0, 1, 2], [1, 0, 3]],
        &BuildParams::default(),
    )
    .unwrap();
    let faces = source.faces().collect::<Vec<_>>();
    let corners = faces
        .iter()
        .map(|&f| source.face_loop(f).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let mut edit = source.edit();
    for (face, region) in faces.iter().zip([7, 9]) {
        op::set_face_region(&mut edit, *face, region).unwrap();
    }
    op::set_edge_sharpness(&mut edit, corners[0][0], 2.0).unwrap();
    op::set_edge_seam(&mut edit, corners[0][0], true).unwrap();
    for face_corners in &corners {
        for &corner in face_corners {
            op::set_corner_normal_override(&mut edit, corner, Some([0.0, 0.0, 1.0])).unwrap();
            op::set_corner_uv(&mut edit, corner, [0.25, 0.75]).unwrap();
        }
    }
    let _: () = edit.finish();
    let result = stretch_vertices(&source, &[step(0.5, 1.0)], &Placement3::IDENTITY).unwrap();
    assert_eq!(result.edge_sharpness(corners[0][0]), Some(2.0));
    assert_eq!(result.edge_seam(corners[0][0]), Some(true));
    let normals = result
        .attrs()
        .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
        .unwrap();
    for &corner in &corners[0] {
        assert_eq!(normals.get(corner.into()), Some(&[0.0, 0.0, 1.0]));
    }
    for &corner in &corners[1] {
        assert_eq!(normals.get(corner.into()), None);
    }
    for face_corners in &corners {
        for &corner in face_corners {
            assert_eq!(
                result
                    .attrs()
                    .sparse(exedra_mesh::attr::CORNER_UV)
                    .unwrap()
                    .get(corner.into()),
                Some(&[0.25, 0.75])
            );
        }
    }
    // An explicit threshold derives the changed face independently of its
    // neighbor; no authored override may mask its new geometric normal.
    let rendered = result.to_trimesh(&ExtractParams {
        normals: NormalsSource::CustomOrDerived,
        normal_params: NormalParams {
            auto_sharp_angle_degrees: Some(1.0),
            ..NormalParams::default()
        },
        ..ExtractParams::default()
    });
    let expected = exedra_math::normalize([-1.0_f32, 0.0, 3.0]).unwrap();
    assert!(
        rendered
            .0
            .normals
            .iter()
            .any(|n| n.iter().zip(expected).all(|(a, b)| (*a - b).abs() < 1e-6))
    );
    for face in faces {
        assert_eq!(
            source
                .attrs()
                .dense(exedra_mesh::attr::FACE_REGION)
                .unwrap()
                .get(face.into()),
            result
                .attrs()
                .dense(exedra_mesh::attr::FACE_REGION)
                .unwrap()
                .get(face.into())
        );
    }
    let zero = stretch_vertices(&source, &[step(0.5, 0.0)], &Placement3::IDENTITY).unwrap();
    assert_eq!(
        exedra_testkit::dump_attributes(&source),
        exedra_testkit::dump_attributes(&zero)
    );
}

#[test]
fn failures_are_atomic_and_zero_preserves_polygon_input() {
    let source = triangle([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
    let before = exedra_testkit::dump_mesh_topology(&source);
    let face = source.faces().next().unwrap();
    assert_eq!(
        stretch_vertices(&source, &[step(0.5, -1.0)], &Placement3::IDENTITY).unwrap_err(),
        VertexStretchError::DegenerateTriangle(face)
    );
    assert_eq!(
        stretch_vertices(&source, &[step(-1.0, f64::MAX)], &Placement3::IDENTITY).unwrap_err(),
        VertexStretchError::NumericLimit
    );
    assert_eq!(
        stretch_vertices(&source, &[], &Placement3::IDENTITY).unwrap_err(),
        VertexStretchError::InvalidInput
    );
    for bad in [f64::NAN, f64::INFINITY] {
        assert_eq!(
            stretch_vertices(&source, &[step(0.5, bad)], &Placement3::IDENTITY).unwrap_err(),
            VertexStretchError::InvalidInput
        );
    }
    let singular = Placement3::from_axes([0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]);
    assert_eq!(
        stretch_vertices(&source, &[step(0.5, 1.0)], &singular).unwrap_err(),
        VertexStretchError::SingularTransform
    );
    assert_eq!(exedra_testkit::dump_mesh_topology(&source), before);
    let quad = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        &[&[0, 1, 2, 3]],
    )
    .unwrap();
    assert!(matches!(
        stretch_vertices(&quad, &[step(0.5, 1.0)], &Placement3::IDENTITY),
        Err(VertexStretchError::UnsupportedFace(_))
    ));
    let unchanged = stretch_vertices(&quad, &[step(0.5, 0.0)], &Placement3::IDENTITY).unwrap();
    assert_eq!(
        exedra_testkit::dump_mesh_topology(&quad),
        exedra_testkit::dump_mesh_topology(&unchanged)
    );
}

#[test]
fn tiny_nonzero_triangles_survive_but_float_storage_collapse_is_refused() {
    let small = triangle([[0.0, 0.0, 0.0], [1e-30, 0.0, 0.0], [0.0, 1e-30, 0.0]]);
    assert!(stretch_vertices(&small, &[step(1.0, 1.0)], &Placement3::IDENTITY).is_ok());
    let source = triangle([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
    assert!(matches!(
        stretch_vertices(&source, &[step(-1.0, 1e20)], &Placement3::IDENTITY),
        Err(VertexStretchError::DegenerateTriangle(_))
    ));
}

#[test]
fn oblique_authored_boundary_and_nearest_stored_neighbors_are_classified_exactly() {
    let plane = Plane3 {
        normal: [1.0; 3],
        distance: 3.0,
    };
    for x in [1.0_f32.next_down(), 1.0, 1.0_f32.next_up()] {
        let source = triangle([[x, 1.0, 1.0], [2.0, 1.0, 0.0], [0.0, 2.0, 1.0]]);
        let result = stretch_vertices(
            &source,
            &[VertexStretchStep { plane, length: 1.0 }],
            &Placement3::IDENTITY,
        )
        .unwrap();
        let original = points(&source);
        let moved = points(&result);
        assert_eq!(&moved[1..], &original[1..]);
        if x > 1.0 {
            assert!(moved[0][0] > 1.5);
        } else {
            assert_eq!(moved, original);
        }
    }
}

fn author_normals(mesh: &mut Mesh) {
    let derived = mesh.derive_corner_normals(&NormalParams::default());
    let corners = mesh
        .faces()
        .flat_map(|f| mesh.face_loop(f))
        .collect::<Vec<_>>();
    let mut edit = mesh.edit();
    for corner in corners {
        op::set_corner_normal_override(&mut edit, corner, derived.get(corner)).unwrap();
    }
    let _: () = edit.finish();
}

#[test]
fn normals_follow_stored_geometry_instead_of_requested_displacement() {
    let mut source = triangle([[0.0, 0.0, 0.0], [2.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    author_normals(&mut source);
    let unchanged = stretch_vertices(&source, &[step(1.0, 1e-8)], &Placement3::IDENTITY).unwrap();
    assert_eq!(points(&source), points(&unchanged));
    assert_eq!(
        exedra_testkit::dump_attributes(&source),
        exedra_testkit::dump_attributes(&unchanged)
    );
    let rigid = stretch_vertices(&source, &[step(-1.0, 0.5)], &Placement3::IDENTITY).unwrap();
    for face in source.faces() {
        for corner in source.face_loop(face) {
            assert_eq!(
                source
                    .attrs()
                    .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                    .unwrap()
                    .get(corner.into()),
                rigid
                    .attrs()
                    .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                    .unwrap()
                    .get(corner.into())
            );
        }
    }
    let mut source = triangle([[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    author_normals(&mut source);
    // At 2^100 even f64(new)-f64(old) loses the difference between movements.
    for length in [16777216.0, f64::from_bits((1023 + 100) << 52)] {
        let result =
            stretch_vertices(&source, &[step(-1.0, length)], &Placement3::IDENTITY).unwrap();
        for face in result.faces() {
            for corner in result.face_loop(face) {
                assert!(
                    result
                        .attrs()
                        .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                        .unwrap()
                        .get(corner.into())
                        .is_none()
                );
            }
        }
        let (rendered, _) = result.to_trimesh(&ExtractParams {
            normals: NormalsSource::CustomOrDerived,
            ..Default::default()
        });
        assert!(rendered.normals.iter().all(|n| *n == [1.0, 0.0, 0.0]));
    }
}

#[test]
fn face_local_overrides_do_not_infer_a_shared_authored_smoothing_policy() {
    let mut source = Mesh::from_indexed_triangles(
        &[
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [2.0, 0.0, 1.0],
        ],
        &[[0, 1, 2], [1, 0, 3]],
        &BuildParams::default(),
    )
    .unwrap();
    // Connected smooth faces, with no sharp edge or UV seam at their boundary.
    author_normals(&mut source);
    let result = stretch_vertices(&source, &[step(0.5, 1.0)], &Placement3::IDENTITY).unwrap();
    let at_origin = |mesh: &Mesh, normals| {
        let (rendered, _) = mesh.to_trimesh(&ExtractParams {
            normals,
            ..Default::default()
        });
        rendered
            .positions
            .iter()
            .zip(&rendered.normals)
            .filter_map(|(p, n)| (*p == [0.0; 3]).then_some(*n))
            .collect::<Vec<_>>()
    };
    let original = at_origin(&source, NormalsSource::CustomOrDerived);
    assert!(original.iter().all(|n| *n == original[0]));
    let mixed = at_origin(&result, NormalsSource::CustomOrDerived);
    assert!(mixed.contains(&original[0]));
    assert!(mixed.iter().any(|n| *n != original[0]));
    // A caller choosing full derivation gets one consistent smooth neighborhood.
    let derived = at_origin(&result, NormalsSource::Derived);
    assert!(!derived.is_empty());
    assert!(derived.iter().all(|n| *n == derived[0]));
    assert_ne!(derived[0], original[0]);
}
