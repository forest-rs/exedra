// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Carried attribute streams through render extraction.

use alloc::vec;
use alloc::vec::Vec;

use crate::attributes::{AttrKey, Domain};
use crate::{
    AttributeBuffer, ExtractAttribute, ExtractParams, FaceId, Mesh, MeshBuilder, TrimeshCache,
    VertexId, attr,
};

/// Three triangles fanned around vertex 0, all in the XY plane.
fn fan() -> (Mesh, Vec<FaceId>) {
    let mut builder = MeshBuilder::new();
    for p in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
        builder.push_vertex([p[0], p[1], 0.0]);
    }
    builder.add_face(&[0, 1, 2]).expect("face");
    builder.add_face(&[0, 2, 3]).expect("face");
    builder.add_face(&[0, 3, 4]).expect("face");
    let built = builder.build().expect("build");
    (built.mesh, built.face_ids)
}

fn params(attributes: Vec<ExtractAttribute>) -> ExtractParams {
    ExtractParams {
        attributes,
        ..ExtractParams::default()
    }
}

fn corner_at(mesh: &Mesh, face: FaceId, vertex: u32) -> crate::CornerId {
    mesh.face_loop(face)
        .find(|corner| mesh.to_vertex(*corner).is_some_and(|v| v.index() == vertex))
        .expect("corner")
}

#[test]
fn no_attributes_matches_historical_output() {
    let (mesh, _) = fan();
    let (tri, stats) = mesh.to_trimesh(&ExtractParams::default());
    assert!(tri.attributes.is_empty());
    assert_eq!(stats.render_vertex_count, 5);
    assert_eq!(stats.attribute_split_count, 0);
    assert_eq!(stats.missing_attribute_layers, 0);
}

#[test]
fn corner_colors_split_shared_vertices() {
    let (mut mesh, faces) = fan();
    mesh.attrs_mut()
        .define_sparse(attr::CORNER_COLOR)
        .expect("define");
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    let mut values = Vec::new();
    for (face, color) in faces.iter().zip([red, blue, blue]) {
        for corner in mesh.face_loop(*face).collect::<Vec<_>>() {
            values.push((corner, color));
        }
    }
    let layer = mesh
        .attrs_mut()
        .sparse_mut(attr::CORNER_COLOR)
        .expect("layer");
    for (corner, color) in values {
        layer.set(corner.as_id(), color);
    }

    let (tri, stats) = mesh.to_trimesh(&params(vec![ExtractAttribute::new(
        attr::CORNER_COLOR,
        [0.0; 4],
    )]));
    // Vertices 0 and 2 each appear red and blue.
    assert_eq!(stats.render_vertex_count, 7);
    assert_eq!(stats.attribute_split_count, 2);
    assert_eq!(stats.uv_split_count, 0);
    assert_eq!(stats.normal_split_count, 0);
    assert_eq!(stats.split_count, 2);
    assert_eq!(stats.attribute_fallback_count, 0);
    let Some(AttributeBuffer::Vec4(colors)) = tri.attribute(attr::CORNER_COLOR) else {
        panic!("color stream");
    };
    assert_eq!(colors.len(), tri.positions.len());
    for (t, expected) in [red, blue, blue].into_iter().enumerate() {
        for k in 0..3 {
            assert_eq!(colors[tri.indices[t * 3 + k] as usize], expected);
        }
    }
    let (again, _) = mesh.to_trimesh(&params(vec![ExtractAttribute::new(
        attr::CORNER_COLOR,
        [0.0; 4],
    )]));
    assert_eq!(tri, again, "extraction is deterministic");
}

#[test]
fn reuse_follows_the_matching_render_vertex_in_a_chain() {
    // Face values 1, 2, 2: the third face must reuse the second face's
    // render vertices at the shared vertices, not the first chain entry.
    let key = AttrKey::<u32>::new(Domain::Face, "face.tag");
    let (mut mesh, faces) = fan();
    mesh.attrs_mut().define_dense(key, 0).expect("define");
    let layer = mesh.attrs_mut().dense_mut(key).expect("layer");
    for (face, tag) in faces.iter().zip([1, 2, 2]) {
        assert!(layer.set(face.as_id(), tag));
    }

    let (tri, stats) = mesh.to_trimesh(&params(vec![ExtractAttribute::new(key, 0)]));
    // Vertex 0 has two variants; vertices 2 (faces 0/1) splits; vertex 3
    // (faces 1/2) is shared by equal tags.
    assert_eq!(stats.render_vertex_count, 7);
    assert_eq!(stats.attribute_split_count, 2);
    let center = |t: usize| {
        (0..3)
            .map(|k| tri.indices[t * 3 + k])
            .find(|&i| tri.positions[i as usize] == [0.0, 0.0, 0.0])
            .expect("center")
    };
    assert_ne!(center(0), center(1));
    assert_eq!(center(1), center(2));
    assert_eq!(
        tri.attribute(key),
        Some(&AttributeBuffer::U32(tri.indices.iter().enumerate().fold(
            vec![0; 7],
            |mut out, (offset, &i)| {
                out[i as usize] = [1, 2, 2][offset / 3];
                out
            }
        )))
    );
}

