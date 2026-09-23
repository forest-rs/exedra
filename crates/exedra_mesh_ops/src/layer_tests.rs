// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Caller-defined layers and corner UVs through mesh operations.

use alloc::vec;
use alloc::vec::Vec;

use exedra_math::{Placement3, Plane3};
use exedra_mesh::attributes::{AttrKey, Domain, Propagation};
use exedra_mesh::{
    ChangeSetBuilder, FaceId, FaceTriangulation, HalfEdgeId, Mesh, MeshBuilder, PropagatePolicy,
    VertexId, attr, op,
};

const TAG: AttrKey<u32> = AttrKey::new(Domain::Face, "face.tag");
const HEAT: AttrKey<f32> = AttrKey::new(Domain::Vertex, "vertex.heat");
const SHADE: AttrKey<[f32; 2]> = AttrKey::new(Domain::HalfEdge, "corner.shade");

/// An axis-aligned box from `min` with edge lengths `size`, outward winding.
fn cuboid(min: [f32; 3], size: [f32; 3]) -> Mesh {
    let mut builder = MeshBuilder::new();
    for z in [0.0, 1.0] {
        for y in [0.0, 1.0] {
            for x in [0.0, 1.0] {
                builder.push_vertex([
                    min[0] + x * size[0],
                    min[1] + y * size[1],
                    min[2] + z * size[2],
                ]);
            }
        }
    }
    for face in [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ] {
        builder.add_face(&face).expect("box face");
    }
    builder.build().expect("box").mesh
}

/// Tags faces `first..`, sets `HEAT` to each vertex's x and `SHADE` to each
/// corner's (x, y). Heat and shade interpolate; tags copy.
fn annotate(mesh: &mut Mesh, first: u32) {
    mesh.define_sparse_layer(TAG).expect("tag");
    mesh.define_sparse_layer(HEAT).expect("heat");
    mesh.define_sparse_layer(SHADE).expect("shade");
    mesh.set_layer_propagation(TAG, Propagation::Copy)
        .expect("tag rule");
    mesh.set_layer_propagation(HEAT, Propagation::Interpolate)
        .expect("heat rule");
    mesh.set_layer_propagation(SHADE, Propagation::Interpolate)
        .expect("shade rule");
    let faces: Vec<FaceId> = mesh.faces().collect();
    let vertices: Vec<VertexId> = mesh.vertices().collect();
    let corners: Vec<HalfEdgeId> = faces.iter().flat_map(|&f| mesh.face_loop(f)).collect();
    let mut edit = mesh.edit();
    for (index, &face) in faces.iter().enumerate() {
        let tag = first + u32::try_from(index).expect("small");
        op::set_attribute(&mut edit, TAG, face, tag).expect("tag");
    }
    for vertex in vertices {
        let x = edit.mesh().vertex_position(vertex).expect("live")[0];
        op::set_attribute(&mut edit, HEAT, vertex, x).expect("heat");
    }
    for corner in corners {
        let vertex = edit.mesh().to_vertex(corner).expect("corner");
        let p = *edit.mesh().vertex_position(vertex).expect("live");
        op::set_attribute(&mut edit, SHADE, corner, [p[0], p[1]]).expect("shade");
    }
    let _: () = edit.finish();
}

fn tag(mesh: &Mesh, face: FaceId) -> Option<u32> {
    mesh.attrs().sparse(TAG)?.get(face.as_id()).copied()
}

fn heat(mesh: &Mesh, vertex: VertexId) -> Option<f32> {
    mesh.attrs().sparse(HEAT)?.get(vertex.as_id()).copied()
}

fn shade(mesh: &Mesh, corner: HalfEdgeId) -> Option<[f32; 2]> {
    mesh.attrs().sparse(SHADE)?.get(corner.as_id()).copied()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-5
}

#[test]
fn extrusion_copies_source_faces_corners_and_vertices() {
    let mut mesh = cuboid([0.0; 3], [1.0; 3]);
    annotate(&mut mesh, 1);
    let top = mesh
        .faces()
        .find(|&face| {
            mesh.face_loop(face).all(|corner| {
                let vertex = mesh.to_vertex(corner).expect("corner");
                mesh.vertex_position(vertex).expect("live")[2] == 1.0
            })
        })
        .expect("top face");
    let top_tag = tag(&mesh, top);
    let mut edit = mesh.edit_with(ChangeSetBuilder::new());
    let (_, output) = crate::face_edit::extrude_faces(
        &mut edit,
        &crate::face_edit::ExtrudeFacesParams {
            faces: vec![top],
            mode: crate::face_edit::ExtrudeMode::ShellOpen,
            distance: 0.5,
        },
        &PropagatePolicy::default(),
    )
    .expect("extrude");
    let changes = edit.finish();
    assert_eq!(changes.unpropagated_attribute_values, 0);
    for &face in output.cap_faces.iter().chain(&output.wall_faces) {
        assert_eq!(tag(&mesh, face), top_tag);
        for corner in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(corner).expect("corner");
            let p = *mesh.vertex_position(vertex).expect("live");
            // Copied cap vertices keep their source vertex's x; the extrusion
            // moves them along z only.
            assert_eq!(heat(&mesh, vertex), Some(p[0]));
            assert_eq!(shade(&mesh, corner), Some([p[0], p[1]]));
        }
    }
}

