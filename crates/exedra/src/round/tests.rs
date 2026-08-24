// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use super::*;
use crate::boolean::{
    BooleanDiagnostics, BooleanOp, BooleanScratch, SeamCleanupPolicy, boolean_mesh, cleanup_seams,
};
use crate::{FaceTriangulation, MeshBuilder, op};

fn box_mesh(length: f64, width: f64, height: f64) -> Mesh {
    #[expect(clippy::cast_possible_truncation, reason = "test geometry narrowing")]
    let narrow = |v: f64| v as f32;
    let (l, w, h) = (narrow(length), narrow(width), narrow(height));
    let positions = [
        [0.0, 0.0, 0.0],
        [l, 0.0, 0.0],
        [l, w, 0.0],
        [0.0, w, 0.0],
        [0.0, 0.0, h],
        [l, 0.0, h],
        [l, w, h],
        [0.0, w, h],
    ];
    let faces: [[u32; 4]; 6] = [
        [3, 2, 1, 0],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ];
    let mut builder = MeshBuilder::new();
    for p in positions {
        builder.push_vertex(p);
    }
    for face in faces {
        builder.add_face(&face).expect("valid box face");
    }
    builder.build().expect("valid box").mesh
}

/// An L-profile prism: concave cross-section with one reflex vertical edge.
fn l_prism(height: f32) -> Mesh {
    let section: [[f32; 2]; 6] = [
        [0.0, 0.0],
        [2.0, 0.0],
        [2.0, 1.0],
        [1.0, 1.0],
        [1.0, 2.0],
        [0.0, 2.0],
    ];
    let n = u32::try_from(section.len()).expect("small section");
    let mut builder = MeshBuilder::new();
    for z in [0.0, height] {
        for p in section {
            builder.push_vertex([p[0], p[1], z]);
        }
    }
    let bottom: Vec<u32> = (0..n).rev().collect();
    builder.add_face(&bottom).expect("bottom cap");
    let top: Vec<u32> = (n..2 * n).collect();
    builder.add_face(&top).expect("top cap");
    for i in 0..n {
        let j = (i + 1) % n;
        builder.add_face(&[i, j, n + j, n + i]).expect("side wall");
    }
    builder.build().expect("valid L prism").mesh
}

/// Marks every canonical interior edge matching `pick` as fully sharp.
fn tag_sharp(mesh: &mut Mesh, pick: impl Fn(&Mesh, HalfEdgeId) -> bool) {
    let mut targets = Vec::new();
    let mut seen = BTreeSet::new();
    for face in mesh.faces() {
        for half_edge in mesh.face_loop(face) {
            let Some(canonical) = mesh.canonical_edge(half_edge) else {
                continue;
            };
            if seen.insert(canonical) && pick(mesh, canonical) {
                targets.push(canonical);
            }
        }
    }
    let mut session = mesh.edit();
    for half_edge in targets {
        let _ = set_edge_sharpness(&mut session, half_edge, 1.0);
    }
    #[expect(unused_must_use, reason = "discard sink output")]
    {
        session.finish();
    }
}

fn edge_endpoints(mesh: &Mesh, half_edge: HalfEdgeId) -> ([f32; 3], [f32; 3]) {
    let from = mesh
        .from_vertex(half_edge)
        .and_then(|v| mesh.vertex_position(v))
        .copied()
        .expect("live edge");
    let to = mesh
        .to_vertex(half_edge)
        .and_then(|v| mesh.vertex_position(v))
        .copied()
        .expect("live edge");
    (from, to)
}

/// True when the edge is the vertical segment between the two positions
/// (in either direction).
fn is_edge_between(mesh: &Mesh, half_edge: HalfEdgeId, a: [f32; 3], b: [f32; 3]) -> bool {
    let (from, to) = edge_endpoints(mesh, half_edge);
    (from == a && to == b) || (from == b && to == a)
}

fn signed_volume(mesh: &Mesh) -> f64 {
    let mut volume = 0.0;
    for face in mesh.faces() {
        let corners: Vec<[f64; 3]> = mesh
            .face_loop(face)
            .filter_map(|h| mesh.to_vertex(h))
            .filter_map(|v| mesh.vertex_position(v))
            .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
            .collect();
        for i in 1..corners.len().saturating_sub(1) {
            let (a, b, c) = (corners[0], corners[i], corners[i + 1]);
            volume += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]);
        }
    }
    volume / 6.0
}

