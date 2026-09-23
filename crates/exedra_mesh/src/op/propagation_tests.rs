// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Caller-defined layer propagation through topology kernels.

use alloc::vec::Vec;

use crate::attributes::{AttrError, AttrKey, Domain, Propagation};
use crate::{
    ChangeSet, ChangeSetBuilder, DeletePolicy, FaceId, HalfEdgeId, Mesh, MeshBuilder,
    PropagatePolicy, VertexId, attr, op,
};

const HEAT: AttrKey<f32> = AttrKey::new(Domain::Vertex, "vertex.heat");
const TAG: AttrKey<u32> = AttrKey::new(Domain::Face, "face.tag");
const SHADE: AttrKey<[f32; 2]> = AttrKey::new(Domain::HalfEdge, "corner.shade");

/// A `width` x `height` grid of unit quads, vertices numbered row by row.
fn grid_mesh(width: u32, height: u32) -> Mesh {
    let mut builder = MeshBuilder::new();
    for y in 0..=height {
        for x in 0..=width {
            builder.push_vertex([x as f32, y as f32, 0.0]);
        }
    }
    let columns = width + 1;
    for y in 0..height {
        for x in 0..width {
            let a = y * columns + x;
            builder
                .add_face(&[a, a + 1, a + 1 + columns, a + columns])
                .expect("quad");
        }
    }
    builder.build().expect("build").mesh
}

/// Two triangles sharing the edge 1 -> 2 of the unit square.
fn two_triangles() -> Mesh {
    let mut builder = MeshBuilder::new();
    for p in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]] {
        builder.push_vertex([p[0], p[1], 0.0]);
    }
    builder.add_face(&[0, 1, 2]).expect("face");
    builder.add_face(&[2, 1, 3]).expect("face");
    builder.build().expect("build").mesh
}

fn vertex(mesh: &Mesh, index: u32) -> VertexId {
    mesh.vertices()
        .find(|v| v.index() == index)
        .expect("vertex")
}

fn edge(mesh: &Mesh, from: u32, to: u32) -> HalfEdgeId {
    mesh.half_edges()
        .find(|&h| {
            mesh.from_vertex(h).map(VertexId::index) == Some(from)
                && mesh.to_vertex(h).map(VertexId::index) == Some(to)
        })
        .expect("half-edge")
}

fn heat(mesh: &Mesh, v: VertexId) -> Option<f32> {
    mesh.attrs()
        .sparse(HEAT)
        .and_then(|l| l.get(v.as_id()))
        .copied()
}

fn shade(mesh: &Mesh, corner: HalfEdgeId) -> Option<[f32; 2]> {
    mesh.attrs()
        .sparse(SHADE)
        .and_then(|l| l.get(corner.as_id()))
        .copied()
}

/// Sets every vertex's heat to its index and every face corner's shade to
/// `[to_vertex, face]`, so sources are recognizable after an edit.
fn author(mesh: &mut Mesh) {
    let vertices: Vec<_> = mesh.vertices().collect();
    let corners: Vec<_> = mesh
        .faces()
        .flat_map(|f| mesh.face_loop(f).map(move |c| (c, f)))
        .collect();
    let mut edit = mesh.edit();
    for v in vertices {
        op::set_attribute(&mut edit, HEAT, v, v.index() as f32).expect("heat");
    }
    for (corner, face) in corners {
        let to = edit.mesh().to_vertex(corner).expect("to").index() as f32;
        op::set_attribute(&mut edit, SHADE, corner, [to, face.index() as f32]).expect("shade");
    }
    let _: () = edit.finish();
}

fn split(mesh: &mut Mesh, from: u32, to: u32) -> (VertexId, ChangeSet) {
    let h = edge(mesh, from, to);
    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    let v = op::split_edge(&mut edit, h, &PropagatePolicy::default()).expect("split");
    (v, edit.finish())
}

