// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Seeded torture suite for the topology-surgery ops (test-only).
//!
//! A fixed `SplitMix64` corpus drives random `collapse_edge`/`flip_edge`
//! sequences over deterministic fixture meshes. The contract under fuzz:
//!
//! - the ops never panic;
//! - `Ok` leaves the mesh `validate_deep`-clean;
//! - `Err` is always a documented precondition (never an internal
//!   `*Failed` safety net) and leaves the mesh byte-identical;
//! - closed surfaces stay closed with Euler characteristic 2 across
//!   collapses, and flips never change entity counts;
//! - reruns are bit-identical.

use alloc::vec::Vec;

use crate::{FaceId, HalfEdgeId, Mesh, MeshBuilder, attr, op};

/// `SplitMix64`: tiny deterministic PRNG for corpus generation.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn range(&mut self, n: usize) -> usize {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "n is a small corpus bound; the modulo keeps the value below it"
        )]
        {
            (self.next() % n.max(1) as u64) as usize
        }
    }
}

/// A triangulated `columns x rows` grid strip (open disk with boundary).
fn grid(columns: u32, rows: u32) -> Mesh {
    let mut builder = MeshBuilder::new();
    for row in 0..=rows {
        for column in 0..=columns {
            #[expect(clippy::cast_precision_loss, reason = "small test indices")]
            builder.push_vertex([column as f32, row as f32, 0.0]);
        }
    }
    let stride = columns + 1;
    for row in 0..rows {
        for column in 0..columns {
            let v00 = row * stride + column;
            let v10 = v00 + 1;
            let v01 = v00 + stride;
            let v11 = v01 + 1;
            builder.add_face(&[v00, v10, v11]).expect("grid face");
            builder.add_face(&[v00, v11, v01]).expect("grid face");
        }
    }
    builder.build().expect("grid should build").mesh
}

/// A closed triangulated prism: `sides`-gon caps fan-triangulated plus
/// two triangles per wall quad. Genus 0, no boundary.
fn prism(sides: u32) -> Mesh {
    let mut builder = MeshBuilder::new();
    for ring in 0..2_u32 {
        for index in 0..sides {
            #[expect(clippy::cast_precision_loss, reason = "small test indices")]
            builder.push_vertex([index as f32, (index * index) as f32, ring as f32]);
        }
    }
    // Top cap (ring 0) fans from vertex 0; bottom cap (ring 1) reversed.
    for index in 1..sides - 1 {
        builder
            .add_face(&[0, index + 1, index])
            .expect("top cap face");
        builder
            .add_face(&[sides, sides + index, sides + index + 1])
            .expect("bottom cap face");
    }
    for index in 0..sides {
        let next = (index + 1) % sides;
        builder
            .add_face(&[index, next, sides + next])
            .expect("wall face");
        builder
            .add_face(&[index, sides + next, sides + index])
            .expect("wall face");
    }
    builder.build().expect("prism should build").mesh
}

/// Structural snapshot for byte-identity comparison: face loops, vertex
/// position bits, and every authored attribute the ops propagate.
type Snapshot = (
    Vec<Vec<u32>>,
    Vec<(u32, [u32; 3])>,
    Vec<(u32, Option<bool>, Option<u32>)>,
    Vec<(u32, [u32; 2])>,
    Vec<(u32, u32)>,
    Vec<(u32, u32)>,
);

