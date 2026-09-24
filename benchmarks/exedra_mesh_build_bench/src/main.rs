// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Scaling benchmark for half-edge mesh construction from many open patches.
//!
//! Foliage-style inputs merge tens of thousands of disconnected quads into one
//! mesh, so every edge is an open boundary edge. Boundary stitching and
//! position welding must stay near-linear in that edge and vertex count.
//!
//! Run with:
//! `cargo run --release -p exedra_mesh_build_bench`
//!
//! Pass face counts to override the default `10000 100000` sweep, e.g.
//! `cargo run --release -p exedra_mesh_build_bench -- 1000 10000`.

use std::hint::black_box;
use std::time::{Duration, Instant};

use exedra_mesh::{BuildParams, Mesh, MeshBuilder};

fn main() {
    let mut counts = std::env::args()
        .skip(1)
        .map(|arg| {
            arg.parse::<u32>().unwrap_or_else(|_| {
                eprintln!("face counts must be positive integers, got {arg}");
                std::process::exit(2);
            })
        })
        .collect::<Vec<_>>();
    if counts.is_empty() {
        counts = vec![10_000, 100_000];
    }

    println!("scenario                         faces  boundary_edges      time");
    for faces in counts {
        let quads = faces / 2;
        let leaves = Leaves::new(quads);
        let boundary_edges = quads as usize * 4;

        report("builder_quads", quads as usize, boundary_edges, || {
            let mut builder = MeshBuilder::new();
            for position in &leaves.positions {
                builder.push_vertex(*position);
            }
            for quad in &leaves.quads {
                builder.add_face(quad).expect("leaf quad is valid");
            }
            builder.build().expect("leaf quads build").mesh
        });
        report("indexed_triangles", faces as usize, boundary_edges, || {
            Mesh::from_indexed_triangles(
                &leaves.positions,
                &leaves.triangles,
                &BuildParams::default(),
            )
            .expect("leaf triangles build")
        });
        report(
            "indexed_triangles_welded",
            faces as usize,
            boundary_edges,
            || {
                Mesh::from_indexed_triangles(
                    &leaves.unwelded_positions,
                    &leaves.unwelded_triangles,
                    &BuildParams {
                        weld_tolerance: Some(1.0e-5),
                    },
                )
                .expect("welded leaf triangles build")
            },
        );
    }
}

fn report(label: &str, faces: usize, boundary_edges: usize, build: impl Fn() -> Mesh) {
    let start = Instant::now();
    let mesh = black_box(build());
    let elapsed = start.elapsed();
    assert_eq!(mesh.faces().count(), faces, "{label} face count");
    println!(
        "{label:<28} {faces:>9} {boundary_edges:>15} {:>9}",
        format_duration(elapsed)
    );
}

fn format_duration(duration: Duration) -> String {
    let millis = duration.as_secs_f64() * 1.0e3;
    if millis >= 1.0e3 {
        format!("{:.2}s", millis / 1.0e3)
    } else {
        format!("{millis:.2}ms")
    }
}

/// Disconnected leaf cards scattered on a jittered lattice.
struct Leaves {
    positions: Vec<[f32; 3]>,
    quads: Vec<[u32; 4]>,
    triangles: Vec<[u32; 3]>,
    /// One position per triangle corner, so welding restores the shared quad
    /// diagonal.
    unwelded_positions: Vec<[f32; 3]>,
    unwelded_triangles: Vec<[u32; 3]>,
}

impl Leaves {
    fn new(count: u32) -> Self {
        let side = (1..).find(|side: &u32| side.pow(3) >= count).unwrap_or(1);
        let mut positions = Vec::with_capacity(count as usize * 4);
        let mut quads = Vec::with_capacity(count as usize);
        let mut triangles = Vec::with_capacity(count as usize * 2);
        let mut state = 0x9e37_79b9_u32;
        for leaf in 0..count {
            let cell = [leaf % side, (leaf / side) % side, leaf / (side * side)];
            let origin = cell.map(|axis| axis as f32 * 1.5);
            let tilt = next_unit(&mut state) * 0.4;
            let base = leaf * 4;
            positions.push(origin);
            positions.push([origin[0] + 1.0, origin[1], origin[2] + tilt]);
            positions.push([origin[0] + 1.0, origin[1] + 1.0, origin[2] + tilt]);
            positions.push([origin[0], origin[1] + 1.0, origin[2]]);
            quads.push([base, base + 1, base + 2, base + 3]);
            triangles.push([base, base + 1, base + 2]);
            triangles.push([base, base + 2, base + 3]);
        }
        let mut unwelded_positions = Vec::with_capacity(triangles.len() * 3);
        let mut unwelded_triangles = Vec::with_capacity(triangles.len());
        for triangle in &triangles {
            let base = u32::try_from(unwelded_positions.len()).expect("benchmark fits u32");
            unwelded_positions.extend(triangle.map(|index| positions[index as usize]));
            unwelded_triangles.push([base, base + 1, base + 2]);
        }
        Self {
            positions,
            quads,
            triangles,
            unwelded_positions,
            unwelded_triangles,
        }
    }
}

fn next_unit(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 8) as f32 / (1_u32 << 24) as f32
}