fn euler_characteristic(mesh: &Mesh) -> i64 {
    let vertices = i64::try_from(mesh.vertices().count()).expect("small");
    let faces = i64::try_from(mesh.faces().count()).expect("small");
    let half_edges: usize = mesh.faces().map(|face| mesh.face_loop(face).count()).sum();
    let edges = i64::try_from(half_edges).expect("small") / 2;
    vertices - edges + faces
}

type Snapshot = (Vec<Vec<u32>>, Vec<[u32; 3]>);

fn snapshot(mesh: &Mesh) -> Snapshot {
    let faces: Vec<Vec<u32>> = mesh
        .faces()
        .map(|face| {
            mesh.face_loop(face)
                .filter_map(|h| mesh.to_vertex(h))
                .map(VertexId::index)
                .collect()
        })
        .collect();
    let positions: Vec<[u32; 3]> = mesh
        .vertices()
        .filter_map(|v| mesh.vertex_position(v))
        .map(|p| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()])
        .collect();
    (faces, positions)
}

fn exact_snapshot(mesh: &Mesh) -> alloc::string::String {
    alloc::format!("{mesh:?}")
}

fn assert_clean(mesh: &Mesh) {
    let errors = mesh.validate_deep();
    assert!(errors.is_empty(), "validate_deep: {errors:?}");
}

/// Closed-form volume of a box with every edge filleted at radius `r`
/// (Minkowski sum of the shrunk box with a ball).
fn rounded_box_volume(l: f64, w: f64, h: f64, r: f64) -> f64 {
    let (a, b, c) = (l - 2.0 * r, w - 2.0 * r, h - 2.0 * r);
    a * b * c
        + 2.0 * r * (a * b + b * c + a * c)
        + core::f64::consts::PI * r * r * (a + b + c)
        + 4.0 / 3.0 * core::f64::consts::PI * r * r * r
}

#[test]
fn fillet_of_every_box_edge_is_watertight_and_volume_close() {
    let (l, w, h, r) = (2.0, 1.5, 1.0, 0.2);
    let mut mesh = box_mesh(l, w, h);
    tag_sharp(&mut mesh, |_, _| true);

    let mut policy = RoundPolicy::fillet(r);
    policy.segments = Some(4);
    policy.region = Some(9);
    let stats = round_sharp_edges(&mut mesh, &policy).expect("full box fillet");

    assert_eq!(stats.chains, 12);
    assert_eq!(stats.corners, 8);
    assert_eq!(stats.closed_chains, 0);
    assert_eq!(stats.strip_faces, 12 * 4);
    // Each corner ring has 3 * 4 edges, fanned around an apex.
    assert_eq!(stats.patch_faces, 8 * 12);
    assert_eq!(stats.rewritten_faces, 6);
    assert_clean(&mesh);
    assert_eq!(euler_characteristic(&mesh), 2);

    let volume = signed_volume(&mesh);
    let exact = rounded_box_volume(l, w, h, r);
    // Inscribed arcs remove slightly more material than the smooth fillet.
    assert!(volume <= exact + 1e-6, "volume {volume} vs exact {exact}");
    assert!(volume >= exact * 0.96, "volume {volume} vs exact {exact}");

    // No sharp edges survive: the chains were consumed.
    for face in mesh.faces() {
        for half_edge in mesh.face_loop(face) {
            assert!(mesh.edge_sharpness(half_edge).unwrap_or(0.0) < 0.5);
        }
    }
}

#[test]
fn fillet_of_one_box_edge_rounds_an_open_chain() {
    let (l, w, h, r) = (1.0, 1.0, 2.0, 0.25);
    let mut mesh = box_mesh(l, w, h);
    // Give every original face a region so preservation is observable.
    {
        let faces: Vec<FaceId> = mesh.faces().collect();
        let mut session = mesh.edit();
        for face in faces {
            let _ = set_face_region(&mut session, face, 3);
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            session.finish();
        }
    }
    #[expect(clippy::cast_possible_truncation, reason = "test corner coordinates")]
    let (lc, wc, hc) = (l as f32, w as f32, h as f32);
    tag_sharp(&mut mesh, |m, e| {
        is_edge_between(m, e, [lc, wc, 0.0], [lc, wc, hc])
    });

    let mut policy = RoundPolicy::fillet(r);
    policy.segments = Some(6);
    policy.region = Some(7);
    let stats = round_sharp_edges(&mut mesh, &policy).expect("single edge fillet");

    assert_eq!(stats.chains, 1);
    assert_eq!(stats.corners, 0);
    assert_eq!(stats.strip_faces, 6);
    assert_eq!(stats.patch_faces, 0);
    // Two flanks and two end caps.
    assert_eq!(stats.rewritten_faces, 4);
    assert_clean(&mesh);
    assert_eq!(euler_characteristic(&mesh), 2);

    let volume = signed_volume(&mesh);
    let exact = l * w * h - (1.0 - core::f64::consts::FRAC_PI_4) * r * r * h;
    assert!(volume < l * w * h);
    assert!(
        (volume - exact).abs() < 0.01 * r * r * h,
        "volume {volume} vs exact {exact}"
    );

    // Strip faces carry the policy region; rewritten faces keep theirs.
    let regions = mesh.attrs().dense(attr::FACE_REGION).expect("regions");
    let mut strip_count = 0;
    let mut kept_count = 0;
    for face in mesh.faces() {
        match regions.get(face.as_id()).copied() {
            Some(7) => strip_count += 1,
            Some(3) => kept_count += 1,
            other => panic!("face without an expected region: {other:?}"),
        }
    }
    assert_eq!(strip_count, 6);
    assert_eq!(kept_count, 6);
}

