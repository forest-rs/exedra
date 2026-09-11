// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use super::*;
use crate::boolean::{
    BooleanDiagnostics, BooleanOp, BooleanScratch, SeamCleanupPolicy, boolean_mesh, cleanup_seams,
};
use crate::{
    ChangeSetBuilder, ExtractParams, FaceTriangulation, MeshBuilder, NormalParams, NormalsSource,
    op,
};

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
    prism(&section, &[0.0, height])
}

fn prism(section: &[[f32; 2]], heights: &[f32]) -> Mesh {
    let n = u32::try_from(section.len()).expect("small section");
    let mut builder = MeshBuilder::new();
    for &z in heights {
        for p in section {
            builder.push_vertex([p[0], p[1], z]);
        }
    }
    let bottom: Vec<u32> = (0..n).rev().collect();
    builder.add_face(&bottom).expect("bottom cap");
    let top_offset = n * u32::try_from(heights.len() - 1).unwrap();
    let top: Vec<u32> = (top_offset..top_offset + n).collect();
    builder.add_face(&top).expect("top cap");
    for layer in 0..heights.len() - 1 {
        let offset = n * u32::try_from(layer).unwrap();
        for i in 0..n {
            let j = (i + 1) % n;
            builder
                .add_face(&[i, j, n + j, n + i].map(|index| index + offset))
                .expect("side wall");
        }
    }
    builder.build().expect("valid prism").mesh
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

fn four_quad_rewrite() -> (Mesh, Plan) {
    let mut builder = MeshBuilder::new();
    for y in [0.0, 1.0, 2.0] {
        for x in [0.0, 1.0, 2.0] {
            builder.push_vertex([x, y, 0.0]);
        }
    }
    for face in [[0, 1, 4, 3], [1, 2, 5, 4], [3, 4, 7, 6], [4, 5, 8, 7]] {
        builder.add_face(&face).unwrap();
    }
    let mesh = builder.build().unwrap().mesh;
    let affected: Vec<_> = mesh.faces().collect();
    // Opposite quads meet only at the center vertex until a third quad joins
    // them. Ascending source-face order need not be a valid insertion order.
    let faces = [0, 3, 1, 2]
        .map(|index| {
            let face = affected[index];
            let entries = mesh
                .face_loop(face)
                .map(|h| Tok::Old(mesh.to_vertex(h).unwrap()))
                .collect();
            NewFace {
                entries,
                region: Some(face.index() + 10),
                source: RoundFaceSource::Face(face),
                normals: alloc::vec![[0.0, 0.0, 1.0]; 4],
                uvs: alloc::vec![Some([f32::from(u16::try_from(index).unwrap()), 0.5]); 4],
            }
        })
        .to_vec();
    let plan = Plan {
        points: Vec::new(),
        faces,
        affected,
        consumed: Vec::new(),
        edge_attrs: Vec::new(),
        stats: RoundStats::default(),
    };
    (mesh, plan)
}

#[test]
fn deferred_rewrite_keeps_face_sources_and_corner_attributes() {
    let (mut mesh, plan) = four_quad_rewrite();
    let expected = plan.faces.clone();
    let result = apply(&mut mesh, plan).unwrap();
    assert_clean(&mesh);
    assert_eq!(result.face_provenance.len(), expected.len());
    for ((face, source), planned) in result.face_provenance.iter().zip(expected) {
        assert_eq!(*source, planned.source);
        assert_eq!(
            mesh.attrs()
                .dense(attr::FACE_REGION)
                .unwrap()
                .get(face.as_id())
                .copied(),
            planned.region
        );
        for (index, corner) in mesh.face_loop(*face).enumerate() {
            assert_eq!(
                mesh.attrs()
                    .sparse(attr::CORNER_UV)
                    .unwrap()
                    .get(corner.as_id())
                    .copied(),
                planned.uvs[index]
            );
            assert_eq!(
                mesh.attrs()
                    .sparse(attr::CORNER_NORMAL_OVERRIDE)
                    .unwrap()
                    .get(corner.as_id()),
                Some(&planned.normals[index])
            );
        }
    }
    for vertex in mesh.vertices() {
        let incident = mesh
            .half_edges
            .iter()
            .filter(|(id, _)| mesh.from_vertex(HalfEdgeId::from(*id)) == Some(vertex))
            .count();
        assert_eq!(
            mesh.vertex_star(vertex).count(),
            incident,
            "single connected fan"
        );
    }
}

#[test]
fn unresolved_rewrite_pinch_is_refused_atomically() {
    let (mut mesh, mut plan) = four_quad_rewrite();
    // With the two connecting quads missing, the center remains a real pinch.
    plan.faces.truncate(2);
    let before = exact_snapshot(&mesh);
    let revision = mesh.revision();
    assert_eq!(
        apply(&mut mesh, plan).unwrap_err(),
        RoundError::UnsupportedTopology {
            detail: "face rewrite would pinch an OUTSIDE boundary vertex",
        }
    );
    assert_eq!(exact_snapshot(&mesh), before);
    assert_eq!(mesh.revision(), revision);
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
fn construction_rail_fillet_samples_match_radius_and_tangent_normals() {
    let size = [0.09, 0.2, 2.0];
    let radius = 0.006;
    let mut mesh = box_mesh(size[0], size[1], size[2]);
    tag_sharp(&mut mesh, |_, _| true);
    let mut policy = RoundPolicy::fillet(radius);
    policy.segments = Some(8);
    round_sharp_edges(&mut mesh, &policy).expect("construction rail fillet");
    assert_clean(&mesh);
    let (render, _) = mesh.to_trimesh(&ExtractParams {
        normals: NormalsSource::CustomOrDerived,
        ..ExtractParams::default()
    });
    let mut curved = 0;
    for (point, normal) in render.positions.iter().zip(&render.normals) {
        let point = point.map(f64::from);
        let center = core::array::from_fn(|axis| point[axis].clamp(radius, size[axis] - radius));
        let ray = sub(point, center);
        let distance = norm(ray);
        assert!(
            (distance - radius).abs() < 2e-7,
            "radius at {point:?}: {distance}"
        );
        let expected = scale(ray, 1.0 / distance);
        let actual = normal.map(f64::from);
        assert!(
            dot(expected, actual) > 0.99999,
            "normal at {point:?}: {actual:?}, expected {expected:?}"
        );
        if expected.iter().filter(|v| v.abs() > 1e-5).count() > 1 {
            curved += 1;
        }
    }
    assert!(curved > 100, "sample the strips and spherical corners");
}

#[test]
fn construction_rail_clearance_is_refused_atomically_at_the_limit() {
    let limit = f64::from(0.09_f32) * 0.5;
    for radius in [limit - 0.0001, limit, limit + 0.0001] {
        let mut mesh = box_mesh(0.09, 0.2, 2.0);
        tag_sharp(&mut mesh, |_, _| true);
        let before = exact_snapshot(&mesh);
        let result = round_sharp_edges(&mut mesh, &RoundPolicy::fillet(radius));
        if radius < limit {
            result.expect("radius below half the rail width");
            assert_clean(&mesh);
        } else {
            assert!(
                matches!(result, Err(RoundError::ClearanceExceeded { .. })),
                "{radius}: {result:?}"
            );
            assert_eq!(exact_snapshot(&mesh), before);
        }
    }
}

#[test]
fn oversized_cube_finish_is_refused_even_when_face_winding_stays_positive() {
    for offset in [0.49, 0.5, 0.6, 2.0] {
        for policy in [RoundPolicy::fillet(offset), RoundPolicy::chamfer(offset)] {
            let mut mesh = box_mesh(1.0, 1.0, 1.0);
            tag_sharp(&mut mesh, |_, _| true);
            let before = exact_snapshot(&mesh);
            let result = round_sharp_edges(&mut mesh, &policy);
            if offset < 0.5 {
                result.expect("offset just below clearance");
                assert_clean(&mesh);
            } else {
                assert!(
                    matches!(result, Err(RoundError::ClearanceExceeded { .. })),
                    "{policy:?}: {result:?}"
                );
                assert_eq!(exact_snapshot(&mesh), before);
            }
        }
    }
}

#[test]
fn corner_patch_surfaces_follow_requested_chord_tolerance() {
    let radius = 0.006;
    for (tolerance, triangle_budget) in [(0.0002, 300), (0.00005, 1200), (0.00002, 4200)] {
        let mut policy = RoundPolicy::fillet(radius);
        policy.chord_tolerance = tolerance;
        let mut mesh = box_mesh(0.09, 0.2, 2.0);
        tag_sharp(&mut mesh, |_, _| true);
        let edges: Vec<_> = mesh.faces().flat_map(|f| mesh.face_loop(f)).collect();
        let result = round_edges(&mut mesh, &edges, &policy).unwrap();
        assert_clean(&mesh);
        assert_eq!(euler_characteristic(&mesh), 2);
        // The fine case used to emit 7,440 corner triangles by rounding its
        // radial layer count up to 16. Leave room for tessellation changes
        // while keeping that oversampling from returning.
        assert!(result.stats.patch_faces <= triangle_budget);
        let mut worst = 0.0_f64;
        let mut samples = 0;
        for (face, source) in result.face_provenance {
            if !matches!(source, RoundFaceSource::Corner(_)) {
                continue;
            }
            for triangle in mesh.face_triangles(face, FaceTriangulation::Fan) {
                let p = triangle
                    .map(|e| promote(*mesh.vertex_position(mesh.to_vertex(e).unwrap()).unwrap()));
                let centroid = scale(add(add(p[0], p[1]), p[2]), 1.0 / 3.0);
                let center = [
                    centroid[0].clamp(radius, 0.09 - radius),
                    centroid[1].clamp(radius, 0.2 - radius),
                    centroid[2].clamp(radius, 2.0 - radius),
                ];
                assert!(dot(newell(&p), sub(centroid, center)) > 0.0);
                // An independent barycentric grid samples the triangle interior
                // and its edges, including where vertex-only radius checks miss.
                for i in 0..=8 {
                    for j in 0..=8 - i {
                        let u = f64::from(i) / 8.0;
                        let v = f64::from(j) / 8.0;
                        let point = add(
                            add(scale(p[0], u), scale(p[1], v)),
                            scale(p[2], 1.0 - u - v),
                        );
                        let center = [
                            point[0].clamp(radius, 0.09 - radius),
                            point[1].clamp(radius, 0.2 - radius),
                            point[2].clamp(radius, 2.0 - radius),
                        ];
                        worst = worst.max((radius - norm(sub(point, center))).abs());
                        samples += 1;
                    }
                }
            }
        }
        assert!(samples > 100);
        assert!(
            worst <= tolerance + 2e-7,
            "corner deviation {worst}, requested {tolerance}"
        );
    }
}

#[test]
fn explicit_foot_chamfer_preserves_untargeted_edges_and_reports_face_sources() {
    let mut original = box_mesh(0.09, 0.2, 0.1);
    tag_sharp(&mut original, |_, _| true);
    let target = original
        .faces()
        .flat_map(|f| original.face_loop(f))
        .find(|&e| is_edge_between(&original, e, [0.09, 0.2, 0.0], [0.09, 0.2, 0.1]))
        .unwrap();
    let twin = original.twin(target).unwrap();
    let source_regions: BTreeMap<_, _> = original.faces().map(|f| (f, f.index() + 10)).collect();
    let normals = original.derive_corner_normals(&NormalParams::default());
    let corners: Vec<_> = original
        .faces()
        .flat_map(|f| original.face_loop(f))
        .collect();
    let mut edit = original.edit_with(ChangeSetBuilder::new());
    for (&face, &region) in &source_regions {
        set_face_region(&mut edit, face, region).unwrap();
    }
    for corner in corners {
        set_corner_uv(&mut edit, corner, [0.25, 0.75]).unwrap();
        set_corner_normal_override(&mut edit, corner, normals.get(corner)).unwrap();
    }
    let _ = edit.finish();
    let mut mesh = original.clone();
    let policy = RoundPolicy::chamfer(0.005);
    let result = round_edges(&mut mesh, &[target, twin, target], &policy).unwrap();
    assert_eq!(
        result.stats.chains, 1,
        "explicit selection must not round the other sharp edges"
    );
    assert_eq!(result.stats.strip_faces, 1);
    assert_clean(&mesh);
    let sources: BTreeMap<_, _> = result.face_provenance.iter().copied().collect();
    for face in mesh.faces() {
        let region = mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .unwrap()
            .get(face.as_id())
            .copied();
        if let Some(source) = sources.get(&face) {
            assert_eq!(region, Some(source_regions[&source.faces()[0]]));
            for corner in mesh.face_loop(face) {
                assert_eq!(
                    mesh.attrs()
                        .sparse(attr::CORNER_UV)
                        .and_then(|uv| uv.get(corner.as_id())),
                    Some(&[0.25, 0.75])
                );
            }
        } else {
            assert_eq!(region, Some(source_regions[&face]));
            let old: Vec<_> = original
                .face_loop(face)
                .map(|e| (original.to_vertex(e), original.edge_sharpness(e)))
                .collect();
            let new: Vec<_> = mesh
                .face_loop(face)
                .map(|e| (mesh.to_vertex(e), mesh.edge_sharpness(e)))
                .collect();
            assert_eq!(old, new);
            for corner in mesh.face_loop(face) {
                assert_eq!(
                    mesh.attrs()
                        .sparse(attr::CORNER_UV)
                        .unwrap()
                        .get(corner.as_id()),
                    Some(&[0.25, 0.75])
                );
            }
        }
    }
    let band = sources
        .iter()
        .find_map(|(&face, source)| source.is_generated().then_some(face))
        .unwrap();
    for edge in mesh.face_loop(band) {
        assert_eq!(
            mesh.edge_sharpness(edge),
            Some(1.0),
            "flat chamfer must have hard boundaries"
        );
    }
    let mut repeated = original;
    assert_eq!(
        round_edges(&mut repeated, &[twin], &policy).unwrap(),
        result
    );
    assert_eq!(exact_snapshot(&mesh), exact_snapshot(&repeated));
    let before = exact_snapshot(&mesh);
    assert_eq!(
        round_edges(&mut mesh, &[target], &policy),
        Err(RoundError::InvalidEdge { edge: target })
    );
    assert_eq!(exact_snapshot(&mesh), before);
    assert_eq!(
        round_edges(&mut mesh, &[], &policy).unwrap(),
        RoundResult::default()
    );
    assert_eq!(exact_snapshot(&mesh), before);
}

/// Deliberately asymmetric charts catch a one-corner rotation as well as lost
/// scale, translation, reflection, or source-face ownership.
fn box_uv(face: FaceId, p: [f32; 3]) -> [f32; 2] {
    let [u, v] = match face.index() {
        0 | 1 => [p[0], p[1]],
        2 | 4 => [p[0], p[2]],
        3 | 5 => [p[1], p[2]],
        _ => unreachable!("input box face"),
    };
    [2.0 * u + 0.5 * v - 3.0, -0.25 * u + 3.0 * v + 7.0]
}

#[test]
fn finishing_preserves_source_uv_charts_on_trims_bands_and_corners() {
    for policy in [RoundPolicy::chamfer(0.2), RoundPolicy::fillet(0.2)] {
        let mut mesh = box_mesh(2.0, 3.0, 4.0);
        let values: Vec<_> = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face).map(move |corner| (face, corner)))
            .map(|(face, corner)| {
                let p = *mesh
                    .vertex_position(mesh.to_vertex(corner).unwrap())
                    .unwrap();
                (corner, box_uv(face, p))
            })
            .collect();
        let mut edit = mesh.edit();
        for (corner, uv) in values {
            set_corner_uv(&mut edit, corner, uv).unwrap();
        }
        let _: () = edit.finish();
        let edges: Vec<_> = mesh.faces().flat_map(|f| mesh.face_loop(f)).collect();
        let result = round_edges(&mut mesh, &edges, &policy).unwrap();
        assert_eq!(result.stats.corners, 8);
        let sources: BTreeMap<_, _> = result.face_provenance.iter().copied().collect();
        for (face, source) in result.face_provenance {
            for corner in mesh.face_loop(face) {
                let p = *mesh
                    .vertex_position(mesh.to_vertex(corner).unwrap())
                    .unwrap();
                let expected = box_uv(source.faces()[0], p);
                let uv = mesh
                    .attrs()
                    .sparse(attr::CORNER_UV)
                    .and_then(|layer| layer.get(corner.as_id()))
                    .expect("finished corner must keep its source chart");
                for axis in 0..2 {
                    assert!(
                        (uv[axis] - expected[axis]).abs() < 2e-6,
                        "{source:?} at {p:?}: {uv:?} != {expected:?}"
                    );
                }
            }
            for edge in mesh.face_loop(face) {
                let twin = mesh.twin(edge).unwrap();
                let other = mesh.face(twin).unwrap();
                let uv = |corner: HalfEdgeId| {
                    mesh.attrs()
                        .sparse(attr::CORNER_UV)
                        .unwrap()
                        .get(corner.as_id())
                        .unwrap()
                        .map(f32::to_bits)
                };
                let differs = uv(edge) != uv(mesh.prev(twin).unwrap())
                    || uv(mesh.prev(edge).unwrap()) != uv(twin);
                if differs {
                    assert_eq!(mesh.edge_seam(edge), Some(true));
                }
                if source.faces()[0] == sources[&other].faces()[0] {
                    assert!(
                        !differs,
                        "one source chart must meet exactly across its generated faces"
                    );
                    assert!(!mesh.edge_seam(edge).unwrap_or(false));
                }
            }
        }
        assert_clean(&mesh);
    }
}