#[test]
fn rules_are_declared_per_layer_and_validated() {
    let mut mesh = two_triangles();
    assert_eq!(
        mesh.set_layer_propagation(HEAT, Propagation::Copy),
        Err(AttrError::NotDefined)
    );
    assert_eq!(
        mesh.set_layer_propagation(attr::CORNER_UV, Propagation::Copy),
        Err(AttrError::Reserved)
    );
    mesh.define_sparse_layer(TAG).expect("tag");
    assert_eq!(
        mesh.set_layer_propagation(TAG, Propagation::Interpolate),
        Err(AttrError::NotInterpolable)
    );
    let mistyped = AttrKey::<f32>::new(Domain::Face, "face.tag");
    assert_eq!(
        mesh.set_layer_propagation(mistyped, Propagation::Copy),
        Err(AttrError::TypeMismatch)
    );
    let before = mesh.revision();
    mesh.set_layer_propagation(TAG, Propagation::Copy)
        .expect("copy");
    assert_eq!(mesh.revision(), before, "declaring a rule writes no value");
    assert_eq!(
        mesh.attrs().propagation(Domain::Face, "face.tag"),
        Some(Propagation::Copy)
    );
}

#[test]
fn split_edge_interpolates_vertices_and_corners() {
    let mut mesh = two_triangles();
    author(&mut mesh);
    mesh.set_layer_propagation(HEAT, Propagation::Interpolate)
        .expect("heat");
    mesh.set_layer_propagation(SHADE, Propagation::Interpolate)
        .expect("shade");
    let (m, changes) = split(&mut mesh, 1, 2);
    assert_eq!(heat(&mesh, m), Some(1.5));
    assert_eq!(changes.unpropagated_attribute_values, 0);
    // Every corner at the inserted vertex blends the corners at the two
    // endpoints of its face's side of the edge; the far corners keep theirs.
    for face in mesh.faces().collect::<Vec<_>>() {
        for corner in mesh.face_loop(face).collect::<Vec<_>>() {
            let to = mesh.to_vertex(corner).expect("to");
            let value = shade(&mesh, corner).expect("every corner carries a value");
            if to == m {
                assert_eq!(value[0], 1.5, "corner at the inserted vertex");
            } else {
                assert_eq!(value[0], to.index() as f32);
            }
        }
    }
    assert!(mesh.validate_deep().is_empty());
}

#[test]
fn split_edge_copy_and_clear_rules() {
    let mut mesh = two_triangles();
    author(&mut mesh);
    mesh.set_layer_propagation(HEAT, Propagation::Copy)
        .expect("heat");
    mesh.set_layer_propagation(SHADE, Propagation::Clear)
        .expect("shade");
    let (m, changes) = split(&mut mesh, 1, 2);
    assert_eq!(heat(&mesh, m), Some(1.0), "copies the split's from vertex");
    let at_m: Vec<_> = mesh
        .half_edges()
        .filter(|&h| mesh.to_vertex(h) == Some(m) && mesh.face(h) != Some(FaceId::OUTSIDE))
        .collect();
    assert_eq!(at_m.len(), 2);
    assert!(at_m.iter().all(|&c| shade(&mesh, c).is_none()));
    assert_eq!(
        changes.unpropagated_attribute_values, 0,
        "clear is declared"
    );
}

#[test]
fn unspecified_rules_clear_and_count() {
    let mut mesh = two_triangles();
    author(&mut mesh);
    let (m, changes) = split(&mut mesh, 1, 2);
    assert_eq!(heat(&mesh, m), None);
    // The new vertex, two child corners and two parent corners.
    assert_eq!(changes.unpropagated_attribute_values, 5);
}