#[test]
fn chamfer_of_one_box_edge_removes_the_exact_wedge() {
    let (l, w, h, s) = (1.0, 1.0, 2.0, 0.3);
    let mut mesh = box_mesh(l, w, h);
    #[expect(clippy::cast_possible_truncation, reason = "test corner coordinates")]
    let (lc, wc, hc) = (l as f32, w as f32, h as f32);
    tag_sharp(&mut mesh, |m, e| {
        is_edge_between(m, e, [lc, wc, 0.0], [lc, wc, hc])
    });

    let policy = RoundPolicy::chamfer(s);
    let stats = round_sharp_edges(&mut mesh, &policy).expect("single edge chamfer");
    assert_eq!(stats.strip_faces, 1);
    assert_eq!(stats.max_segments, 1);
    assert_clean(&mesh);
    assert_eq!(euler_characteristic(&mesh), 2);

    let volume = signed_volume(&mesh);
    let exact = l * w * h - 0.5 * s * s * h;
    assert!(
        (volume - exact).abs() < 1e-5,
        "volume {volume} vs exact {exact}"
    );
}

#[test]
fn chamfer_of_every_box_edge_builds_corner_triangles() {
    let mut mesh = box_mesh(2.0, 1.5, 1.0);
    tag_sharp(&mut mesh, |_, _| true);
    let policy = RoundPolicy::chamfer(0.15);
    let stats = round_sharp_edges(&mut mesh, &policy).expect("full box chamfer");
    assert_eq!(stats.chains, 12);
    assert_eq!(stats.corners, 8);
    assert_eq!(stats.strip_faces, 12);
    // Chamfer corner rings are triangles.
    assert_eq!(stats.patch_faces, 8);
    assert_clean(&mesh);
    assert_eq!(euler_characteristic(&mesh), 2);
}

#[test]
fn l_prism_convex_edge_rounds_and_reflex_edge_refuses() {
    // Convex vertical edge at (2, 0).
    let mut mesh = l_prism(1.0);
    tag_sharp(&mut mesh, |m, e| {
        is_edge_between(m, e, [2.0, 0.0, 0.0], [2.0, 0.0, 1.0])
    });
    let mut policy = RoundPolicy::fillet(0.2);
    policy.segments = Some(4);
    let stats = round_sharp_edges(&mut mesh, &policy).expect("convex L edge fillet");
    assert_eq!(stats.chains, 1);
    assert_clean(&mesh);
    assert_eq!(euler_characteristic(&mesh), 2);

    // Reflex vertical edge at (1, 1): typed refusal, mesh untouched.
    let mut mesh = l_prism(1.0);
    tag_sharp(&mut mesh, |m, e| {
        is_edge_between(m, e, [1.0, 1.0, 0.0], [1.0, 1.0, 1.0])
    });
    let before = snapshot(&mesh);
    let error = round_sharp_edges(&mut mesh, &policy).expect_err("reflex edge must refuse");
    assert!(matches!(error, RoundError::ConcaveEdge { .. }), "{error:?}");
    assert_eq!(snapshot(&mesh), before);
}