#[test]
fn poke_averages_the_center_and_copies_the_rim() {
    let mut mesh = cuboid([0.0; 3], [2.0; 3]);
    annotate(&mut mesh, 1);
    let face = mesh.faces().next().expect("face");
    let face_tag = tag(&mesh, face);
    let mut edit = mesh.edit();
    let output = crate::poke::poke_faces(
        &mut edit,
        &crate::poke::PokeFacesParams { faces: vec![face] },
        &PropagatePolicy::default(),
    )
    .expect("poke");
    let _: () = edit.finish();
    let center = output.center_vertices[0];
    let p = *mesh.vertex_position(center).expect("center");
    assert_eq!(heat(&mesh, center), Some(p[0]));
    for fan in output.fan_faces {
        assert_eq!(tag(&mesh, fan), face_tag);
        for corner in mesh.face_loop(fan) {
            let vertex = mesh.to_vertex(corner).expect("corner");
            let q = *mesh.vertex_position(vertex).expect("live");
            let value = shade(&mesh, corner).expect("shade");
            assert!(
                close(value[0], q[0]) && close(value[1], q[1]),
                "{value:?} at {q:?}"
            );
        }
    }
}

#[test]
fn rounding_carries_owner_values_and_interpolates_new_points() {
    let mut mesh = cuboid([0.0; 3], [2.0; 3]);
    annotate(&mut mesh, 1);
    let edges: Vec<HalfEdgeId> = mesh.half_edges().collect();
    let mut edit = mesh.edit();
    for edge in edges {
        let _ = op::set_edge_sharpness(&mut edit, edge, 1.0);
    }
    let _: () = edit.finish();
    crate::round::round_sharp_edges(&mut mesh, &crate::round::RoundPolicy::chamfer(0.25))
        .expect("chamfer");
    for face in mesh.faces() {
        assert!(tag(&mesh, face).is_some(), "every face has an owner");
        for corner in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(corner).expect("corner");
            let p = *mesh.vertex_position(vertex).expect("live");
            let value = shade(&mesh, corner).expect("shade");
            // Every corner of a planar owner interpolates its linear chart.
            assert!((0.0..=2.0).contains(&value[0]), "{value:?}");
            let h = heat(&mesh, vertex).expect("heat");
            assert!((0.0..=2.0).contains(&h), "{h}");
            if face_is_axis_aligned(&mesh, face) {
                assert!(
                    close(value[0], p[0]) && close(value[1], p[1]),
                    "{value:?} at {p:?}"
                );
            }
        }
    }
}

/// Whether every vertex of `face` shares one coordinate (an original box face).
fn face_is_axis_aligned(mesh: &Mesh, face: FaceId) -> bool {
    let points: Vec<[f32; 3]> = mesh
        .face_loop(face)
        .map(|corner| {
            *mesh
                .vertex_position(mesh.to_vertex(corner).expect("c"))
                .expect("p")
        })
        .collect();
    (0..3).any(|axis| points.iter().all(|p| p[axis] == points[0][axis]))
}

#[test]
fn stretch_samples_unmoved_source_positions() {
    let mut mesh = cuboid([0.0; 3], [2.0; 3]);
    annotate(&mut mesh, 1);
    let result = crate::stretch::stretch_mesh(
        &mesh,
        &Plane3 {
            normal: [1.0, 0.0, 0.0],
            distance: 1.0,
        },
        2.0,
        &Placement3::IDENTITY,
        &crate::stretch::StretchPolicy::default(),
    )
    .expect("stretch");
    assert!(result.topology_rebuilt);
    let out = &result.mesh;
    assert_eq!(
        out.attrs().propagation(Domain::Vertex, HEAT.name()),
        Some(Propagation::Interpolate),
        "rules survive the rebuild"
    );
    assert_eq!(result.stats.unpropagated_attribute_values, 0);
    for vertex in out.vertices() {
        let x = out.vertex_position(vertex).expect("live")[0];
        // Everything beyond the plane moved by the stretch length.
        let source_x = if x > 1.0 { x - 2.0 } else { x };
        let h = heat(out, vertex).expect("heat");
        assert!(close(h, source_x), "heat {h} at x {x}");
    }
    for face in out.faces() {
        let source = result.face_sources[&face].face();
        assert_eq!(tag(out, face), tag(&mesh, source));
    }
}

