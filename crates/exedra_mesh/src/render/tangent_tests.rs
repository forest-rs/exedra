// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! MikkTSpace tangent generation through render extraction.

use alloc::vec;
use alloc::vec::Vec;

use exedra_math::{cross, dot, norm};

use crate::math::FloatExt;

use crate::{
    AttributeBuffer, ExtractAttribute, ExtractParams, Mesh, MeshBuilder, NormalsSource, TangentUv,
    TriMesh, attr, op,
};

fn with_tangents(uv_set: TangentUv) -> ExtractParams {
    ExtractParams {
        tangents: Some(uv_set),
        ..ExtractParams::default()
    }
}

/// Builds a mesh from quads and assigns each corner the UV `uv(position)`,
/// evaluated per face so seams can differ between faces.
fn quads(
    points: &[[f32; 3]],
    faces: &[[u32; 4]],
    uv: impl Fn(usize, [f32; 3]) -> [f32; 2],
) -> Mesh {
    let mut builder = MeshBuilder::new();
    for p in points {
        builder.push_vertex(*p);
    }
    for face in faces {
        builder.add_face(face).expect("face");
    }
    let built = builder.build().expect("build");
    let mut mesh = built.mesh;
    let assignments: Vec<_> = built
        .face_ids
        .iter()
        .enumerate()
        .flat_map(|(f, &face)| {
            mesh.face_loop(face)
                .map(|corner| {
                    let v = mesh.to_vertex(corner).expect("vertex");
                    (corner, uv(f, *mesh.vertex_position(v).expect("position")))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let mut edit = mesh.edit();
    for (corner, value) in assignments {
        op::set_corner_uv(&mut edit, corner, value).expect("uv");
    }
    let _: () = edit.finish();
    mesh
}

fn unit_quad(uv: impl Fn(usize, [f32; 3]) -> [f32; 2]) -> Mesh {
    quads(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        &[[0, 1, 2, 3]],
        uv,
    )
}

fn assert_frame(tri: &TriMesh) {
    assert_eq!(tri.tangents.len(), tri.positions.len());
    for (t, n) in tri.tangents.iter().zip(&tri.normals) {
        let xyz = [t[0], t[1], t[2]];
        assert!((norm(xyz) - 1.0).abs() < 1e-5, "unit tangent {t:?}");
        assert!(
            dot(xyz, *n).abs() < 1e-5,
            "tangent {t:?} lies in the normal plane"
        );
        assert!(t[3] == 1.0 || t[3] == -1.0, "handedness {t:?}");
    }
}

#[test]
fn no_request_emits_no_tangents() {
    let mesh = unit_quad(|_, p| [p[0], p[1]]);
    let (tri, stats) = mesh.to_trimesh(&ExtractParams::default());
    assert!(tri.tangents.is_empty());
    assert_eq!(stats.tangent_split_count, 0);
}

#[test]
fn planar_uvs_align_tangents_with_u() {
    let mesh = unit_quad(|_, p| [p[0], p[1]]);
    let (tri, stats) = mesh.to_trimesh(&with_tangents(TangentUv::Primary));
    assert_frame(&tri);
    assert_eq!(tri.tangents, vec![[1.0, 0.0, 0.0, 1.0]; 4]);
    assert_eq!(stats.render_vertex_count, 4);
    assert_eq!(stats.tangent_split_count, 0);
    assert_eq!(stats.tangent_fallback_count, 0);
    // The rest of the output is unchanged by the request.
    let (plain, _) = mesh.to_trimesh(&ExtractParams::default());
    assert_eq!(
        (&tri.indices, &tri.positions, &tri.uvs, &tri.normals),
        (&plain.indices, &plain.positions, &plain.uvs, &plain.normals)
    );
}

#[test]
fn mirrored_uvs_flip_handedness() {
    // u runs along -x: the tangent follows u and the sign restores +v as the
    // bitangent.
    let mesh = unit_quad(|_, p| [-p[0], p[1]]);
    let (tri, _) = mesh.to_trimesh(&with_tangents(TangentUv::Primary));
    assert_frame(&tri);
    for (t, n) in tri.tangents.iter().zip(&tri.normals) {
        assert_eq!(*t, [-1.0, 0.0, 0.0, -1.0]);
        let bitangent = cross(*n, [t[0], t[1], t[2]]).map(|c| c * t[3]);
        assert_eq!(bitangent, [0.0, 1.0, 0.0]);
    }
}

#[test]
fn mirror_seam_splits_shared_render_vertices() {
    // Two quads share the edge x = 1. The right quad mirrors u about that
    // edge, so the shared corners have equal UVs and normals (one render
    // vertex each before tangents) but opposite handedness.
    let mesh = quads(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [2.0, 1.0, 0.0],
        ],
        &[[0, 1, 4, 3], [1, 2, 5, 4]],
        |f, p| {
            if f == 0 {
                [p[0], p[1]]
            } else {
                [2.0 - p[0], p[1]]
            }
        },
    );
    let (plain, _) = mesh.to_trimesh(&ExtractParams::default());
    assert_eq!(plain.positions.len(), 6);
    let (tri, stats) = mesh.to_trimesh(&with_tangents(TangentUv::Primary));
    assert_frame(&tri);
    assert_eq!(stats.tangent_split_count, 2);
    assert_eq!(stats.render_vertex_count, 8);
    // Every triangle of the left quad is right-handed, of the right quad left.
    for (t, triangle) in tri.indices.chunks(3).enumerate() {
        let expected = if t < 2 {
            [1.0, 0.0, 0.0, 1.0]
        } else {
            [-1.0, 0.0, 0.0, -1.0]
        };
        for &i in triangle {
            assert_eq!(tri.tangents[i as usize], expected);
        }
    }
}

#[test]
fn cylinder_tangents_follow_the_circumference_across_the_seam() {
    let segments = 8_u32;
    let mut points = Vec::new();
    for i in 0..segments {
        let a = core::f32::consts::TAU * i as f32 / segments as f32;
        let (s, c) = (a.sin_ext(), a.cos_ext());
        points.push([c, s, 0.0]);
        points.push([c, s, 1.0]);
    }
    let faces: Vec<[u32; 4]> = (0..segments)
        .map(|i| {
            let j = (i + 1) % segments;
            [2 * i, 2 * j, 2 * j + 1, 2 * i + 1]
        })
        .collect();
    let mesh = quads(&points, &faces, |f, p| {
        // u = segment fraction; the last face closes the seam at u = 1.
        let a = p[1].atan2_ext(p[0]).rem_euclid(core::f32::consts::TAU);
        let mut u = a / core::f32::consts::TAU;
        if f + 1 == segments as usize && u < 0.5 {
            u = 1.0;
        }
        [u, p[2]]
    });
    let (tri, stats) = mesh.to_trimesh(&with_tangents(TangentUv::Primary));
    assert_frame(&tri);
    assert_eq!(stats.tangent_fallback_count, 0);
    for (t, p) in tri.tangents.iter().zip(&tri.positions) {
        // Counter-clockwise circumference direction at this point.
        let expected = [-p[1], p[0], 0.0];
        assert!(dot([t[0], t[1], t[2]], expected) > 0.9, "{t:?} at {p:?}");
        assert_eq!(t[3], 1.0);
    }
    // Seam render vertices (u = 0 and u = 1 at angle 0) agree closely.
    let seam: Vec<_> = tri
        .positions
        .iter()
        .zip(&tri.tangents)
        .filter(|(p, _)| p[0] == 1.0 && p[2] == 0.0)
        .map(|(_, t)| *t)
        .collect();
    assert_eq!(seam.len(), 2, "the seam splits by UV");
    assert!(
        dot(
            [seam[0][0], seam[0][1], seam[0][2]],
            [seam[1][0], seam[1][1], seam[1][2]]
        ) > 0.9
    );
}

#[test]
fn missing_uvs_fall_back_perpendicular_to_the_normal() {
    let mut builder = MeshBuilder::new();
    for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        builder.push_vertex(p);
    }
    builder.add_face(&[0, 1, 2]).expect("face");
    let mesh = builder.build().expect("build").mesh;
    let (tri, stats) = mesh.to_trimesh(&with_tangents(TangentUv::Primary));
    assert_frame(&tri);
    assert_eq!(stats.tangent_fallback_count, 3);
    assert_eq!(stats.missing_tangent_uv_sets, 0);
    assert_eq!(tri.tangents, vec![[1.0, 0.0, 0.0, 1.0]; 3]);

    let (tri, stats) = mesh.to_trimesh(&with_tangents(TangentUv::Attribute(attr::CORNER_UV1)));
    assert_eq!(stats.missing_tangent_uv_sets, 1);
    assert_eq!(stats.tangent_fallback_count, 3);
    assert_frame(&tri);
}

#[test]
fn zero_normals_still_yield_unit_tangents() {
    let mesh = unit_quad(|_, p| [p[0], p[1]]);
    let params = ExtractParams {
        normals: NormalsSource::CustomOnly,
        tangents: Some(TangentUv::Primary),
        ..ExtractParams::default()
    };
    let (tri, _) = mesh.to_trimesh(&params);
    for t in &tri.tangents {
        assert!((norm([t[0], t[1], t[2]]) - 1.0).abs() < 1e-5);
    }
}

#[test]
fn tangents_can_follow_a_carried_uv_set() {
    // UV0 runs along x; UV1 along y. Tangents follow the requested set and
    // the carried stream stays parallel to the output vertices.
    let mut mesh = unit_quad(|_, p| [p[0], p[1]]);
    let face = mesh.faces().next().expect("face");
    let corners: Vec<_> = mesh.face_loop(face).collect();
    let mut edit = mesh.edit();
    for corner in corners {
        let v = edit.mesh().to_vertex(corner).expect("vertex");
        let p = *edit.mesh().vertex_position(v).expect("position");
        op::set_attribute(&mut edit, attr::CORNER_UV1, corner, [p[1], p[0]]).expect("uv1");
    }
    let _: () = edit.finish();
    let params = ExtractParams {
        attributes: vec![ExtractAttribute::new(attr::CORNER_UV1, [0.0, 0.0])],
        tangents: Some(TangentUv::Attribute(attr::CORNER_UV1)),
        ..ExtractParams::default()
    };
    let (tri, stats) = mesh.to_trimesh(&params);
    assert_frame(&tri);
    assert_eq!(stats.tangent_fallback_count, 0);
    for t in &tri.tangents {
        // u1 = y, v1 = x on a +z face: left-handed.
        assert_eq!(*t, [0.0, 1.0, 0.0, -1.0]);
    }
    let Some(AttributeBuffer::Vec2(uv1)) = tri.attribute(attr::CORNER_UV1) else {
        panic!("uv1 stream");
    };
    for (p, uv) in tri.positions.iter().zip(uv1) {
        assert_eq!(*uv, [p[1], p[0]]);
    }
}

#[test]
fn tangent_extraction_is_deterministic() {
    let mesh = quads(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.2],
            [2.0, 0.0, 0.0],
            [0.0, 1.0, 0.1],
            [1.0, 1.0, 0.5],
            [2.0, 1.0, 0.0],
        ],
        &[[0, 1, 4, 3], [1, 2, 5, 4]],
        |_, p| [p[0] * 0.5, p[1] * 2.0],
    );
    let params = with_tangents(TangentUv::Primary);
    let (a, _) = mesh.to_trimesh(&params);
    let (b, _) = mesh.to_trimesh(&params);
    assert_eq!(a, b);
    assert_frame(&a);
}