#[test]
fn vertex_domain_values_never_split() {
    let key = AttrKey::<[f32; 4]>::new(Domain::Vertex, "vertex.pivot");
    let (mut mesh, _) = fan();
    mesh.attrs_mut()
        .define_dense(key, [0.0; 4])
        .expect("define");
    let layer = mesh.attrs_mut().dense_mut(key).expect("layer");
    for index in 0..5 {
        let id = VertexId::from(crate::Id::new(index, core::num::NonZeroU32::MIN));
        assert!(layer.set(id.as_id(), [index as f32, 0.0, 0.0, 1.0]));
    }

    let (tri, stats) = mesh.to_trimesh(&params(vec![ExtractAttribute::new(key, [0.0; 4])]));
    assert_eq!(stats.render_vertex_count, 5);
    assert_eq!(stats.split_count, 0);
    let Some(AttributeBuffer::Vec4(pivots)) = tri.attribute(key) else {
        panic!("pivot stream");
    };
    for (position, pivot) in tri.positions.iter().zip(pivots) {
        let expected = match (position[0], position[1]) {
            (0.0, 0.0) => 0.0,
            (1.0, 0.0) => 1.0,
            (0.0, 1.0) => 2.0,
            (-1.0, 0.0) => 3.0,
            _ => 4.0,
        };
        assert_eq!(*pivot, [expected, 0.0, 0.0, 1.0]);
    }
}

#[test]
fn sparse_gaps_and_missing_layers_use_the_missing_value() {
    let (mut mesh, faces) = fan();
    mesh.attrs_mut()
        .define_sparse(attr::CORNER_UV1)
        .expect("define");
    let corner = corner_at(&mesh, faces[0], 1);
    mesh.attrs_mut()
        .sparse_mut(attr::CORNER_UV1)
        .expect("layer")
        .set(corner.as_id(), [0.5, 0.25]);
    // A layer registered under the requested name with another type is
    // reported as missing, not reinterpreted.
    let mistyped = AttrKey::<f32>::new(Domain::HalfEdge, "corner.mistyped");
    mesh.attrs_mut().define_sparse(mistyped).expect("define");

    let (tri, stats) = mesh.to_trimesh(&params(vec![
        ExtractAttribute::new(attr::CORNER_UV1, [-1.0, -1.0]),
        ExtractAttribute::new(AttrKey::<u32>::new(Domain::Vertex, "vertex.absent"), 9),
        ExtractAttribute::new(
            AttrKey::<[f32; 2]>::new(Domain::HalfEdge, "corner.mistyped"),
            [7.0, 7.0],
        ),
    ]));
    assert_eq!(stats.render_vertex_count, 5);
    assert_eq!(stats.missing_attribute_layers, 2);
    // Four UV1 gaps plus five values for each of the two unbound streams.
    assert_eq!(stats.attribute_fallback_count, 4 + 5 + 5);
    let names: Vec<_> = tri.attributes.iter().map(|stream| stream.name).collect();
    assert_eq!(names, ["corner.uv1", "vertex.absent", "corner.mistyped"]);
    let Some(AttributeBuffer::Vec2(uv1)) = tri.attribute(attr::CORNER_UV1) else {
        panic!("uv1 stream");
    };
    let authored = uv1.iter().filter(|uv| **uv == [0.5, 0.25]).count();
    assert_eq!(authored, 1);
    assert_eq!(uv1.iter().filter(|uv| **uv == [-1.0, -1.0]).count(), 4);
    assert_eq!(
        tri.attribute(AttrKey::<u32>::new(Domain::Vertex, "vertex.absent")),
        Some(&AttributeBuffer::U32(vec![9; 5]))
    );
    assert_eq!(
        tri.attribute(AttrKey::<[f32; 2]>::new(
            Domain::HalfEdge,
            "corner.mistyped"
        )),
        Some(&AttributeBuffer::Vec2(vec![[7.0, 7.0]; 5]))
    );
}

#[test]
fn cache_keys_on_carried_attributes() {
    let (mesh, _) = fan();
    let mut cache = TrimeshCache::new();
    let plain = ExtractParams::default();
    let carried = params(vec![ExtractAttribute::new(attr::CORNER_UV1, [0.0, 0.0])]);
    let _ = mesh.to_trimesh_cached(&plain, &mut cache);
    let (tri, stats) = mesh.to_trimesh_cached(&carried, &mut cache);
    assert_eq!(stats.full_reuses, 0);
    assert_eq!(stats.incremental_fallbacks, 1);
    assert_eq!(tri.attributes.len(), 1);
    let (_, stats) = mesh.to_trimesh_cached(&carried, &mut cache);
    assert_eq!(stats.full_reuses, 1);
}