#[test]
fn split_interpolates_intersections_and_leaves_caps_empty() {
    let mut mesh = cuboid([0.0; 3], [2.0; 3]);
    annotate(&mut mesh, 1);
    let split = crate::section::split_mesh(
        &mesh,
        Plane3 {
            normal: [1.0, 0.0, 0.0],
            distance: 0.5,
        },
        &crate::section::SectionPolicy::default(),
        99,
    )
    .expect("split");
    let half = split.negative.expect("negative half");
    let out = &half.mesh;
    for (&face, source) in &half.face_sources {
        match source {
            crate::section::CutFaceSource::Original(original) => {
                assert_eq!(tag(out, face), tag(&mesh, *original));
                for corner in out.face_loop(face) {
                    let p = *out
                        .vertex_position(out.to_vertex(corner).expect("c"))
                        .expect("p");
                    let value = shade(out, corner).expect("shade");
                    assert!(
                        close(value[0], p[0]) && close(value[1], p[1]),
                        "{value:?} {p:?}"
                    );
                }
            }
            crate::section::CutFaceSource::Cap => assert_eq!(tag(out, face), None),
        }
    }
    for vertex in out.vertices() {
        let x = out.vertex_position(vertex).expect("live")[0];
        assert!(close(heat(out, vertex).expect("heat"), x));
    }
}