#[test]
fn sharp_turns_and_boundary_edges_refuse_untouched() {
    // The top rectangle is a closed ring with 90-degree turns.
    let mut mesh = box_mesh(1.0, 1.0, 1.0);
    tag_sharp(&mut mesh, |m, e| {
        let (a, b) = edge_endpoints(m, e);
        a[2] == 1.0 && b[2] == 1.0
    });
    let before = snapshot(&mesh);
    let error = round_sharp_edges(&mut mesh, &RoundPolicy::fillet(0.1))
        .expect_err("square ring must refuse");
    assert!(
        matches!(error, RoundError::UnsupportedJunction { .. }),
        "{error:?}"
    );
    assert_eq!(snapshot(&mesh), before);

    // A lone quad has boundary edges only.
    let mut builder = MeshBuilder::new();
    for p in [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ] {
        builder.push_vertex(p);
    }
    builder.add_face(&[0, 1, 2, 3]).expect("quad");
    let mut sheet = builder.build().expect("valid sheet").mesh;
    tag_sharp(&mut sheet, |_, _| true);
    let before = snapshot(&sheet);
    let error = round_sharp_edges(&mut sheet, &RoundPolicy::fillet(0.1))
        .expect_err("boundary edges must refuse");
    assert!(
        matches!(error, RoundError::BoundaryEdge { .. }),
        "{error:?}"
    );
    assert_eq!(snapshot(&sheet), before);

    // Invalid policy refuses before touching anything.
    let mut mesh = box_mesh(1.0, 1.0, 1.0);
    let error = round_sharp_edges(&mut mesh, &RoundPolicy::fillet(0.0))
        .expect_err("zero radius must refuse");
    assert!(
        matches!(error, RoundError::InvalidPolicy { .. }),
        "{error:?}"
    );
}

#[test]
fn rounding_without_selection_is_a_noop() {
    let mut mesh = box_mesh(1.0, 1.0, 1.0);
    let before = snapshot(&mesh);
    let stats = round_sharp_edges(&mut mesh, &RoundPolicy::fillet(0.1)).expect("no-op");
    assert_eq!(stats, RoundStats::default());
    assert_eq!(snapshot(&mesh), before);
}

#[test]
fn rounding_refuses_a_zero_length_sharp_edge_without_welding_it() {
    // Rounding is a geometric rewrite, not an identity repair pass. This
    // topologically manifold cube contains two distinct coincident vertices
    // on one marked edge and must fail atomically with the typed geometry
    // error instead of merging those identities by coordinate.
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
    ];
    let faces: [&[u32]; 6] = [
        &[3, 2, 1, 8, 0],
        &[4, 5, 6, 7],
        &[0, 8, 1, 5, 4],
        &[1, 2, 6, 5],
        &[2, 3, 7, 6],
        &[3, 0, 4, 7],
    ];
    let mut builder = MeshBuilder::new();
    for position in positions {
        builder.push_vertex(position);
    }
    for face in faces {
        builder.add_face(face).expect("topologically valid face");
    }
    let mut mesh = builder.build().expect("manifold cube topology").mesh;
    tag_sharp(&mut mesh, |mesh, edge| {
        mesh.from_vertex(edge)
            .and_then(|vertex| mesh.vertex_position(vertex))
            .zip(
                mesh.to_vertex(edge)
                    .and_then(|vertex| mesh.vertex_position(vertex)),
            )
            .is_some_and(|(from, to)| from == to)
    });
    let before = exact_snapshot(&mesh);

    let error = round_sharp_edges(&mut mesh, &RoundPolicy::fillet(0.1))
        .expect_err("zero-length selected edge is invalid input");

    assert!(matches!(error, RoundError::DegenerateEdge { .. }));
    assert_eq!(exact_snapshot(&mesh), before);
}

#[test]
fn rounding_is_deterministic() {
    let build = || {
        let mut mesh = box_mesh(2.0, 1.5, 1.0);
        tag_sharp(&mut mesh, |_, _| true);
        let mut policy = RoundPolicy::fillet(0.2);
        policy.segments = Some(3);
        round_sharp_edges(&mut mesh, &policy).expect("fillet");
        mesh
    };
    assert_eq!(snapshot(&build()), snapshot(&build()));
}

// --- Boolean-output fixtures: the drilled slab's seam rims. ---------------

fn slab() -> Mesh {
    box_mesh(4.0, 4.0, 1.0)
}

fn drill_prism() -> Mesh {
    let n = 16_u32;
    let mut builder = MeshBuilder::new();
    for z in [-1.0_f64, 2.0] {
        for i in 0..n {
            let angle = core::f64::consts::TAU * f64::from(i) / f64::from(n);
            let position = [2.0 + 0.8 * angle.cos(), 2.0 + 0.8 * angle.sin(), z];
            #[expect(clippy::cast_possible_truncation, reason = "test geometry narrowing")]
            builder.push_vertex([position[0] as f32, position[1] as f32, position[2] as f32]);
        }
    }
    let bottom: Vec<u32> = (0..n).rev().collect();
    builder.add_face(&bottom).expect("bottom cap");
    let top: Vec<u32> = (n..2 * n).collect();
    builder.add_face(&top).expect("top cap");
    for i in 0..n {
        let j = (i + 1) % n;
        builder.add_face(&[i, j, n + j, n + i]).expect("side wall");
    }
    builder.build().expect("valid prism").mesh
}

