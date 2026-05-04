// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Executable wind-tunnel scenarios for Exedra kernel regression checks.
//!
//! Run the quick profile (the default):
//! `cargo run --release -p exedra_wind_tunnel -- --quick`
//!
//! Run the formal WT-1 stress profile:
//! `cargo run --release -p exedra_wind_tunnel -- --wt1-stress`

use std::hint::black_box;
use std::time::{Duration, Instant};

use exedra::{ExtractParams, Mesh, op};
use exedra_primitives::{GridParams, grid};
use exedra_testkit::trimesh_signature;

fn main() {
    let profile = Profile::from_args(std::env::args().skip(1));
    let scenario = Wt1Scenario::new(profile);
    run_wt1(&scenario);
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Profile {
    Quick,
    Stress,
}

impl Profile {
    fn from_args(args: impl Iterator<Item = String>) -> Self {
        let mut profile = Self::Quick;
        for arg in args {
            match arg.as_str() {
                "--quick" => profile = Self::Quick,
                "--wt1-stress" => profile = Self::Stress,
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                _ => {
                    eprintln!("unknown argument: {arg}");
                    print_help();
                    std::process::exit(2);
                }
            }
        }
        profile
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Stress => "stress",
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct Wt1Scenario {
    profile: Profile,
    width: u32,
    height: u32,
    iterations: usize,
    sharp_period: usize,
}

impl Wt1Scenario {
    const fn new(profile: Profile) -> Self {
        match profile {
            Profile::Quick => Self {
                profile,
                width: 64,
                height: 32,
                iterations: 8,
                sharp_period: 17,
            },
            Profile::Stress => Self {
                profile,
                width: 1_000,
                height: 500,
                iterations: 5,
                sharp_period: 31,
            },
        }
    }

    const fn expected_faces(self) -> u32 {
        self.width * self.height
    }
}

fn run_wt1(scenario: &Wt1Scenario) {
    let build_start = Instant::now();
    let mesh = wt1_triangulation_stress_mesh(scenario);
    let build_elapsed = build_start.elapsed();
    let params = ExtractParams::default();
    let (warm_tri, warm_stats) = mesh.to_trimesh(&params);
    let warm_signature = trimesh_signature(&warm_tri);
    let repeat_signature = trimesh_signature(&mesh.to_trimesh(&params).0);
    assert_eq!(
        warm_signature, repeat_signature,
        "WT-1 extraction signature must be stable across repeated runs"
    );

    let mut total = Duration::ZERO;
    let mut best = Duration::MAX;
    let mut checksum = 0_u64;
    for _ in 0..scenario.iterations {
        let start = Instant::now();
        let (tri, stats) = black_box(mesh.to_trimesh(&params));
        let elapsed = start.elapsed();
        best = best.min(elapsed);
        total += elapsed;
        checksum = checksum.wrapping_add(
            trimesh_signature(&tri)
                .wrapping_add(stats.triangle_count)
                .wrapping_add(stats.render_vertex_count),
        );
    }

    let avg = total / u32::try_from(scenario.iterations).expect("iterations fit in u32");
    println!(
        "scenario=WT-1 profile={} mesh_faces={} expected_faces={} mesh_vertices={} triangles={} render_vertices={} splits={} uv_splits={} normal_splits={} build_ns={} best_ns={} avg_ns={} signature={:016x} checksum={:016x}",
        scenario.profile.label(),
        mesh.faces().count(),
        scenario.expected_faces(),
        mesh.vertices().count(),
        warm_stats.triangle_count,
        warm_stats.render_vertex_count,
        warm_stats.split_count,
        warm_stats.uv_split_count,
        warm_stats.normal_split_count,
        build_elapsed.as_nanos(),
        best.as_nanos(),
        avg.as_nanos(),
        warm_signature,
        checksum,
    );
}

fn wt1_triangulation_stress_mesh(scenario: &Wt1Scenario) -> Mesh {
    let mut mesh = grid(&GridParams {
        size: [scenario.width as f32, scenario.height as f32],
        segments: [scenario.width, scenario.height],
        centered: false,
    })
    .mesh;

    let mut corner_writes = Vec::with_capacity(mesh.faces().count() * 4);
    let mut sharp_edges = Vec::new();
    for (face_index, face) in mesh.faces().enumerate() {
        for (corner_index, corner) in mesh.face_loop(face).enumerate() {
            corner_writes.push((
                corner,
                [
                    (face_index % scenario.width as usize) as f32,
                    corner_index as f32,
                ],
            ));
            if (face_index + corner_index) % scenario.sharp_period == 0 {
                sharp_edges.push(corner);
            }
        }
    }

    let mut edit = mesh.edit();
    for (corner, uv) in corner_writes {
        op::set_corner_uv(&mut edit, corner, uv).expect("WT-1 corner UV write should succeed");
    }
    for edge in sharp_edges {
        op::set_edge_sharpness(&mut edit, edge, 2.0)
            .expect("WT-1 edge sharpness write should succeed");
    }
    let _: () = edit.finish();
    mesh
}

fn print_help() {
    println!(
        "exedra_wind_tunnel\n\n  --quick       run a small deterministic WT-1 profile (default)\n  --wt1-stress  run the formal WT-1 profile (500k faces)\n"
    );
}

#[cfg(test)]
mod tests {
    use super::{Profile, Wt1Scenario, wt1_triangulation_stress_mesh};

    #[test]
    fn wt1_quick_mesh_matches_expected_counts_and_validates() {
        let scenario = Wt1Scenario::new(Profile::Quick);
        let mesh = wt1_triangulation_stress_mesh(&scenario);
        assert_eq!(mesh.faces().count(), scenario.expected_faces() as usize);
        assert!(mesh.vertices().count() > 0);
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn wt1_profiles_are_stable_contracts() {
        assert_eq!(Wt1Scenario::new(Profile::Stress).expected_faces(), 500_000);
        assert_eq!(Wt1Scenario::new(Profile::Quick).expected_faces(), 2_048);
    }
}