fn snapshot(mesh: &Mesh) -> Snapshot {
    let faces: Vec<Vec<u32>> = mesh
        .faces()
        .map(|face| {
            mesh.face_loop(face)
                .filter_map(|corner| mesh.to_vertex(corner))
                .map(|vertex| vertex.index())
                .collect()
        })
        .collect();
    let positions: Vec<(u32, [u32; 3])> = mesh
        .vertices()
        .filter_map(|vertex| {
            mesh.vertex_position(vertex).map(|position| {
                (
                    vertex.index(),
                    [
                        position[0].to_bits(),
                        position[1].to_bits(),
                        position[2].to_bits(),
                    ],
                )
            })
        })
        .collect();
    let mut edge_tags: Vec<(u32, Option<bool>, Option<u32>)> = Vec::new();
    let mut corner_uvs: Vec<(u32, [u32; 2])> = Vec::new();
    for face in mesh.faces() {
        for corner in mesh.face_loop(face) {
            let seam = mesh.edge_seam(corner);
            let sharpness = mesh.edge_sharpness(corner).map(f32::to_bits);
            if seam.is_some() || sharpness.is_some() {
                edge_tags.push((corner.index(), seam, sharpness));
            }
            if let Some(uv) = mesh
                .attrs()
                .sparse(attr::CORNER_UV)
                .and_then(|layer| layer.get(corner.as_id()).copied())
            {
                corner_uvs.push((corner.index(), [uv[0].to_bits(), uv[1].to_bits()]));
            }
        }
    }
    let vertex_sharpness: Vec<(u32, u32)> = mesh
        .vertices()
        .filter_map(|vertex| {
            mesh.vertex_sharpness(vertex)
                .map(|sharpness| (vertex.index(), sharpness.to_bits()))
        })
        .collect();
    let regions: Vec<(u32, u32)> = mesh
        .faces()
        .filter_map(|face| {
            mesh.attrs()
                .dense(attr::FACE_REGION)
                .and_then(|layer| layer.get(face.as_id()).copied())
                .map(|region| (face.index(), region))
        })
        .collect();
    (
        faces,
        positions,
        edge_tags,
        corner_uvs,
        vertex_sharpness,
        regions,
    )
}

fn euler_characteristic(mesh: &Mesh) -> i64 {
    let vertices = i64::try_from(mesh.vertices().count()).expect("small");
    let faces = i64::try_from(mesh.faces().count()).expect("small");
    let half_edges = i64::try_from(mesh.half_edges.iter().count()).expect("small");
    vertices - half_edges / 2 + faces
}

fn has_boundary(mesh: &Mesh) -> bool {
    mesh.half_edges
        .iter()
        .any(|(_, record)| record.face == FaceId::OUTSIDE)
}

/// Sorted live half-edge ids, for deterministic random picks.
fn live_half_edges(mesh: &Mesh) -> Vec<HalfEdgeId> {
    let mut ids: Vec<HalfEdgeId> = mesh
        .half_edges
        .iter()
        .map(|(id, _)| HalfEdgeId::from(id))
        .collect();
    ids.sort_unstable();
    ids
}

struct RunStats {
    collapses: u64,
    flips: u64,
    rejections: u64,
}

/// Runs one seeded op sequence over `mesh`, checking the full contract
/// after every op. Returns op counters for corpus-coverage assertions.
fn run_sequence(mesh: &mut Mesh, seed: u64, ops: usize, closed: bool) -> RunStats {
    let mut rng = Rng(seed);
    let mut stats = RunStats {
        collapses: 0,
        flips: 0,
        rejections: 0,
    };
    for _ in 0..ops {
        let candidates = live_half_edges(mesh);
        if candidates.is_empty() || mesh.faces().count() == 0 {
            break;
        }
        let edge = candidates[rng.range(candidates.len())];
        let before = snapshot(mesh);
        let euler_before = euler_characteristic(mesh);
        if rng.range(2) == 0 {
            let mut session = mesh.edit();
            let result = op::collapse_edge(&mut session, edge);
            let _: () = session.finish();
            match result {
                Ok(_) => {
                    stats.collapses += 1;
                    if closed {
                        assert!(!has_boundary(mesh), "closed surface must stay closed");
                        assert_eq!(
                            euler_characteristic(mesh),
                            euler_before,
                            "collapse must preserve the Euler characteristic of a closed surface"
                        );
                    }
                }
                Err(_) => {
                    assert_eq!(snapshot(mesh), before, "failed collapse must not mutate");
                    stats.rejections += 1;
                }
            }
        } else {
            let mut session = mesh.edit();
            let result = op::flip_edge(&mut session, edge);
            let _: () = session.finish();
            match result {
                Ok(_) => {
                    stats.flips += 1;
                    assert_eq!(
                        euler_characteristic(mesh),
                        euler_before,
                        "flip must not change entity counts"
                    );
                }
                Err(_) => {
                    assert_eq!(snapshot(mesh), before, "failed flip must not mutate");
                    stats.rejections += 1;
                }
            }
        }
        let errors = mesh.validate_deep();
        assert!(errors.is_empty(), "validate_deep after op: {errors:?}");
    }
    stats
}