#[test]
fn reflection_carries_values_verbatim_on_the_same_corners() {
    const FROZEN: AttrKey<u32> = AttrKey::new(Domain::Vertex, "vertex.frozen");
    let mut mesh = cuboid([0.0; 3], [1.0; 3]);
    annotate(&mut mesh, 1);
    mesh.define_sparse_layer(FROZEN).expect("frozen");
    mesh.set_layer_propagation(FROZEN, Propagation::Clear)
        .expect("clear rule");
    let vertices: Vec<VertexId> = mesh.vertices().collect();
    let mut edit = mesh.edit();
    for vertex in &vertices {
        op::set_attribute(&mut edit, FROZEN, *vertex, vertex.index() + 7).expect("frozen");
    }
    let _: () = edit.finish();
    let mirror = Placement3 {
        rows: [
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    };
    let out = crate::transform::transform(&mesh, &mirror).expect("reflect");
    for (source, output) in mesh.vertices().zip(out.vertices()) {
        assert_eq!(
            out.attrs()
                .sparse(FROZEN)
                .and_then(|l| l.get(output.as_id())),
            Some(&(source.index() + 7)),
            "a Clear rule does not apply to a relabeling rebuild"
        );
    }
    for (source, output) in mesh.faces().zip(out.faces()) {
        assert_eq!(tag(&out, output), tag(&mesh, source));
        for corner in out.face_loop(output) {
            let p = *out
                .vertex_position(out.to_vertex(corner).expect("c"))
                .expect("p");
            // Mirrored x: the corner kept the value authored at (-x, y).
            assert_eq!(shade(&out, corner), Some([-p[0], p[1]]));
        }
    }
}

fn boolean(
    a: &Mesh,
    b: &Mesh,
) -> Result<crate::boolean::BooleanOutput, crate::boolean::BooleanError> {
    crate::boolean::boolean_mesh(
        a,
        b,
        crate::boolean::BooleanOp::Difference,
        FaceTriangulation::Fan,
        &mut crate::boolean::BooleanScratch::new(),
        &mut crate::boolean::BooleanDiagnostics::default(),
    )
}

#[test]
fn booleans_keep_uvs_and_each_operands_values() {
    let mut a = cuboid([0.0; 3], [1.0; 3]);
    annotate(&mut a, 1);
    let corners: Vec<HalfEdgeId> = a.faces().flat_map(|f| a.face_loop(f)).collect();
    let mut edit = a.edit();
    for corner in corners {
        let p = *edit
            .mesh()
            .vertex_position(edit.mesh().to_vertex(corner).expect("c"))
            .expect("p");
        op::set_corner_uv(&mut edit, corner, [p[0] + p[2], p[1]]).expect("uv");
    }
    let _: () = edit.finish();
    let mut b = cuboid([0.5; 3], [1.0; 3]);
    annotate(&mut b, 100);

    let output = boolean(&a, &b).expect("difference");
    let mesh = &output.mesh;
    assert_eq!(output.stats.uv_unmapped_faces, 0);
    assert_eq!(output.stats.unpropagated_attribute_values, 0);
    let uvs = mesh.attrs().sparse(attr::CORNER_UV).expect("uv layer");
    let mut cut_faces = 0;
    for &(face, side, source) in &output.face_provenance {
        let operand = match side {
            crate::boolean::MeshSide::A => &a,
            crate::boolean::MeshSide::B => {
                cut_faces += 1;
                &b
            }
        };
        assert_eq!(tag(mesh, face), tag(operand, source));
        for corner in mesh.face_loop(face) {
            let p = *mesh
                .vertex_position(mesh.to_vertex(corner).expect("c"))
                .expect("p");
            let value = shade(mesh, corner).expect("shade");
            assert!(
                close(value[0], p[0]) && close(value[1], p[1]),
                "{value:?} {p:?}"
            );
            if side == crate::boolean::MeshSide::A {
                let uv = uvs.get(corner.as_id()).expect("A corners keep UVs");
                assert!(
                    close(uv[0], p[0] + p[2]) && close(uv[1], p[1]),
                    "{uv:?} {p:?}"
                );
            } else {
                assert!(uvs.get(corner.as_id()).is_none(), "B has no chart");
            }
        }
    }
    assert!(cut_faces > 0, "the cut surface comes from B");

    let mut conflicting = cuboid([0.5; 3], [1.0; 3]);
    conflicting
        .define_sparse_layer(AttrKey::<f32>::new(Domain::Face, TAG.name()))
        .expect("f32 tag");
    assert_eq!(
        boolean(&a, &conflicting).map(|_| ()),
        Err(crate::boolean::BooleanError::AttributeLayerConflict)
    );
}

#[test]
fn component_cleanup_counts_cleared_values() {
    let mut mesh = cuboid([0.0; 3], [1.0; 3]);
    let mut builder_mesh = cuboid([5.0; 3], [1.0; 3]);
    annotate(&mut builder_mesh, 1);
    // One annotated box removed as a small component.
    let removed = builder_mesh.faces().count();
    let cleanup = crate::components::remove_small_components(
        &mut builder_mesh,
        u32::try_from(removed + 1).expect("small"),
    )
    .expect("cleanup");
    assert_eq!(cleanup.removed_faces.len(), removed);
    // 6 tags, 8 heats, 24 shades.
    assert_eq!(cleanup.cleared_attribute_values, 6 + 8 + 24);
    let untouched = crate::components::remove_small_components(&mut mesh, 1).expect("noop");
    assert_eq!(untouched.cleared_attribute_values, 0);
}

const CORNER_CODE: AttrKey<u32> = AttrKey::new(Domain::HalfEdge, "corner.code");
const VERTEX_CODE: AttrKey<u32> = AttrKey::new(Domain::Vertex, "vertex.code");

/// A position's code: twice each coordinate as a decimal digit, so half-unit
/// grid points get distinct integers.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test positions are small non-negative half-units"
)]
fn code(p: [f32; 3]) -> u32 {
    let digit = |c: f32| (c * 2.0).round() as u32;
    digit(p[0]) * 100 + digit(p[1]) * 10 + digit(p[2])
}

/// Registers `Copy` u32 corner and vertex layers holding each position's
/// code, the integer data a `Copy` rule must never blend or misplace.
fn encode_positions(mesh: &mut Mesh) {
    for key in [CORNER_CODE, VERTEX_CODE] {
        mesh.define_sparse_layer(key).expect("code layer");
        mesh.set_layer_propagation(key, Propagation::Copy)
            .expect("copy rule");
    }
    let vertices: Vec<VertexId> = mesh.vertices().collect();
    let corners: Vec<HalfEdgeId> = mesh
        .faces()
        .flat_map(|f| mesh.face_loop(f).collect::<Vec<_>>())
        .collect();
    let mut edit = mesh.edit();
    for vertex in vertices {
        let p = *edit.mesh().vertex_position(vertex).expect("live");
        op::set_attribute(&mut edit, VERTEX_CODE, vertex, code(p)).expect("vertex code");
    }
    for corner in corners {
        let vertex = edit.mesh().to_vertex(corner).expect("corner");
        let p = *edit.mesh().vertex_position(vertex).expect("live");
        op::set_attribute(&mut edit, CORNER_CODE, corner, code(p)).expect("corner code");
    }
    let _: () = edit.finish();
}