/// Two planar triangles sharing edge `1-2`: `[0, 1, 2]` has collinear UVs
/// (no usable gradient), `[1, 3, 2]` is mapped.
const SHARED_POSITIONS: [[f32; 3]; 4] = [
    [0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [1.0, 1.0, 0.0],
];
const SHARED_NORMALS: [[f32; 3]; 4] = [[0.0, 0.0, 1.0]; 4];
const SHARED_UVS: [[f32; 2]; 4] = [[0.5, 0.5], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];

#[test]
fn groups_do_not_depend_on_face_order() {
    let run = |indices: &[u32]| {
        super::tangents::corner_tangents(indices, &SHARED_POSITIONS, &SHARED_NORMALS, &SHARED_UVS)
    };
    let unmapped_first = run(&[0, 1, 2, 1, 3, 2]);
    let mapped_first = run(&[1, 3, 2, 0, 1, 2]);
    // Corner `c` of the unmapped-first order is corner `(c + 3) % 6` of the
    // other: same triangle, same vertex.
    for c in 0..6 {
        let d = (c + 3) % 6;
        assert_eq!(unmapped_first.tangents[c], mapped_first.tangents[d]);
        assert_eq!(unmapped_first.fallback[c], mapped_first.fallback[d]);
    }
    // Only the unmapped triangle's private corner (vertex 0) is unreached.
    assert_eq!(
        unmapped_first.fallback,
        [true, false, false, false, false, false]
    );
    // The unmapped triangle joins the mapped group on the shared vertices.
    assert_eq!(unmapped_first.tangents[1], unmapped_first.tangents[3]);
    assert_eq!(unmapped_first.tangents[2], unmapped_first.tangents[5]);
}

#[test]
fn degenerate_triangles_copy_from_a_valid_corner() {
    // `[0, 1, 1]` welds two corners together; it copies the valid
    // triangle's tangents for vertices 0 and 1.
    let indices = [0, 1, 2, 0, 1, 1];
    let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals = [[0.0, 0.0, 1.0]; 3];
    let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    let result = super::tangents::corner_tangents(&indices, &positions, &normals, &uvs);
    assert!(result.fallback.iter().all(|f| !f));
    assert_eq!(result.tangents[3], result.tangents[0]);
    assert_eq!(result.tangents[4], result.tangents[1]);
    assert_eq!(result.tangents[5], result.tangents[1]);
    assert_eq!(result.tangents[0], [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn tangent_requests_key_the_extraction_cache() {
    let mesh = unit_quad(|_, p| [p[0], p[1]]);
    let mut cache = crate::TrimeshCache::new();
    let tangents = with_tangents(TangentUv::Primary);
    let (first, _) = mesh.to_trimesh_cached(&tangents, &mut cache);
    let (again, stats) = mesh.to_trimesh_cached(&tangents, &mut cache);
    assert_eq!(stats.full_reuses, 1);
    assert_eq!(again, first);
    let (plain, stats) = mesh.to_trimesh_cached(&ExtractParams::default(), &mut cache);
    assert_eq!(stats.full_reuses, 0);
    assert_eq!(stats.incremental_fallbacks, 1);
    assert!(plain.tangents.is_empty());
}