/// A quad split into two triangles whose shared edge carries a UV seam.
fn seamed_quads() -> Mesh {
    let mut builder = MeshBuilder::new();
    for p in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
        builder.push_vertex([p[0], p[1], 0.0]);
    }
    builder.add_face(&[0, 1, 2]).expect("face");
    builder.add_face(&[0, 2, 3]).expect("face");
    let built = builder.build().expect("build");
    let mut mesh = built.mesh;
    let corners: Vec<_> = built
        .face_ids
        .iter()
        .flat_map(|face| mesh.face_loop(*face).collect::<Vec<_>>())
        .collect();
    let mut edit = mesh.edit();
    for (k, corner) in corners.into_iter().enumerate() {
        // Every corner gets a distinct UV, so vertices 0 and 2 split.
        crate::op::set_corner_uv(&mut edit, corner, [k as f32, 0.0]).expect("uv");
    }
    let _: () = edit.finish();
    mesh
}

#[test]
fn uniform_carried_values_keep_seamed_buffers_identical() {
    let mut mesh = seamed_quads();
    let key = AttrKey::<u32>::new(Domain::Face, "face.tag");
    mesh.define_dense_layer(key, 5).expect("define");
    let (plain, plain_stats) = mesh.to_trimesh(&ExtractParams::default());
    let (carried, stats) = mesh.to_trimesh(&params(vec![ExtractAttribute::new(key, 0)]));
    assert!(plain_stats.uv_split_count > 0, "fixture has UV seams");
    assert_eq!(carried.indices, plain.indices);
    assert_eq!(carried.positions, plain.positions);
    assert_eq!(carried.uvs, plain.uvs);
    assert_eq!(carried.normals, plain.normals);
    assert_eq!(stats.split_count, plain_stats.split_count);
    assert_eq!(stats.uv_split_count, plain_stats.uv_split_count);
    assert_eq!(stats.attribute_split_count, 0);
    assert_eq!(
        carried.attribute(key),
        Some(&AttributeBuffer::U32(vec![5; plain.positions.len()]))
    );
}

#[test]
fn per_cause_split_counters_count_every_varying_cause() {
    // Vertex 0 corners: (u1, red), (u2, red), (u1, blue). The third corner is
    // split by its color, but UVs already vary at the vertex, so it also
    // counts as a UV split (documented on `ExtractStats`).
    let (mut mesh, faces) = fan();
    let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 0.0]];
    let colors = [
        [1.0, 0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
    ];
    let centers: Vec<_> = faces
        .iter()
        .map(|face| corner_at(&mesh, *face, 0))
        .collect();
    let mut edit = mesh.edit();
    for (k, corner) in centers.iter().enumerate() {
        crate::op::set_corner_uv(&mut edit, *corner, uvs[k]).expect("uv");
        crate::op::set_attribute(&mut edit, attr::CORNER_COLOR, *corner, colors[k]).expect("color");
    }
    let _: () = edit.finish();

    let (_, plain) = mesh.to_trimesh(&ExtractParams::default());
    let (_, carried) = mesh.to_trimesh(&params(vec![ExtractAttribute::new(
        attr::CORNER_COLOR,
        [0.0; 4],
    )]));
    // Vertex 0 gets 2 render vertices without colors, 3 with them.
    assert_eq!(plain.split_count, 1);
    assert_eq!(plain.uv_split_count, 1);
    assert_eq!(carried.split_count, 2);
    assert_eq!(carried.attribute_split_count, 1);
    assert_eq!(carried.uv_split_count, 2);
}

#[test]
fn streams_keep_their_source_domain() {
    // One name in two domains names two layers; both streams stay distinct.
    let vertex_tag = AttrKey::<u32>::new(Domain::Vertex, "tag");
    let face_tag = AttrKey::<u32>::new(Domain::Face, "tag");
    let (mut mesh, _) = fan();
    mesh.define_dense_layer(vertex_tag, 1).expect("vertex");
    mesh.define_dense_layer(face_tag, 2).expect("face");
    let (tri, stats) = mesh.to_trimesh(&params(vec![
        ExtractAttribute::new(vertex_tag, 0),
        ExtractAttribute::new(face_tag, 0),
    ]));
    assert_eq!(stats.missing_attribute_layers, 0);
    let domains: Vec<_> = tri.attributes.iter().map(|s| (s.domain, s.name)).collect();
    assert_eq!(domains, [(Domain::Vertex, "tag"), (Domain::Face, "tag")]);
    let n = tri.positions.len();
    assert_eq!(
        tri.attribute(vertex_tag),
        Some(&AttributeBuffer::U32(vec![1; n]))
    );
    assert_eq!(
        tri.attribute(face_tag),
        Some(&AttributeBuffer::U32(vec![2; n]))
    );
}