/// Asserts that every output corner and vertex whose source position
/// `source_of` identifies as an original vertex carries that vertex's code.
/// Returns how many corners and vertices were checked.
fn assert_codes_at_originals(
    mesh: &Mesh,
    source_of: impl Fn([f32; 3]) -> Option<[f32; 3]>,
) -> usize {
    let mut checked = 0;
    for vertex in mesh.vertices() {
        let p = *mesh.vertex_position(vertex).expect("live");
        if let Some(source) = source_of(p) {
            let value = mesh
                .attrs()
                .sparse(VERTEX_CODE)
                .and_then(|l| l.get(vertex.as_id()).copied());
            assert_eq!(value, Some(code(source)), "vertex at {p:?}");
            checked += 1;
        }
    }
    for face in mesh.faces() {
        for corner in mesh.face_loop(face) {
            let p = *mesh
                .vertex_position(mesh.to_vertex(corner).expect("corner"))
                .expect("live");
            if let Some(source) = source_of(p) {
                let value = mesh
                    .attrs()
                    .sparse(CORNER_CODE)
                    .and_then(|l| l.get(corner.as_id()).copied());
                assert_eq!(value, Some(code(source)), "corner at {p:?}");
                checked += 1;
            }
        }
    }
    checked
}

/// Whether every coordinate lies in `values`.
fn on_grid(p: [f32; 3], values: &[f32]) -> bool {
    p.iter().all(|c| values.contains(c))
}

#[test]
fn copy_layers_keep_exact_values_at_original_vertices_through_booleans() {
    for op_kind in [
        crate::boolean::BooleanOp::Difference,
        crate::boolean::BooleanOp::Union,
    ] {
        let mut a = cuboid([0.0; 3], [1.0; 3]);
        let mut b = cuboid([0.5; 3], [1.0; 3]);
        encode_positions(&mut a);
        encode_positions(&mut b);
        let output = crate::boolean::boolean_mesh(
            &a,
            &b,
            op_kind,
            FaceTriangulation::Fan,
            &mut crate::boolean::BooleanScratch::new(),
            &mut crate::boolean::BooleanDiagnostics::default(),
        )
        .expect("boolean");
        assert_eq!(output.stats.unpropagated_attribute_values, 0);
        // A's vertices sit on {0, 1}, B's on {0.5, 1.5}; seam vertices mix.
        let checked = assert_codes_at_originals(&output.mesh, |p| {
            (on_grid(p, &[0.0, 1.0]) || on_grid(p, &[0.5, 1.5])).then_some(p)
        });
        assert!(checked > 20, "{op_kind:?} checked {checked}");
    }
}

#[test]
fn a_touching_vertex_keeps_its_own_operand_values() {
    // B stands on A's top face: B's bottom vertices lie inside A's top face.
    let mut a = cuboid([0.0; 3], [2.0; 3]);
    let mut b = cuboid([0.5, 0.5, 2.0], [1.0; 3]);
    encode_positions(&mut a);
    encode_positions(&mut b);
    let output = crate::boolean::boolean_mesh(
        &a,
        &b,
        crate::boolean::BooleanOp::Union,
        FaceTriangulation::Fan,
        &mut crate::boolean::BooleanScratch::new(),
        &mut crate::boolean::BooleanDiagnostics::default(),
    )
    .expect("union");
    let mesh = &output.mesh;
    let mut touching = 0;
    for vertex in mesh.vertices() {
        let p = *mesh.vertex_position(vertex).expect("live");
        if on_grid([p[0], p[1], 0.5], &[0.5, 1.5]) && p[2] == 2.0 {
            let value = mesh
                .attrs()
                .sparse(VERTEX_CODE)
                .and_then(|l| l.get(vertex.as_id()).copied());
            assert_eq!(value, Some(code(p)), "B vertex at {p:?}");
            touching += 1;
        }
    }
    assert_eq!(touching, 4, "B's four bottom vertices are in the result");
}