#[test]
fn partial_uv_charts_keep_surviving_corners_without_inventing_a_mapping() {
    let mut mesh = box_mesh(2.0, 3.0, 4.0);
    let target = mesh
        .faces()
        .flat_map(|f| mesh.face_loop(f))
        .find(|&edge| is_edge_between(&mesh, edge, [2.0, 3.0, 0.0], [2.0, 3.0, 4.0]))
        .unwrap();
    // One surviving corner on each source face; none is a complete chart.
    let mut original = BTreeMap::new();
    for face in mesh.faces() {
        let corner = mesh
            .face_loop(face)
            .find(|&corner| {
                let p = mesh
                    .vertex_position(mesh.to_vertex(corner).unwrap())
                    .unwrap();
                p[0] == 0.0 || p[1] == 0.0
            })
            .unwrap();
        original.insert(
            (face, mesh.to_vertex(corner).unwrap()),
            (corner, [-0.0, 0.75]),
        );
    }
    let mut edit = mesh.edit();
    for &(corner, uv) in original.values() {
        set_corner_uv(&mut edit, corner, uv).unwrap();
    }
    let _: () = edit.finish();
    let result = round_edges(&mut mesh, &[target], &RoundPolicy::fillet(0.2)).unwrap();
    let sources: BTreeMap<_, _> = result.face_provenance.into_iter().collect();
    let mut retained = 0;
    for face in mesh.faces() {
        let source = sources
            .get(&face)
            .copied()
            .unwrap_or(RoundFaceSource::Face(face));
        for corner in mesh.face_loop(face) {
            let uv = mesh
                .attrs()
                .sparse(attr::CORNER_UV)
                .unwrap()
                .get(corner.as_id())
                .copied();
            let expected = original
                .get(&(source.faces()[0], mesh.to_vertex(corner).unwrap()))
                .map(|&(_, uv)| uv);
            assert_eq!(
                uv.map(|uv| uv.map(f32::to_bits)),
                expected.map(|uv| uv.map(f32::to_bits))
            );
            retained += usize::from(uv.is_some());
        }
    }
    assert_eq!(retained, original.len());
}

