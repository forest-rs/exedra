// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Simple executable benchmark for representative `Mesh::to_trimesh` workloads.
//!
//! Run with:
//! `cargo run --release -p exedra_render_bench`

use std::hint::black_box;
use std::time::{Duration, Instant};

use exedra::{ExtractParams, Mesh, NormalsSource, op};
use exedra_fidget::VmField;
use exedra_isosurface::{DualContourParams, EdgeSearchParams, dual_contour};
use exedra_primitives::{TorusParams, UvSphereParams, torus, uv_sphere};
use exedra_qef::QefParams;
use exedra_spatial::Aabb;
use fidget::{context::Tree, vm::VmShape};

fn main() {
    let scenarios = [
        Scenario {
            label: "torus_smooth",
            mesh: smooth_torus_mesh(),
            params: ExtractParams::default(),
            iterations: 40,
        },
        Scenario {
            label: "uv_sphere_face_uv_splits",
            mesh: uv_split_sphere_mesh(),
            params: ExtractParams {
                normals: NormalsSource::Derived,
                ..ExtractParams::default()
            },
            iterations: 20,
        },
        Scenario {
            label: "implicit_toothed_torus",
            mesh: implicit_toothed_torus_mesh(),
            params: ExtractParams {
                normals: NormalsSource::CustomOrDerived,
                ..ExtractParams::default()
            },
            iterations: 3,
        },
    ];

    for scenario in scenarios {
        run_scenario(&scenario);
    }
}

struct Scenario {
    label: &'static str,
    mesh: Mesh,
    params: ExtractParams,
    iterations: usize,
}

fn run_scenario(scenario: &Scenario) {
    let (warm_tri, warm_stats) = scenario.mesh.to_trimesh(&scenario.params);
    let face_count = scenario.mesh.faces().count();
    let vertex_count = scenario.mesh.vertices().count();
    let mut elapsed_total = Duration::ZERO;
    let mut best = Duration::MAX;
    let mut checksum = 0_u64;

    for _ in 0..scenario.iterations {
        let start = Instant::now();
        let (tri, stats) = black_box(scenario.mesh.to_trimesh(&scenario.params));
        let elapsed = start.elapsed();
        elapsed_total += elapsed;
        best = best.min(elapsed);
        checksum = checksum
            .wrapping_add(tri.indices.len() as u64)
            .wrapping_add(stats.render_vertex_count);
    }

    let avg = elapsed_total / u32::try_from(scenario.iterations).expect("iterations fit in u32");
    println!(
        "{}: mesh_faces={} mesh_vertices={} tris={} render_vertices={} splits={} uv_splits={} normal_splits={} best={:?} avg={:?} checksum={}",
        scenario.label,
        face_count,
        vertex_count,
        warm_stats.triangle_count,
        warm_stats.render_vertex_count,
        warm_stats.split_count,
        warm_stats.uv_split_count,
        warm_stats.normal_split_count,
        best,
        avg,
        checksum ^ warm_tri.positions.len() as u64,
    );
}

fn smooth_torus_mesh() -> Mesh {
    torus(&TorusParams {
        major_radius: 1.0,
        minor_radius: 0.32,
        major_segments: 96,
        minor_segments: 48,
    })
    .mesh
}

fn uv_split_sphere_mesh() -> Mesh {
    let mut mesh = uv_sphere(&UvSphereParams {
        radius: 1.0,
        lat_segments: 48,
        lon_segments: 96,
        centered: true,
    })
    .mesh;
    let writes = mesh
        .faces()
        .enumerate()
        .flat_map(|(face_index, face)| {
            mesh.face_loop(face)
                .enumerate()
                .map(move |(corner_index, corner)| {
                    (corner, [face_index as f32, corner_index as f32])
                })
        })
        .collect::<Vec<_>>();
    let mut session = mesh.edit();
    for (corner, uv) in writes {
        op::set_corner_uv(&mut session, corner, uv).expect("corner UV write should succeed");
    }
    let _: () = session.finish();
    mesh
}

fn implicit_toothed_torus_mesh() -> Mesh {
    let params = DualContourParams {
        root_bounds: Aabb::new([-1.15, -1.15, -0.55], [1.15, 1.15, 0.55]).expect("valid bounds"),
        max_depth: 6,
        cell_budget: None,
        edge_search: EdgeSearchParams {
            bisection_steps: 10,
        },
        qef: QefParams::default(),
    };

    let x = Tree::x();
    let y = Tree::y();
    let z = Tree::z();
    let ring_radius = 0.68;
    let tube_base = 0.19;
    let theta = y.atan2(x.clone());
    let radial = (x.square() + y.square()).sqrt();
    let tube_angle = z.atan2(radial.clone() - ring_radius);
    let teeth = (theta * 12.0).cos() * 0.055;
    let fluting = (tube_angle * 3.0).cos() * 0.025;
    let tube_radius = tube_base + teeth + fluting;
    let field = VmField::new(VmShape::from(
        ((radial - ring_radius).square() + z.square()).sqrt() - tube_radius,
    ))
    .expect("shape should be axis-only");

    dual_contour(&field, &params)
        .expect("implicit toothed torus extraction should succeed")
        .mesh
}

#[cfg(test)]
mod tests {
    use super::{implicit_toothed_torus_mesh, smooth_torus_mesh, uv_split_sphere_mesh};

    #[test]
    fn scenarios_build_non_empty_meshes() {
        let meshes = [
            smooth_torus_mesh(),
            uv_split_sphere_mesh(),
            implicit_toothed_torus_mesh(),
        ];
        for mesh in meshes {
            assert!(mesh.faces().count() > 0);
            assert!(mesh.vertices().count() > 0);
            assert!(mesh.validate_deep().is_empty());
        }
    }
}