#[test]
fn copy_layers_keep_exact_values_through_stretch_and_split() {
    let mut mesh = cuboid([0.0; 3], [2.0; 3]);
    encode_positions(&mut mesh);
    let result = crate::stretch::stretch_mesh(
        &mesh,
        &Plane3 {
            normal: [1.0, 0.0, 0.0],
            distance: 1.0,
        },
        2.0,
        &Placement3::IDENTITY,
        &crate::stretch::StretchPolicy::default(),
    )
    .expect("stretch");
    assert_eq!(result.stats.unpropagated_attribute_values, 0);
    // Vertices beyond the plane moved by 2; cut vertices at x = 1 or 3 blend
    // two ends and are skipped.
    let checked = assert_codes_at_originals(&result.mesh, |p| match p[0] {
        0.0 => Some(p),
        4.0 => Some([2.0, p[1], p[2]]),
        _ => None,
    });
    assert!(checked > 0, "stretch checked {checked}");

    let split = crate::section::split_mesh(
        &mesh,
        Plane3 {
            normal: [1.0, 0.0, 0.0],
            distance: 0.5,
        },
        &crate::section::SectionPolicy::default(),
        99,
    )
    .expect("split");
    let half = split.negative.expect("negative half");
    let checked = assert_codes_at_originals(&half.mesh, |p| (p[0] == 0.0).then_some(p));
    assert!(checked > 0, "split checked {checked}");
}

#[test]
fn operands_with_different_rules_conflict() {
    let mut a = cuboid([0.0; 3], [1.0; 3]);
    let mut b = cuboid([0.5; 3], [1.0; 3]);
    encode_positions(&mut a);
    encode_positions(&mut b);
    b.set_layer_propagation(VERTEX_CODE, Propagation::Clear)
        .expect("clear rule");
    assert_eq!(
        boolean(&a, &b).map(|_| ()),
        Err(crate::boolean::BooleanError::AttributeLayerConflict)
    );
}

/// A size-2 box whose top/front edge carries an extra collinear vertex at
/// (1, 0, 2), code 204. The robust triangulation drops that T-vertex as a
/// degenerate ear, so only the face loop knows it.
fn box_with_collinear_vertex() -> Mesh {
    let mut builder = MeshBuilder::new();
    for p in [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [0.0, 2.0, 0.0],
        [2.0, 2.0, 0.0],
        [0.0, 0.0, 2.0],
        [2.0, 0.0, 2.0],
        [0.0, 2.0, 2.0],
        [2.0, 2.0, 2.0],
        [1.0, 0.0, 2.0],
    ] {
        builder.push_vertex(p);
    }
    for face in [
        &[0, 2, 3, 1][..],
        &[4, 8, 5, 7, 6],
        &[0, 1, 5, 8, 4],
        &[2, 6, 7, 3],
        &[0, 4, 6, 2],
        &[1, 3, 7, 5],
    ] {
        builder.add_face(face).expect("box face");
    }
    let mut mesh = builder.build().expect("box").mesh;
    encode_positions(&mut mesh);
    mesh
}

#[test]
fn collinear_source_vertices_keep_exact_values() {
    let mesh = box_with_collinear_vertex();
    let at_t_vertex =
        |target: [f32; 3]| move |p: [f32; 3]| (p == target).then_some([1.0, 0.0, 2.0]);
    let up = Plane3 {
        normal: [0.0, 0.0, 1.0],
        distance: 1.0,
    };

    let split =
        crate::section::split_mesh(&mesh, up, &crate::section::SectionPolicy::default(), 99)
            .expect("split");
    let top = split.positive.expect("positive half");
    let checked = assert_codes_at_originals(&top.mesh, at_t_vertex([1.0, 0.0, 2.0]));
    assert!(checked >= 3, "split checked {checked}");

    let stretched = crate::stretch::stretch_mesh(
        &mesh,
        &up,
        2.0,
        &Placement3::IDENTITY,
        &crate::stretch::StretchPolicy::default(),
    )
    .expect("stretch");
    let checked = assert_codes_at_originals(&stretched.mesh, at_t_vertex([1.0, 0.0, 4.0]));
    assert!(checked >= 3, "stretch checked {checked}");

    let mut cutter = cuboid([0.5, 0.5, -1.0], [1.0, 1.0, 2.0]);
    encode_positions(&mut cutter);
    let output = boolean(&mesh, &cutter).expect("difference");
    let checked = assert_codes_at_originals(&output.mesh, at_t_vertex([1.0, 0.0, 2.0]));
    assert!(checked >= 3, "boolean checked {checked}");
}

/// Adds `offset` to every value `encode_positions` wrote.
fn offset_codes(mesh: &mut Mesh, offset: u32) {
    let vertices: Vec<VertexId> = mesh.vertices().collect();
    let corners: Vec<HalfEdgeId> = mesh
        .faces()
        .flat_map(|f| mesh.face_loop(f).collect::<Vec<_>>())
        .collect();
    let mut edit = mesh.edit();
    for vertex in vertices {
        let p = *edit.mesh().vertex_position(vertex).expect("live");
        op::set_attribute(&mut edit, VERTEX_CODE, vertex, code(p) + offset).expect("vertex");
    }
    for corner in corners {
        let vertex = edit.mesh().to_vertex(corner).expect("corner");
        let p = *edit.mesh().vertex_position(vertex).expect("live");
        op::set_attribute(&mut edit, CORNER_CODE, corner, code(p) + offset).expect("corner");
    }
    let _: () = edit.finish();
}