fn drilled_slab() -> Mesh {
    drilled_slab_rotated(0.0)
}

fn rotate_z(mesh: &mut Mesh, angle: f32) {
    let (sin, cos) = angle.sin_cos();
    let positions: Vec<(VertexId, [f32; 3])> = mesh
        .vertices()
        .filter_map(|vertex| mesh.vertex_position(vertex).copied().map(|p| (vertex, p)))
        .collect();
    let mut session = mesh.edit();
    for (vertex, [x, y, z]) in positions {
        op::set_vertex_position(
            &mut session,
            vertex,
            [x * cos - y * sin, x * sin + y * cos, z],
        )
        .expect("collected vertex remains live");
    }
    #[expect(unused_must_use, reason = "discard sink output")]
    {
        session.finish();
    }
}

fn drilled_slab_rotated(angle: f32) -> Mesh {
    let mut scratch = BooleanScratch::new();
    let mut diagnostics = BooleanDiagnostics::default();
    let output = boolean_mesh(
        &slab(),
        &drill_prism(),
        BooleanOp::Difference,
        FaceTriangulation::Fan,
        &mut scratch,
        &mut diagnostics,
    )
    .expect("drill boolean succeeds");
    let mut mesh = output.mesh;
    rotate_z(&mut mesh, angle);
    mesh
}

#[test]
fn drilled_rim_chamfer_keeps_the_hole_watertight() {
    let mut mesh = drilled_slab();
    assert_eq!(euler_characteristic(&mesh), 0);
    let before_volume = signed_volume(&mesh);

    let mut policy = RoundPolicy::chamfer(0.04);
    policy.region = Some(21);
    let stats = round_sharp_edges(&mut mesh, &policy).expect("rim chamfer");

    // The drill pierces both caps: two seam rims, both closed rings.
    assert_eq!(stats.chains, 2);
    assert_eq!(stats.closed_chains, 2);
    assert_eq!(stats.corners, 0);
    assert!(stats.strip_faces > 0);
    assert_clean(&mesh);
    assert_eq!(euler_characteristic(&mesh), 0, "genus preserved");
    let after_volume = signed_volume(&mesh);
    assert!(
        after_volume < before_volume,
        "chamfer removes material: {after_volume} vs {before_volume}"
    );
    // Removed material stays in the right order of magnitude: each rim
    // removes roughly perimeter * s^2 / 2.
    let rim = 2.0 * core::f64::consts::PI * 0.8;
    let removed = before_volume - after_volume;
    assert!(
        removed < 2.0 * rim * 0.04 * 0.04,
        "removed {removed} out of scale"
    );
}

#[test]
fn drilled_rim_fillet_is_deterministic_and_clean() {
    let build = || {
        let mut mesh = drilled_slab();
        let mut policy = RoundPolicy::fillet(0.05);
        policy.segments = Some(3);
        round_sharp_edges(&mut mesh, &policy).expect("rim fillet");
        mesh
    };
    let mesh = build();
    assert_clean(&mesh);
    assert_eq!(euler_characteristic(&mesh), 0, "genus preserved");
    assert_eq!(snapshot(&mesh), snapshot(&build()));
}

#[test]
fn cleaned_drilled_rim_rounding_never_panics_or_partially_rewrites() {
    // Seam cleanup may simplify collinear rim runs before rounding. Whatever
    // valid topology it returns, an unsupported fillet must fail atomically
    // instead of leaving a partial topology rewrite behind.
    for angle in [0.0, core::f32::consts::FRAC_PI_4] {
        let mut mesh = drilled_slab_rotated(angle);
        let _cleanup = cleanup_seams(&mut mesh, &SeamCleanupPolicy::default());

        let before = exact_snapshot(&mesh);
        let revision = mesh.revision();
        let error = round_sharp_edges(&mut mesh, &RoundPolicy::fillet(0.05))
            .expect_err("the cleaned rim is not currently roundable");

        assert_eq!(
            error,
            RoundError::UnsupportedTopology {
                detail: "face rewrite would pinch an OUTSIDE boundary vertex",
            }
        );
        assert_eq!(
            exact_snapshot(&mesh),
            before,
            "failed rounding must be atomic"
        );
        assert_eq!(mesh.revision(), revision, "failed rounding keeps revision");
    }
}