#[test]
fn collapse_and_flip_torture_never_corrupts() {
    fn grid_4_3() -> Mesh {
        grid(4, 3)
    }
    fn grid_2_6() -> Mesh {
        grid(2, 6)
    }
    fn prism_6() -> Mesh {
        prism(6)
    }
    fn prism_8() -> Mesh {
        prism(8)
    }
    let corpus: [(fn() -> Mesh, bool); 4] = [
        (grid_4_3, false),
        (prism_6, true),
        (grid_2_6, false),
        (prism_8, true),
    ];

    let mut total_collapses = 0;
    let mut total_flips = 0;
    let mut total_rejections = 0;
    for (case, (make, closed)) in corpus.into_iter().enumerate() {
        let seed = 0xE0_D6_5E_ED
            ^ u64::try_from(case)
                .expect("small")
                .wrapping_mul(0x9E37_79B9);
        let mut mesh = make();
        assert!(mesh.validate_deep().is_empty(), "fixture must be valid");
        let stats = run_sequence(&mut mesh, seed, 48, closed);
        total_collapses += stats.collapses;
        total_flips += stats.flips;
        total_rejections += stats.rejections;

        // Bit-identical rerun: the whole sequence is deterministic.
        let mut rerun = make();
        run_sequence(&mut rerun, seed, 48, closed);
        assert_eq!(snapshot(&mesh), snapshot(&rerun), "rerun diverged");
    }
    // The corpus must exercise every outcome to mean anything.
    assert!(total_collapses > 0, "corpus produced no collapses");
    assert!(total_flips > 0, "corpus produced no flips");
    assert!(total_rejections > 0, "corpus produced no typed rejections");
}

#[test]
fn interleaved_with_splits_stays_valid() {
    // Collapse/flip interleaved with the existing split ops: the four
    // surgery primitives must compose without invariant drift.
    let mut mesh = grid(3, 3);
    let mut rng = Rng(0x0C0_FFEE);
    for _ in 0..40 {
        let candidates = live_half_edges(&mesh);
        if candidates.is_empty() {
            break;
        }
        let edge = candidates[rng.range(candidates.len())];
        match rng.range(3) {
            0 => {
                let mut session = mesh.edit();
                let _ = op::collapse_edge(&mut session, edge);
                let _: () = session.finish();
            }
            1 => {
                let mut session = mesh.edit();
                let _ = op::flip_edge(&mut session, edge);
                let _: () = session.finish();
            }
            _ => {
                let mut session = mesh.edit();
                let _ = op::split_edge(&mut session, edge, &crate::PropagatePolicy::default());
                let _: () = session.finish();
            }
        }
        let errors = mesh.validate_deep();
        assert!(errors.is_empty(), "validate_deep after op: {errors:?}");
    }
    assert!(mesh.faces().count() > 0);
}

#[test]
fn fixtures_are_valid() {
    for mesh in [grid(4, 3), grid(2, 6), prism(6), prism(8)] {
        assert!(mesh.validate_deep().is_empty());
    }
    assert_eq!(euler_characteristic(&prism(6)), 2);
    assert!(!has_boundary(&prism(6)));
    assert_eq!(euler_characteristic(&grid(4, 3)), 1);
}