#[test]
fn operand_vertices_on_the_other_operands_faces_keep_their_own_values() {
    // B is A's corner sub-box: its faces are coplanar with A's, so the union
    // is A's shape and B's vertices lie on A's faces or at A's corner.
    let mut a = cuboid([0.0; 3], [2.0; 3]);
    let mut b = cuboid([1.0; 3], [1.0; 3]);
    encode_positions(&mut a);
    encode_positions(&mut b);
    offset_codes(&mut b, 1000);
    let output = crate::boolean::boolean_mesh(
        &a,
        &b,
        crate::boolean::BooleanOp::Union,
        FaceTriangulation::Fan,
        &mut crate::boolean::BooleanScratch::new(),
        &mut crate::boolean::BooleanDiagnostics::default(),
    )
    .expect("union");
    let mesh = &output.mesh;
    let from_b: Vec<VertexId> = output
        .vertex_provenance
        .iter()
        .filter(|&&(_, side, _)| side == crate::boolean::MeshSide::B)
        .map(|&(vertex, _, _)| vertex)
        .collect();
    let mut b_only = 0;
    for vertex in mesh.vertices() {
        let p = *mesh.vertex_position(vertex).expect("live");
        let value = mesh
            .attrs()
            .sparse(VERTEX_CODE)
            .and_then(|l| l.get(vertex.as_id()).copied());
        if p.contains(&1.0) {
            // Only B has vertices with a coordinate at 1.
            assert!(from_b.contains(&vertex), "B vertex at {p:?} has provenance");
            assert_eq!(value, Some(code(p) + 1000), "B vertex at {p:?}");
            b_only += 1;
        } else {
            // A's own vertices, including the corner both operands share.
            assert_eq!(value, Some(code(p)), "A vertex at {p:?}");
        }
    }
    assert!(b_only > 0, "some B vertices lie on A's faces");
}

#[test]
fn vertex_provenance_names_only_live_output_vertices() {
    // Touching and overlapping pairs whose shared original vertices can be
    // welded and then deleted as isolated (for example an empty
    // intersection of face-adjacent boxes).
    let pairs = [
        (
            cuboid([0.0; 3], [1.0; 3]),
            cuboid([1.0, 0.0, 0.0], [1.0; 3]),
        ),
        (
            cuboid([0.0; 3], [1.0; 3]),
            cuboid([1.0, 1.0, 0.0], [1.0; 3]),
        ),
        (cuboid([0.0; 3], [2.0; 3]), cuboid([1.0; 3], [1.0; 3])),
        (
            cuboid([0.0; 3], [2.0; 3]),
            cuboid([1.0, 1.0, -1.0], [2.0; 3]),
        ),
    ];
    let mut rows = 0;
    for (a, b) in &pairs {
        for op in [
            crate::boolean::BooleanOp::Difference,
            crate::boolean::BooleanOp::Union,
            crate::boolean::BooleanOp::Intersection,
        ] {
            let Ok(output) = crate::boolean::boolean_mesh(
                a,
                b,
                op,
                FaceTriangulation::Fan,
                &mut crate::boolean::BooleanScratch::new(),
                &mut crate::boolean::BooleanDiagnostics::default(),
            ) else {
                continue;
            };
            let live: Vec<VertexId> = output.mesh.vertices().collect();
            for &(vertex, _, _) in &output.vertex_provenance {
                assert!(live.contains(&vertex), "{op:?}: row names a dead vertex");
                rows += 1;
            }
        }
    }
    assert!(rows > 0, "some rows were checked");
}

const VERTEX_CODE_B: AttrKey<u32> = AttrKey::new(Domain::Vertex, "vertex.code.b");