#[test]
fn split_face_continues_the_face_and_corners() {
    let mut mesh = grid_mesh(1, 1);
    author(&mut mesh);
    mesh.define_dense_layer(TAG, 0).expect("tag");
    let face = mesh.faces().next().expect("face");
    let mut edit = mesh.edit();
    op::set_attribute(&mut edit, TAG, face, 7).expect("tag");
    op::set_face_region(&mut edit, face, 5).expect("region");
    let _: () = edit.finish();
    mesh.set_layer_propagation(TAG, Propagation::Copy)
        .expect("tag");
    mesh.set_layer_propagation(SHADE, Propagation::Copy)
        .expect("shade");

    let corners: Vec<_> = mesh.face_loop(face).collect();
    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    let new_face = op::split_face(
        &mut edit,
        corners[0],
        corners[2],
        &PropagatePolicy::default(),
    )
    .expect("split");
    let changes = edit.finish();
    assert_eq!(changes.unpropagated_attribute_values, 0);
    let tag = mesh.attrs().dense(TAG).expect("tag");
    assert_eq!(tag.get(new_face.as_id()), Some(&7));
    assert_eq!(tag.get(face.as_id()), Some(&7));
    // The new face extends the face arena; its dense layers (the built-in
    // region included) must already cover it when written.
    let region = mesh.attrs().dense(attr::FACE_REGION).expect("region");
    assert_eq!(region.get(new_face.as_id()), Some(&5));
    for face in [face, new_face] {
        for corner in mesh.face_loop(face).collect::<Vec<_>>() {
            let to = mesh.to_vertex(corner).expect("to").index() as f32;
            assert_eq!(shade(&mesh, corner).map(|v| v[0]), Some(to));
        }
    }
}

#[test]
fn flip_edge_reaims_diagonal_corners() {
    let mut mesh = two_triangles();
    author(&mut mesh);
    mesh.set_layer_propagation(SHADE, Propagation::Copy)
        .expect("shade");
    let h = edge(&mesh, 1, 2);
    let mut edit = mesh.edit();
    let _ = op::flip_edge(&mut edit, h).expect("flip");
    let _: () = edit.finish();
    for face in mesh.faces().collect::<Vec<_>>() {
        for corner in mesh.face_loop(face).collect::<Vec<_>>() {
            let to = mesh.to_vertex(corner).expect("to").index() as f32;
            assert_eq!(shade(&mesh, corner).map(|v| v[0]), Some(to));
        }
    }
}

#[test]
fn collapse_edge_transfers_dying_corners() {
    let mut mesh = grid_mesh(3, 3);
    author(&mut mesh);
    mesh.set_layer_propagation(SHADE, Propagation::Copy)
        .expect("shade");
    // An edge between the interior vertices 5 and 6; its quads shrink.
    let h = edge(&mesh, 5, 6);
    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    op::collapse_edge(&mut edit, h).expect("collapse");
    let changes = edit.finish();
    assert_eq!(changes.unpropagated_attribute_values, 0);
    for face in mesh.faces().collect::<Vec<_>>() {
        for corner in mesh.face_loop(face).collect::<Vec<_>>() {
            assert!(
                shade(&mesh, corner).is_some(),
                "every surviving corner keeps or inherits a value"
            );
        }
    }
    assert!(changes.cleared_attribute_values > 0, "dying corners count");
}

#[test]
fn dissolve_edges_merges_agreeing_faces_and_counts_conflicts() {
    for (tags, carried) in [([3, 3], Some(3)), ([3, 4], None)] {
        let mut mesh = two_triangles();
        author(&mut mesh);
        mesh.define_sparse_layer(TAG).expect("tag");
        mesh.set_layer_propagation(TAG, Propagation::Copy)
            .expect("tag");
        mesh.set_layer_propagation(SHADE, Propagation::Copy)
            .expect("shade");
        let faces: Vec<_> = mesh.faces().collect();
        let mut edit = mesh.edit();
        for (face, tag) in faces.into_iter().zip(tags) {
            op::set_attribute(&mut edit, TAG, face, tag).expect("tag");
        }
        let _: () = edit.finish();

        let canonical = mesh.canonical_edge(edge(&mesh, 1, 2)).expect("edge");
        let mut edit = mesh.edit_with(ChangeSetBuilder::new());
        let merged = op::dissolve_edges(&mut edit, &[canonical]).expect("dissolve");
        let changes = edit.finish();
        let tag = mesh
            .attrs()
            .sparse(TAG)
            .and_then(|l| l.get(merged[0].as_id()))
            .copied();
        assert_eq!(tag, carried);
        assert_eq!(
            changes.unpropagated_attribute_values,
            u64::from(carried.is_none())
        );
        for corner in mesh.face_loop(merged[0]).collect::<Vec<_>>() {
            let to = mesh.to_vertex(corner).expect("to").index() as f32;
            assert_eq!(shade(&mesh, corner).map(|v| v[0]), Some(to));
        }
    }
}

