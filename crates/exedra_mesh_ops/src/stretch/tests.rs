// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

fn cube() -> Mesh {
    Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
            [0.0, 0.0, 2.0],
            [2.0, 0.0, 2.0],
            [2.0, 2.0, 2.0],
            [0.0, 2.0, 2.0],
        ],
        &[
            &[3, 2, 1, 0],
            &[4, 5, 6, 7],
            &[0, 1, 5, 4],
            &[1, 2, 6, 5],
            &[2, 3, 7, 6],
            &[3, 0, 4, 7],
        ],
    )
    .unwrap()
}

#[test]
fn direct_expansion_and_contraction_return_complete_source_correspondence() {
    let source = cube();
    for (length, width, bands) in [(2.0, 4.0, 4), (-0.5, 1.5, 0)] {
        let result = stretch_mesh(
            &source,
            &Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 1.0,
            },
            length,
            &Placement3::IDENTITY,
            &StretchPolicy::default(),
        )
        .unwrap();
        assert!(result.topology_rebuilt);
        assert!(result.mesh.validate_deep().is_empty());
        assert!(!has_open_boundary(&result.mesh));
        assert_eq!(result.stats.band_faces, bands);
        assert_eq!(result.face_sources.len(), result.mesh.faces().count());
        assert_eq!(result.vertex_sources.len(), result.mesh.vertices().count());
        let mut max_x = 0.0_f32;
        let mut seam_vertices = 0;
        for vertex in result.mesh.vertices() {
            let point = result.mesh.vertex_position(vertex).unwrap();
            max_x = max_x.max(point[0]);
            match result.vertex_sources[&vertex] {
                StretchVertexSource::Original(original) => {
                    let old = source.vertex_position(original).unwrap();
                    assert_eq!(&point[1..], &old[1..]);
                    assert_eq!(
                        f64::from(point[0]),
                        f64::from(old[0]) + if old[0] > 1.0 { length } else { 0.0 }
                    );
                }
                StretchVertexSource::Seam { .. } => seam_vertices += 1,
            }
        }
        assert!(seam_vertices > 0);
        assert_eq!(max_x, width);
        assert_eq!(
            result
                .face_sources
                .values()
                .filter(|source| matches!(source, StretchFaceSource::Band(_)))
                .count() as u64,
            bands
        );
        for from in result.face_sources.values() {
            assert!(source.face_edge(from.face()).is_some());
        }
    }
}

#[test]
fn raw_stretch_rejects_invalid_inputs_and_translation_overflow_without_mutation() {
    let source = cube();
    let before = source.to_trimesh(&exedra_mesh::ExtractParams::default());
    let plane = Plane3 {
        normal: [1.0, 0.0, 0.0],
        distance: -1.0,
    };
    for length in [0.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            stretch_mesh(
                &source,
                &plane,
                length,
                &Placement3::IDENTITY,
                &StretchPolicy::default()
            )
            .unwrap_err(),
            StretchError::InvalidInput
        );
    }
    assert_eq!(
        stretch_mesh(
            &source,
            &Plane3 {
                normal: [0.0; 3],
                distance: 0.0
            },
            1.0,
            &Placement3::IDENTITY,
            &StretchPolicy::default()
        )
        .unwrap_err(),
        StretchError::InvalidInput
    );
    assert_eq!(
        stretch_mesh(
            &source,
            &plane,
            f64::MAX,
            &Placement3::IDENTITY,
            &StretchPolicy::default()
        )
        .unwrap_err(),
        StretchError::NumericLimit
    );
    assert_eq!(
        source.to_trimesh(&exedra_mesh::ExtractParams::default()),
        before
    );
    let result = stretch_mesh(
        &source,
        &plane,
        2.0,
        &Placement3::IDENTITY,
        &StretchPolicy::default(),
    )
    .unwrap();
    assert!(!result.topology_rebuilt);
    for (vertex, from) in result.vertex_sources {
        assert_eq!(from, StretchVertexSource::Original(vertex));
    }
}

/// A cube whose corner UVs encode their destination vertex position (y, z)
/// for faces on the unmoved side.
fn uv_cube() -> Mesh {
    let mut mesh = cube();
    let corners: Vec<_> = mesh.faces().flat_map(|f| mesh.face_loop(f)).collect();
    let mut edit = mesh.edit();
    for corner in corners {
        let vertex = edit.mesh().to_vertex(corner).unwrap();
        let p = *edit.mesh().vertex_position(vertex).unwrap();
        exedra_mesh::op::set_corner_uv(&mut edit, corner, [p[1] + 10.0 * p[0], p[2]]).unwrap();
    }
    let _: () = edit.finish();
    mesh
}

#[test]
fn rebuilt_corner_uvs_stay_at_their_own_vertex() {
    let source = uv_cube();
    let result = stretch_mesh(
        &source,
        &Plane3 {
            normal: [1.0, 0.0, 0.0],
            distance: 1.0,
        },
        2.0,
        &Placement3::IDENTITY,
        &StretchPolicy::default(),
    )
    .unwrap();
    assert!(result.topology_rebuilt);
    let mesh = &result.mesh;
    let uvs = mesh.attrs().sparse(exedra_mesh::attr::CORNER_UV).unwrap();
    let mut checked = 0;
    for face in mesh.faces() {
        for corner in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(corner).unwrap();
            let p = *mesh.vertex_position(vertex).unwrap();
            // Unmoved original vertices keep their authored UV exactly.
            if matches!(
                result.vertex_sources[&vertex],
                StretchVertexSource::Original(_)
            ) && p[0] < 1.0
            {
                assert_eq!(
                    uvs.get(corner.as_id()),
                    Some(&[p[1] + 10.0 * p[0], p[2]]),
                    "corner at {p:?}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0);
}