#[test]
fn sub_precision_finishes_are_refused_atomically() {
    for offset in [1e-9, 1e-30] {
        for policy in [RoundPolicy::fillet(offset), RoundPolicy::chamfer(offset)] {
            let mut mesh = box_mesh(1.0, 1.0, 1.0);
            tag_sharp(&mut mesh, |_, _| true);
            let before = exact_snapshot(&mesh);
            let result = round_sharp_edges(&mut mesh, &policy);
            assert!(result.is_err(), "{policy:?}: {result:?}");
            assert_eq!(exact_snapshot(&mesh), before);
        }
    }
}

#[test]
fn unachievable_rounding_resolution_is_an_atomic_error() {
    let mut mesh = box_mesh(1.0, 1.0, 1.0);
    tag_sharp(&mut mesh, |_, _| true);
    for (segments, tolerance) in [(Some(0), 0.001), (Some(257), 0.001), (None, 1e-20)] {
        let before = exact_snapshot(&mesh);
        let mut policy = RoundPolicy::fillet(0.1);
        policy.segments = segments;
        policy.chord_tolerance = tolerance;
        assert!(matches!(
            round_sharp_edges(&mut mesh, &policy),
            Err(RoundError::InvalidPolicy { .. })
        ));
        assert_eq!(exact_snapshot(&mesh), before);
    }
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
    assert!(
        stats.patch_faces > 8 * 12,
        "corner interiors need their own samples"
    );
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
fn l_prism_convex_and_concave_edges_round() {
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

    // The reflex fillet adds material into the notch, with its cylinder
    // center in the void rather than inside the original solid.
    let mut mesh = l_prism(1.0);
    tag_sharp(&mut mesh, |m, e| {
        is_edge_between(m, e, [1.0, 1.0, 0.0], [1.0, 1.0, 1.0])
    });
    let stats = round_sharp_edges(&mut mesh, &policy).expect("concave L edge fillet");
    assert_eq!(stats.chains, 1);
    assert_clean(&mesh);
    assert_eq!(euler_characteristic(&mesh), 2);
    let swept = 4.0 * (core::f64::consts::FRAC_PI_2 / 4.0).sin_ext();
    let expected = 3.0 + 0.2 * 0.2 * (1.0 - swept * 0.5);
    assert!((signed_volume(&mesh) - expected).abs() < 1e-6);
}

#[test]
fn concave_notch_fillet_has_inward_radial_normals_and_outward_winding() {
    let mut mesh = l_prism(1.0);
    tag_sharp(&mut mesh, |m, e| {
        is_edge_between(m, e, [1.0, 1.0, 0.0], [1.0, 1.0, 1.0])
    });
    let mut policy = RoundPolicy::fillet(0.2);
    policy.segments = Some(8);
    policy.region = Some(91);
    round_sharp_edges(&mut mesh, &policy).unwrap();
    let mut bands = 0;
    for face in mesh.faces() {
        if mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .unwrap()
            .get(face.as_id())
            != Some(&91)
        {
            continue;
        }
        let points: Vec<_> = mesh
            .face_loop(face)
            .map(|corner| {
                promote(
                    *mesh
                        .vertex_position(mesh.to_vertex(corner).unwrap())
                        .unwrap(),
                )
            })
            .collect();
        let outward = normalize(newell(&points)).unwrap();
        for (corner, point) in mesh.face_loop(face).zip(points) {
            let inward = sub([1.2, 1.2, point[2]], point);
            assert!((norm(inward) - 0.2).abs() < 2e-7);
            let expected = normalize(inward).unwrap();
            let actual = promote(
                *mesh
                    .attrs()
                    .sparse(attr::CORNER_NORMAL_OVERRIDE)
                    .unwrap()
                    .get(corner.as_id())
                    .unwrap(),
            );
            assert!(dot(expected, actual) > 0.99999);
            assert!(dot(outward, actual) > 0.99);
        }
        bands += 1;
    }
    assert_eq!(bands, 8);
}

#[test]
fn separate_concave_shoulders_and_convex_edges_finish_in_one_pass() {
    let section = [
        [0.0, 0.0],
        [3.0, 0.0],
        [3.0, 2.0],
        [2.0, 2.0],
        [2.0, 1.0],
        [1.0, 1.0],
        [1.0, 2.0],
        [0.0, 2.0],
    ];
    // Boolean splitting can fragment each straight shoulder into several
    // collinear edges with distinct, coplanar flank faces.
    let original = prism(&section, &[0.0, 0.3, 0.7, 1.0]);
    for policy in [RoundPolicy::fillet(0.2), RoundPolicy::chamfer(0.2)] {
        let mut mesh = original.clone();
        tag_sharp(&mut mesh, |m, e| {
            let (a, b) = edge_endpoints(m, e);
            a[0] == b[0]
                && a[1] == b[1]
                && ((a[1] == 1.0 && (a[0] == 1.0 || a[0] == 2.0)) || (a[0] == 0.0 && a[1] == 0.0))
        });
        let mut repeat = mesh.clone();
        let stats = round_sharp_edges(&mut mesh, &policy).unwrap();
        round_sharp_edges(&mut repeat, &policy).unwrap();
        assert_eq!(exact_snapshot(&mesh), exact_snapshot(&repeat));
        assert_eq!(stats.chains, 3);
        assert_eq!(stats.corners, 0);
        assert_clean(&mesh);
        assert_eq!(euler_characteristic(&mesh), 2);
        // Two concave additions minus one equal convex removal.
        let delta = match policy.kind {
            RoundKind::Chamfer { .. } => 0.5 * 0.2 * 0.2,
            RoundKind::Fillet { .. } => {
                let segments = f64::from(stats.max_segments);
                0.2 * 0.2
                    * (1.0 - 0.5 * segments * (core::f64::consts::FRAC_PI_2 / segments).sin_ext())
            }
        };
        assert!((signed_volume(&mesh) - 5.0 - delta).abs() < 1e-6);
    }
}

#[test]
fn concave_junctions_and_overlapping_setbacks_refuse_atomically() {
    for policy in [RoundPolicy::fillet(1.1), RoundPolicy::chamfer(1.1)] {
        let mut mesh = l_prism(1.0);
        tag_sharp(&mut mesh, |m, e| {
            is_edge_between(m, e, [1.0, 1.0, 0.0], [1.0, 1.0, 1.0])
        });
        let before = exact_snapshot(&mesh);
        assert!(matches!(
            round_sharp_edges(&mut mesh, &policy),
            Err(RoundError::ClearanceExceeded { .. })
        ));
        assert_eq!(exact_snapshot(&mesh), before);
    }
    for all_edges in [false, true] {
        let mut mesh = l_prism(1.0);
        tag_sharp(&mut mesh, |m, e| {
            all_edges
                || is_edge_between(m, e, [1.0, 1.0, 0.0], [1.0, 1.0, 1.0])
                || is_edge_between(m, e, [1.0, 1.0, 1.0], [2.0, 1.0, 1.0])
        });
        let before = exact_snapshot(&mesh);
        assert!(matches!(
            round_sharp_edges(&mut mesh, &RoundPolicy::fillet(0.2)),
            Err(RoundError::ConcaveEdge { .. })
        ));
        assert_eq!(exact_snapshot(&mesh), before);
    }
    for closed in [false, true] {
        let mut mesh = recessed_panel();
        let edges: Vec<_> = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .filter(|&edge| {
                let (a, b) = edge_endpoints(&mesh, edge);
                a[2] == 0.4
                    && b[2] == 0.4
                    && (closed || (a[0] == 1.0 && b[0] == 1.0) || (a[1] == 0.75 && b[1] == 0.75))
            })
            .collect();
        let before = exact_snapshot(&mesh);
        let mut policy = RoundPolicy::fillet(0.02);
        policy.max_tangent_turn = core::f64::consts::PI;
        assert!(matches!(
            round_edges(&mut mesh, &edges, &policy),
            Err(RoundError::ConcaveEdge { .. })
        ));
        assert_eq!(exact_snapshot(&mesh), before);
    }
}

#[test]
fn concave_cap_fan_refuses_sub_precision_triangles_atomically() {
    let mut mesh = l_prism(1.0);
    tag_sharp(&mut mesh, |m, e| {
        is_edge_between(m, e, [1.0, 1.0, 0.0], [1.0, 1.0, 1.0])
    });
    let positions: Vec<_> = mesh
        .vertices()
        .map(|vertex| {
            (
                vertex,
                mesh.vertex_position(vertex).unwrap().map(|p| p + 10000.0),
            )
        })
        .collect();
    let mut edit = mesh.edit();
    for (vertex, p) in positions {
        op::set_vertex_position(&mut edit, vertex, p).unwrap();
    }
    let _: () = edit.finish();
    let before = exact_snapshot(&mesh);
    let mut policy = RoundPolicy::fillet(0.2);
    policy.segments = Some(256);
    assert!(matches!(
        round_sharp_edges(&mut mesh, &policy),
        Err(RoundError::ClearanceExceeded { .. })
    ));
    assert_eq!(exact_snapshot(&mesh), before);
}

#[test]
fn concave_chain_with_an_oblique_end_cap_refuses_atomically() {
    let mut mesh = l_prism(1.0);
    tag_sharp(&mut mesh, |m, e| {
        is_edge_between(m, e, [1.0, 1.0, 0.0], [1.0, 1.0, 1.0])
    });
    let positions: Vec<_> = mesh
        .vertices()
        .map(|vertex| {
            let mut p = *mesh.vertex_position(vertex).unwrap();
            if p[2] == 1.0 {
                p[2] += 0.1 * p[0];
            }
            (vertex, p)
        })
        .collect();
    let mut edit = mesh.edit();
    for (vertex, p) in positions {
        op::set_vertex_position(&mut edit, vertex, p).unwrap();
    }
    let _: () = edit.finish();
    let before = exact_snapshot(&mesh);
    // A large plane tolerance must not silently approximate the elliptical
    // section needed by an oblique cylinder/end-plane intersection.
    let mut policy = RoundPolicy::fillet(0.2);
    policy.max_planar_deviation = 1.0;
    assert!(matches!(
        round_sharp_edges(&mut mesh, &policy),
        Err(RoundError::UnsupportedEnd { .. })
    ));
    assert_eq!(exact_snapshot(&mesh), before);
}

#[test]
fn boolean_cut_through_housing_rounds_both_internal_shoulders() {
    let beam = box_mesh(3.0, 1.0, 2.0);
    let mut cutter = box_mesh(1.0, 1.2, 1.1);
    let positions: Vec<_> = cutter
        .vertices()
        .map(|vertex| {
            let p = *cutter.vertex_position(vertex).unwrap();
            (vertex, [p[0] + 1.0, p[1] - 0.1, p[2] + 1.0])
        })
        .collect();
    let mut edit = cutter.edit();
    for (vertex, point) in positions {
        op::set_vertex_position(&mut edit, vertex, point).unwrap();
    }
    let _: () = edit.finish();
    let mut scratch = BooleanScratch::default();
    let mut diagnostics = BooleanDiagnostics::default();
    let original = boolean_mesh(
        &beam,
        &cutter,
        BooleanOp::Difference,
        FaceTriangulation::Robust,
        &mut scratch,
        &mut diagnostics,
    )
    .unwrap()
    .mesh;
    let mut mesh = original.clone();
    // The Boolean marks additional seams; select only the two floor shoulders.
    let edges: Vec<_> = mesh
        .faces()
        .flat_map(|face| mesh.face_loop(face))
        .filter(|&edge| {
            let (a, b) = edge_endpoints(&mesh, edge);
            a[0] == b[0] && (a[0] == 1.0 || a[0] == 2.0) && a[2] == 1.0 && b[2] == 1.0
        })
        .collect();
    let before = exact_snapshot(&mesh);
    assert!(matches!(
        round_edges(&mut mesh, &edges, &RoundPolicy::fillet(0.2)),
        Err(RoundError::ClearanceExceeded { .. })
    ));
    assert_eq!(exact_snapshot(&mesh), before);
    // The setback must fit before the next existing cap-boundary vertex;
    // this pass does not dissolve collinear Boolean subdivisions.
    let radius = 0.02;
    let mut policy = RoundPolicy::fillet(radius);
    policy.segments = Some(8);
    let result = round_edges(&mut mesh, &edges, &policy).unwrap();
    assert_eq!(result.stats.chains, 2);
    assert_clean(&mesh);
    assert_eq!(euler_characteristic(&mesh), 2);
    let added =
        2.0 * radius * radius * (1.0 - 4.0 * (core::f64::consts::FRAC_PI_2 / 8.0).sin_ext());
    assert!((signed_volume(&mesh) - signed_volume(&original) - added).abs() < 1e-6);
    for face in mesh.faces() {
        assert!(
            !mesh
                .face_triangles_counted(face, FaceTriangulation::Robust)
                .1
        );
    }
}

#[test]
fn concave_fill_refuses_obstructions_in_its_conservative_clearance_prism() {
    for position in [1.02, 1.09, 1.3] {
        let mut builder = MeshBuilder::new();
        for (source, offset) in [
            (l_prism(1.0), [0.0; 3]),
            (box_mesh(0.01, 0.01, 0.2), [position, position, 0.4]),
        ] {
            let vertices: BTreeMap<_, _> = source
                .vertices()
                .map(|vertex| {
                    let p = *source.vertex_position(vertex).unwrap();
                    (
                        vertex,
                        builder.push_vertex(core::array::from_fn(|axis| p[axis] + offset[axis])),
                    )
                })
                .collect();
            for face in source.faces() {
                let indices: Vec<_> = source
                    .face_loop(face)
                    .map(|corner| vertices[&source.to_vertex(corner).unwrap()])
                    .collect();
                builder.add_face(&indices).unwrap();
            }
        }
        let mut mesh = builder.build().unwrap().mesh;
        tag_sharp(&mut mesh, |m, e| {
            is_edge_between(m, e, [1.0, 1.0, 0.0], [1.0, 1.0, 1.0])
        });
        let before = exact_snapshot(&mesh);
        let result = round_sharp_edges(&mut mesh, &RoundPolicy::fillet(0.2));
        if position < 1.2 {
            // 1.02 intersects the addition; 1.09 lies beyond the arc but
            // inside its triangular clearance envelope and is also refused.
            assert!(matches!(result, Err(RoundError::ClearanceExceeded { .. })));
            assert_eq!(exact_snapshot(&mesh), before);
        } else {
            result.unwrap();
            assert_clean(&mesh);
            assert_eq!(euler_characteristic(&mesh), 4);
        }
    }
}

#[test]
fn concave_chains_keep_their_radius_after_shear_and_oblique_placement() {
    let section = [
        [0.0, 0.0],
        [2.0, 0.0],
        [2.0, 1.0],
        [1.0, 1.0],
        [1.0, 2.0],
        [0.0, 2.0],
    ];
    let mut original = prism(&section, &[0.0, 0.3, 0.7, 1.0]);
    tag_sharp(&mut original, |m, e| {
        let (a, b) = edge_endpoints(m, e);
        a[0] == 1.0 && b[0] == 1.0 && a[1] == 1.0 && b[1] == 1.0
    });
    let axis = normalize([1.0, 2.0, 3.0]).unwrap();
    let rotate = |p| {
        add(
            add(
                scale(p, 0.7_f64.cos_ext()),
                scale(cross(axis, p), 0.7_f64.sin_ext()),
            ),
            scale(axis, dot(axis, p) * (1.0 - 0.7_f64.cos_ext())),
        )
    };
    for shear in [-0.4, 0.0, 0.4] {
        let mut mesh = original.clone();
        let positions: Vec<_> = mesh
            .vertices()
            .map(|vertex| {
                let mut p = promote(*mesh.vertex_position(vertex).unwrap());
                p[0] += shear * p[1];
                (vertex, narrow(add(rotate(p), [0.3, -0.7, 1.2])))
            })
            .collect();
        let mut edit = mesh.edit();
        for (vertex, p) in positions {
            op::set_vertex_position(&mut edit, vertex, p).unwrap();
        }
        let _: () = edit.finish();
        let before = signed_volume(&mesh);
        let mut policy = RoundPolicy::fillet(0.1);
        policy.segments = Some(8);
        let stats = round_sharp_edges(&mut mesh, &policy).unwrap();
        assert_eq!(stats.chains, 1);
        assert_clean(&mesh);
        let sweep = (-shear / (1.0 + shear * shear).sqrt_ext()).acos_ext();
        let tangent = (sweep * 0.5).sin_ext() / (sweep * 0.5).cos_ext();
        let added = 0.1 * 0.1 * (tangent - 4.0 * (sweep / 8.0).sin_ext());
        assert!((signed_volume(&mesh) - before - added).abs() < 1e-6);
    }
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

/// A 4 x 3 x 1 panel with a 2 x 1.5 rectangular pocket, 0.6 deep.
/// Separate coplanar top faces also exercise a rim whose common flank has
/// been split into several faces by a previous modeling operation.
fn recessed_panel() -> Mesh {
    let mut builder = MeshBuilder::new();
    for z in [0.0, 1.0] {
        for [x, y] in [[0.0, 0.0], [4.0, 0.0], [4.0, 3.0], [0.0, 3.0]] {
            builder.push_vertex([x, y, z]);
        }
    }
    for z in [1.0, 0.4] {
        for [x, y] in [[1.0, 0.75], [3.0, 0.75], [3.0, 2.25], [1.0, 2.25]] {
            builder.push_vertex([x, y, z]);
        }
    }
    builder.add_face(&[3, 2, 1, 0]).unwrap();
    for i in 0..4 {
        let j = (i + 1) % 4;
        builder.add_face(&[i, j, j + 4, i + 4]).unwrap();
        builder.add_face(&[i + 4, j + 4, j + 8, i + 8]).unwrap();
        builder.add_face(&[i + 8, j + 8, j + 12, i + 12]).unwrap();
    }
    builder.add_face(&[12, 13, 14, 15]).unwrap();
    builder.build().unwrap().mesh
}

fn square_rim(mesh: &Mesh, recessed: bool) -> Vec<HalfEdgeId> {
    mesh.faces()
        .flat_map(|f| mesh.face_loop(f))
        .filter(|&edge| {
            let (a, b) = edge_endpoints(mesh, edge);
            a[2] == 1.0
                && b[2] == 1.0
                && (!recessed
                    || [a, b]
                        .iter()
                        .all(|p| (1.0..=3.0).contains(&p[0]) && (0.75..=2.25).contains(&p[1])))
        })
        .collect()
}

#[test]
fn square_rim_chamfers_keep_the_requested_setback_through_miters() {
    let setback = 0.2;
    for recessed in [false, true] {
        let mut mesh = if recessed {
            recessed_panel()
        } else {
            box_mesh(4.0, 3.0, 1.0)
        };
        assert_clean(&mesh);
        let before = signed_volume(&mesh);
        let edges = square_rim(&mesh, recessed);
        let mut policy = RoundPolicy::chamfer(setback);
        policy.max_tangent_turn = core::f64::consts::FRAC_PI_2;
        let result = round_edges(&mut mesh, &edges, &policy).expect("square rim chamfer");
        assert_clean(&mesh);
        assert_eq!(result.stats.closed_chains, 1);
        assert_eq!(result.stats.strip_faces, 4);
        let expected = if recessed {
            (2.0 + 1.5) * setback * setback + 4.0 / 3.0 * setback.powi(3)
        } else {
            (4.0 + 3.0) * setback * setback - 4.0 / 3.0 * setback.powi(3)
        };
        let removed = before - signed_volume(&mesh);
        assert!(
            (removed - expected).abs() < 1e-6,
            "recessed={recessed}: removed {removed}, expected {expected}"
        );
        // At the top, a miter must keep the full setback on both axes.
        let expected_corner = if recessed {
            [0.8, 0.55, 1.0]
        } else {
            [0.2, 0.2, 1.0]
        };
        assert!(
            mesh.vertices().any(|v| {
                mesh.vertex_position(v)
                    .unwrap()
                    .iter()
                    .zip(expected_corner)
                    .all(|(&a, b)| (a - b).abs() < 1e-6)
            }),
            "missing miter tangency {expected_corner:?}"
        );
    }
}

#[test]
fn square_rim_fillets_follow_both_cylinders_and_keep_miter_creases() {
    let radius = 0.2;
    for recessed in [false, true] {
        let mut mesh = if recessed {
            recessed_panel()
        } else {
            box_mesh(4.0, 3.0, 1.0)
        };
        let edges = square_rim(&mesh, recessed);
        let planes: BTreeMap<_, _> = mesh
            .faces()
            .map(|face| {
                let points: Vec<_> = mesh
                    .face_loop(face)
                    .map(|corner| {
                        promote(
                            *mesh
                                .vertex_position(mesh.to_vertex(corner).unwrap())
                                .unwrap(),
                        )
                    })
                    .collect();
                (face, (normalize(newell(&points)).unwrap(), points[0]))
            })
            .collect();
        let before = signed_volume(&mesh);
        let mut policy = RoundPolicy::fillet(radius);
        policy.segments = Some(16);
        policy.max_tangent_turn = core::f64::consts::FRAC_PI_2;
        let result = round_edges(&mut mesh, &edges, &policy).expect("square rim fillet");
        assert_clean(&mesh);
        assert_eq!(result.stats.closed_chains, 1);
        assert_eq!(result.stats.strip_faces, 64);
        let sources: BTreeMap<_, _> = result.face_provenance.iter().copied().collect();
        let mut creases = BTreeSet::new();
        for &(face, source) in &result.face_provenance {
            let RoundFaceSource::Edge(pair) = source else {
                continue;
            };
            let (normal, anchor) = pair
                .into_iter()
                .map(|f| planes[&f])
                .find(|(n, _)| n[2] == 0.0)
                .unwrap();
            let axis_height = dot(normal, anchor) - radius;
            for corner in mesh.face_loop(face) {
                let point = promote(
                    *mesh
                        .vertex_position(mesh.to_vertex(corner).unwrap())
                        .unwrap(),
                );
                let horizontal = dot(normal, point) - axis_height;
                let vertical = point[2] - (1.0 - radius);
                let distance = (horizontal * horizontal + vertical * vertical).sqrt();
                assert!(
                    (distance - radius).abs() < 5e-7,
                    "recessed={recessed} radius at {point:?}: {distance}"
                );
                let expected = add(
                    scale(normal, horizontal / distance),
                    [0.0, 0.0, vertical / distance],
                );
                let actual = mesh
                    .attrs()
                    .sparse(attr::CORNER_NORMAL_OVERRIDE)
                    .unwrap()
                    .get(corner.as_id())
                    .unwrap();
                assert!(norm(sub(promote(*actual), expected)) < 2e-6);
                let other = mesh.face(mesh.twin(corner).unwrap()).unwrap();
                if matches!(sources.get(&other), Some(RoundFaceSource::Edge(other_pair)) if *other_pair != pair)
                {
                    assert_eq!(mesh.edge_sharpness(corner), Some(1.0));
                    creases.insert(mesh.canonical_edge(corner).unwrap());
                }
            }
        }
        assert_eq!(
            creases.len(),
            64,
            "four miter creases, each split into 16 bands"
        );
        let straight = 2.0
            * (if recessed { 2.0 + 1.5 } else { 4.0 + 3.0 })
            * radius.powi(2)
            * (1.0 - core::f64::consts::PI / 4.0);
        let corners = 4.0 * radius.powi(3) * (5.0 / 3.0 - core::f64::consts::FRAC_PI_2);
        let expected = straight + if recessed { corners } else { -corners };
        let removed = before - signed_volume(&mesh);
        assert!(
            (removed / expected - 1.0).abs() < 0.01,
            "recessed={recessed}: removed {removed}, expected {expected}"
        );
    }
}

#[test]
fn boolean_recess_rim_finishes_and_oversized_offsets_refuse_atomically() {
    let panel = box_mesh(4.0, 3.0, 1.0);
    let mut cutter = box_mesh(2.0, 1.5, 0.8);
    let positions: Vec<_> = cutter
        .vertices()
        .map(|v| {
            let p = *cutter.vertex_position(v).unwrap();
            (v, [p[0] + 1.0, p[1] + 0.75, p[2] + 0.4])
        })
        .collect();
    let mut edit = cutter.edit();
    for (vertex, point) in positions {
        op::set_vertex_position(&mut edit, vertex, point).unwrap();
    }
    let _: () = edit.finish();
    let mut scratch = BooleanScratch::new();
    let mut diagnostics = BooleanDiagnostics::default();
    let input = boolean_mesh(
        &panel,
        &cutter,
        BooleanOp::Difference,
        FaceTriangulation::Robust,
        &mut scratch,
        &mut diagnostics,
    )
    .unwrap()
    .mesh;
    assert_clean(&input);
    let edges = square_rim(&input, true);
    assert!(!edges.is_empty());
    for mut policy in [RoundPolicy::chamfer(0.2), RoundPolicy::fillet(0.2)] {
        policy.max_tangent_turn = core::f64::consts::FRAC_PI_2;
        let mut mesh = input.clone();
        round_edges(&mut mesh, &edges, &policy).expect("Boolean recess rim");
        assert_clean(&mesh);
        assert!(signed_volume(&mesh) < signed_volume(&input));
        let mut repeat = input.clone();
        let mut reversed = edges.clone();
        reversed.reverse();
        round_edges(&mut repeat, &reversed, &policy).unwrap();
        assert_eq!(exact_snapshot(&mesh), exact_snapshot(&repeat));
    }
    for mut policy in [RoundPolicy::chamfer(0.7), RoundPolicy::fillet(0.7)] {
        policy.max_tangent_turn = core::f64::consts::FRAC_PI_2;
        let mut mesh = input.clone();
        assert!(
            round_edges(&mut mesh, &edges, &policy).is_err(),
            "offset crosses the pocket floor"
        );
        assert_eq!(exact_snapshot(&mesh), exact_snapshot(&input));
    }
}

#[test]
fn planar_miters_preserve_profiles_on_sloped_flanks() {
    let mut input = box_mesh(4.0, 4.0, 1.0);
    let edges = square_rim(&input, false);
    let positions: Vec<_> = input
        .vertices()
        .map(|v| {
            let [x, y, z] = *input.vertex_position(v).unwrap();
            let scale = 1.0 + (1.0 - z) * 0.5;
            (v, [(x - 2.0) * scale + 2.0, (y - 2.0) * scale + 2.0, z])
        })
        .collect();
    let mut edit = input.edit();
    for (vertex, point) in positions {
        op::set_vertex_position(&mut edit, vertex, point).unwrap();
    }
    let _: () = edit.finish();
    for mut policy in [RoundPolicy::chamfer(0.2), RoundPolicy::fillet(0.2)] {
        policy.max_tangent_turn = core::f64::consts::FRAC_PI_2;
        policy.segments = Some(8);
        let mut mesh = input.clone();
        let result = round_edges(&mut mesh, &edges, &policy).unwrap();
        assert_clean(&mesh);
        let tangent = match policy.kind {
            RoundKind::Chamfer { setback } => setback,
            RoundKind::Fillet { radius } => radius * (2.0_f64.sqrt() - 1.0),
        };
        let expected = [tangent, tangent, 1.0];
        assert!(
            mesh.vertices().any(|v| {
                norm(sub(promote(*mesh.vertex_position(v).unwrap()), expected)) < 1e-6
            })
        );
        if let RoundKind::Fillet { radius } = policy.kind {
            for (face, source) in result.face_provenance {
                if !matches!(source, RoundFaceSource::Edge(_)) {
                    continue;
                }
                let points: Vec<_> = mesh
                    .face_loop(face)
                    .map(|corner| {
                        promote(
                            *mesh
                                .vertex_position(mesh.to_vertex(corner).unwrap())
                                .unwrap(),
                        )
                    })
                    .collect();
                // The 45-degree flanks put each cylinder axis at a known
                // distance from the top perimeter. One cylinder must contain
                // every corner of the strip, including its miter endpoints.
                assert!([0, 1].into_iter().any(|axis| {
                    [tangent, 4.0 - tangent].into_iter().any(|center| {
                        points.iter().all(|p| {
                            let a = p[axis] - center;
                            let b = p[2] - (1.0 - radius);
                            ((a * a + b * b).sqrt() - radius).abs() < 1e-6
                        })
                    })
                }));
            }
        }
    }
}

#[test]
fn planar_rim_finishing_commutes_with_rigid_placement() {
    // Rotate around an oblique axis so no face remains axis-aligned. Shared
    // flank fragments then acquire slightly different normals in stored f32.
    let rotate = |p: [f64; 3]| {
        let axis = normalize([1.0, 2.0, 3.0]).unwrap();
        let angle = 0.7_f64;
        add(
            add(scale(p, angle.cos()), scale(cross(axis, p), angle.sin())),
            scale(axis, dot(axis, p) * (1.0 - angle.cos())),
        )
    };
    let place = |p| add(rotate(p), [0.3, -0.7, 1.2]);
    for recessed in [false, true] {
        let input = if recessed {
            recessed_panel()
        } else {
            box_mesh(4.0, 3.0, 1.0)
        };
        let edges = square_rim(&input, recessed);
        let mut placed = input.clone();
        let positions: Vec<_> = placed
            .vertices()
            .map(|v| {
                (
                    v,
                    narrow(place(promote(*placed.vertex_position(v).unwrap()))),
                )
            })
            .collect();
        let mut edit = placed.edit();
        for (vertex, point) in positions {
            op::set_vertex_position(&mut edit, vertex, point).unwrap();
        }
        let _: () = edit.finish();
        for mut policy in [RoundPolicy::chamfer(0.2), RoundPolicy::fillet(0.2)] {
            policy.max_tangent_turn = core::f64::consts::FRAC_PI_2;
            policy.segments = Some(8);
            let mut reference = input.clone();
            let mut actual = placed.clone();
            round_edges(&mut reference, &edges, &policy).unwrap();
            round_edges(&mut actual, &edges, &policy).expect("placed planar rim");
            assert_clean(&actual);
            assert_eq!(reference.vertices().count(), actual.vertices().count());
            for vertex in reference.vertices() {
                let expected = place(promote(*reference.vertex_position(vertex).unwrap()));
                assert!(
                    actual.vertices().any(|v| {
                        norm(sub(promote(*actual.vertex_position(v).unwrap()), expected)) < 3e-6
                    }),
                    "missing placed point {expected:?}"
                );
            }
        }
    }
}

#[test]
fn derived_bands_bound_the_miter_curve_chord_error() {
    let mut mesh = box_mesh(4.0, 3.0, 1.0);
    let edges = square_rim(&mesh, false);
    let radius = 0.2;
    let mut policy = RoundPolicy::fillet(radius);
    policy.chord_tolerance = 0.007;
    policy.max_tangent_turn = core::f64::consts::FRAC_PI_2;
    round_edges(&mut mesh, &edges, &policy).unwrap();
    let mut samples = 0;
    for edge in mesh.faces().flat_map(|f| mesh.face_loop(f)) {
        let (a, b) = edge_endpoints(&mesh, edge);
        let on_miter = |p: [f32; 3]| {
            (p[0] - p[1]).abs() < 1e-7 && p[0] >= 0.0 && p[0] <= 0.200001 && p[2] >= 0.799999
        };
        if !on_miter(a) || !on_miter(b) {
            continue;
        }
        let [a, b] = [a, b].map(promote);
        let theta = [a, b].map(|p| (radius - p[0]).atan2(p[2] - (1.0 - radius)));
        let middle = (theta[0] + theta[1]) * 0.5;
        let curve = [
            radius * (1.0 - middle.sin()),
            radius * (1.0 - middle.sin()),
            1.0 - radius + radius * middle.cos(),
        ];
        let deviation = norm(sub(scale(add(a, b), 0.5), curve));
        assert!(
            deviation <= policy.chord_tolerance + 1e-6,
            "miter chord deviation {deviation} exceeds {}",
            policy.chord_tolerance
        );
        samples += 1;
    }
    assert!(samples >= 6);
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
fn cleaned_drilled_rims_round_without_temporary_boundary_pinches() {
    // Seam cleanup changes the order in which replacement faces can attach
    // to the surviving surface. Both rims must still round successfully.
    for angle in [0.0, core::f32::consts::FRAC_PI_4] {
        let mut mesh = drilled_slab_rotated(angle);
        let _cleanup = cleanup_seams(&mut mesh, &SeamCleanupPolicy::default());

        let volume_before = signed_volume(&mesh);
        let stats = round_sharp_edges(&mut mesh, &RoundPolicy::fillet(0.05)).unwrap();
        assert_eq!(stats.closed_chains, 2);
        assert_clean(&mesh);
        assert_eq!(euler_characteristic(&mesh), 0);
        let volume = signed_volume(&mesh);
        assert!(volume > 0.99 * volume_before && volume < volume_before);
        for face in mesh.faces() {
            assert!(
                mesh.face_loop(face)
                    .all(|h| mesh.face(mesh.twin(h).unwrap()) != Some(FaceId::OUTSIDE))
            );
        }
    }
}