#[test]
fn layers_only_one_operand_carries_keep_its_values_at_shared_vertices() {
    // B is A's corner sub-box, so (2, 2, 2) is original in both operands.
    // A carries `vertex.code`; B carries only `vertex.code.b`.
    let mut a = cuboid([0.0; 3], [2.0; 3]);
    encode_positions(&mut a);
    let mut b = cuboid([1.0; 3], [1.0; 3]);
    b.define_sparse_layer(VERTEX_CODE_B).expect("b layer");
    b.set_layer_propagation(VERTEX_CODE_B, Propagation::Copy)
        .expect("copy rule");
    let vertices: Vec<VertexId> = b.vertices().collect();
    let mut edit = b.edit();
    for vertex in vertices {
        let p = *edit.mesh().vertex_position(vertex).expect("live");
        op::set_attribute(&mut edit, VERTEX_CODE_B, vertex, code(p) + 1000).expect("b code");
    }
    let _: () = edit.finish();

    let output = crate::boolean::boolean_mesh(
        &a,
        &b,
        crate::boolean::BooleanOp::Union,
        FaceTriangulation::Fan,
        &mut crate::boolean::BooleanScratch::new(),
        &mut crate::boolean::BooleanDiagnostics::default(),
    )
    .expect("union");
    let mesh = &output.mesh;
    let shared = mesh
        .vertices()
        .find(|&v| mesh.vertex_position(v) == Some(&[2.0, 2.0, 2.0]))
        .expect("shared corner");
    let get = |key: AttrKey<u32>| {
        mesh.attrs()
            .sparse(key)
            .and_then(|l| l.get(shared.as_id()).copied())
    };
    assert_eq!(get(VERTEX_CODE), Some(code([2.0; 3])), "A's layer");
    assert_eq!(
        get(VERTEX_CODE_B),
        Some(code([2.0; 3]) + 1000),
        "B-only layer"
    );
    assert_eq!(output.stats.unpropagated_attribute_values, 0);
}

#[test]
fn collinear_source_vertices_stay_exact_far_from_the_origin() {
    // The collinear box, rotated off the axes and moved far out. A point on
    // the loop edge from the T-vertex toward (2, 0, 2), computed in f64 and
    // narrowed as a split stores it, must still sample the T-vertex's corner
    // as heaviest: far from the origin narrowing moves it farther than an
    // edge-relative tolerance allows.
    for (offset, t) in [(300.0, 0.2), (500.0, 0.1), (1000.0, 0.2)] {
        let mut mesh = box_with_collinear_vertex();
        let placement = Placement3::euler_extrinsic_xyz_then_translate(
            0.3,
            -0.7,
            1.1,
            [offset, -offset, offset],
        );
        let place = |p: [f64; 3]| {
            let r = placement.rows;
            [0, 1, 2].map(|i| r[i][0] * p[0] + r[i][1] * p[1] + r[i][2] * p[2] + r[i][3])
        };
        let vertices: Vec<VertexId> = mesh.vertices().collect();
        let mut edit = mesh.edit();
        for vertex in vertices {
            let p = exedra_math::promote(*edit.mesh().vertex_position(vertex).expect("live"));
            op::set_vertex_position(&mut edit, vertex, exedra_math::narrow(place(p)))
                .expect("move");
        }
        let _: () = edit.finish();
        let t_vertex = mesh
            .vertices()
            .find(|&v| {
                mesh.attrs()
                    .sparse(VERTEX_CODE)
                    .and_then(|l| l.get(v.as_id()).copied())
                    == Some(204)
            })
            .expect("T-vertex");
        let top = mesh
            .faces()
            .find(|&f| {
                mesh.face_loop(f).count() == 5 && {
                    let z: Vec<u32> = mesh
                        .face_loop(f)
                        .filter_map(|c| mesh.to_vertex(c))
                        .filter_map(|v| {
                            mesh.attrs()
                                .sparse(VERTEX_CODE)
                                .and_then(|l| l.get(v.as_id()).copied())
                        })
                        .collect();
                    z.contains(&404) && z.contains(&444)
                }
            })
            .expect("top face");
        let t_corner = mesh
            .face_loop(top)
            .find(|&c| mesh.to_vertex(c) == Some(t_vertex))
            .expect("T corner");
        // Along the edge in the stored (narrowed) coordinates, narrowed again
        // as a split would store the new vertex.
        let a = exedra_math::promote(*mesh.vertex_position(t_vertex).expect("live"));
        let far = mesh
            .vertices()
            .find(|&v| {
                mesh.attrs()
                    .sparse(VERTEX_CODE)
                    .and_then(|l| l.get(v.as_id()).copied())
                    == Some(404)
            })
            .expect("(2, 0, 2)");
        let b = exedra_math::promote(*mesh.vertex_position(far).expect("live"));
        let point = exedra_math::promote(exedra_math::narrow(
            [0, 1, 2].map(|i| a[i] + t * (b[i] - a[i])),
        ));
        let weights = crate::layers::Chart::new(&mesh, top).weights(point);
        assert_eq!(
            weights.first().map(|&(corner, _)| corner),
            Some(t_corner),
            "offset {offset}, t {t}: {weights:?}"
        );
        assert_eq!(weights.len(), 2, "offset {offset}, t {t}: on the edge");
    }
}
