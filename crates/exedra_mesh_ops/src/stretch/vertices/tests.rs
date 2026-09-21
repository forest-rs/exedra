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