#[test]
fn dissolve_vertices_restores_corners_by_vertex() {
    // Two quads sharing the path 0 -> 4 -> 2 through the valence-2 vertex 4.
    let mut builder = MeshBuilder::new();
    for p in [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [1.0, 1.0]] {
        builder.push_vertex([p[0], p[1], 0.0]);
    }
    builder.add_face(&[0, 1, 2, 4]).expect("face");
    builder.add_face(&[0, 4, 2, 3]).expect("face");
    let mut mesh = builder.build().expect("build").mesh;
    author(&mut mesh);
    mesh.set_layer_propagation(SHADE, Propagation::Copy)
        .expect("shade");
    let middle = vertex(&mesh, 4);
    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    let rebuilt = op::dissolve_vertices(&mut edit, &[middle]).expect("dissolve");
    let changes = edit.finish();
    assert_eq!(rebuilt.len(), 2);
    assert_eq!(changes.unpropagated_attribute_values, 0);
    for face in rebuilt {
        for corner in mesh.face_loop(face).collect::<Vec<_>>() {
            let to = mesh.to_vertex(corner).expect("to").index() as f32;
            assert_eq!(shade(&mesh, corner).map(|v| v[0]), Some(to));
        }
    }
}

#[test]
fn deleted_slots_never_leak_values_to_recycled_elements() {
    let mut mesh = two_triangles();
    mesh.define_sparse_layer(TAG).expect("tag");
    let doomed = mesh.faces().last().expect("face");
    let mut edit = mesh.edit();
    op::set_attribute(&mut edit, TAG, doomed, 9).expect("tag");
    let _: () = edit.finish();

    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    op::delete_faces(&mut edit, &[doomed], DeletePolicy::KeepIsolated).expect("delete");
    let deleted = edit.finish();
    assert_eq!(deleted.cleared_attribute_values, 1);

    let loop_vertices = [vertex(&mesh, 2), vertex(&mesh, 1), vertex(&mesh, 3)];
    let mut edit = mesh.edit();
    let recycled = op::add_face(&mut edit, &loop_vertices).expect("add");
    let _: () = edit.finish();
    assert_eq!(recycled.index(), doomed.index(), "the slot is recycled");
    assert_eq!(
        mesh.attrs()
            .sparse(TAG)
            .and_then(|l| l.get(recycled.as_id())),
        None
    );
}

#[test]
fn one_sided_blends_do_not_depend_on_edge_direction() {
    // Only vertex 2 carries heat. Splitting 1 -> 2 or 2 -> 1 must treat the
    // one-sided blend alike: `Interpolate` carries vertex 2's value, and
    // `Unspecified` counts it as lost, in both directions.
    for (from, to) in [(1, 2), (2, 1)] {
        let mut mesh = two_triangles();
        let hot = vertex(&mesh, 2);
        let mut edit = mesh.edit();
        op::set_attribute(&mut edit, HEAT, hot, 7.0).expect("heat");
        let _: () = edit.finish();

        let mut interpolated = mesh.clone();
        interpolated
            .set_layer_propagation(HEAT, Propagation::Interpolate)
            .expect("heat");
        let (m, changes) = split(&mut interpolated, from, to);
        assert_eq!(heat(&interpolated, m), Some(7.0), "split {from} -> {to}");
        assert_eq!(changes.unpropagated_attribute_values, 0);

        let (m, changes) = split(&mut mesh, from, to);
        assert_eq!(heat(&mesh, m), None);
        assert_eq!(
            changes.unpropagated_attribute_values, 1,
            "split {from} -> {to} loses the one-sided vertex value"
        );
    }
}

#[test]
fn reserved_keys_report_no_caller_rule() {
    let mesh = two_triangles();
    assert_eq!(mesh.attrs().propagation(Domain::Face, "face.region"), None);
    assert_eq!(
        mesh.attrs().propagation(Domain::HalfEdge, "corner.uv"),
        None
    );
}
