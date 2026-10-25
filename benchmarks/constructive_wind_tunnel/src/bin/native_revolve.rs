// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Matched native/mesh revolution construction, realization and chart costs.
//! `cargo run --release -p constructive_wind_tunnel --bin native_revolve -- 9`
//! Set `EXEDRA_REVOLVE_PROFILE` to native, native-uv, mesh or mesh-uv for a paused
//! three-call allocation batch, after one warm call. No allocator code is added.

use exedra_analytic::realize::RealizationPolicy;
use exedra_analytic::revolve::{FaceRole, Revolution};
use exedra_constructive::chart::{ChartTransform, SurfaceChart};
use exedra_constructive::ir::{CapMode, Placement3};
use exedra_constructive::profile::{Loop2, Profile2, Seg2};
use exedra_constructive::tessellate::{
    EvalPolicy, tessellate_revolve, tessellate_revolve_with_chart,
};
use exedra_profile::ProfilePolicy;
use std::error::Error;
use std::hint::black_box;
use std::time::Instant;

fn profile() -> Profile2 {
    Profile2::new(
        Loop2::new(
            [
                [22.0, 0.0],
                [22.0, 5.0],
                [18.0, 5.0],
                [18.0, 10.0],
                [20.0, 10.0],
                [20.0, 13.0],
                [18.0, 13.0],
                [18.0, 30.0],
                [14.0, 38.0],
                [14.0, 44.0],
                [11.0, 47.0],
                [7.0, 47.0],
                [7.0, 26.0],
                [10.0, 19.0],
                [10.0, 8.0],
                [7.0, 3.0],
                [7.0, 0.0],
            ]
            .into_iter()
            .map(|p| Seg2::line((p[0], p[1])))
            .collect(),
        )
        .unwrap(),
        vec![],
    )
    .unwrap()
}
fn chart() -> SurfaceChart {
    SurfaceChart::Revolve {
        reference_radius: 18.0,
        wall: ChartTransform {
            matrix: [[1.0 / 12.0, 0.0], [0.0, 1.0 / 12.0]],
            offset: [0.0; 2],
        },
        caps: ChartTransform::IDENTITY,
    }
}
fn time<T>(mut run: impl FnMut() -> Result<T, Box<dyn Error>>) -> Result<f64, Box<dyn Error>> {
    let start = Instant::now();
    for _ in 0..3 {
        black_box(run()?);
    }
    Ok(start.elapsed().as_secs_f64() * 1e6 / 3.0)
}
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_unstable_by(f64::total_cmp);
    v[v.len() / 2]
}
fn main() -> Result<(), Box<dyn Error>> {
    let profile = profile();
    let shared = profile.to_line_arc(&ProfilePolicy::new(1e-6))?;
    let chart = chart();
    if let Ok(mode) = std::env::var("EXEDRA_REVOLVE_PROFILE") {
        return allocation_profile(&mode, &profile, &shared, chart);
    }
    let reps: u32 = std::env::args().nth(1).map_or(Ok(9), |s| s.parse())?;
    if reps == 0 {
        return Err("repetitions must be positive".into());
    }
    println!("sweep rings triangles construct_us native_us mesh_us native_chart_us mesh_chart_us");
    for sweep in [core::f64::consts::TAU, 1.5 * core::f64::consts::PI] {
        for n in [24, 64, 128] {
            let sin = (sweep / (4.0 * f64::from(n))).sin();
            let tolerance = 2.0 * 22.0 * sin * sin * 1.01;
            let native_policy = RealizationPolicy {
                tolerance,
                min_arc_segments: n,
                max_edge_segments: n,
                ..RealizationPolicy::default()
            };
            let mut mesh_policy = EvalPolicy::default();
            mesh_policy.discretize.chord_tolerance = tolerance;
            mesh_policy.discretize.min_arc_edges = n;
            mesh_policy.discretize.max_segment_edges = n;
            let solid = Revolution::new(shared.clone(), Placement3::IDENTITY, sweep)?;
            let native = solid.realize(&native_policy)?;
            let mesh = tessellate_revolve(
                &profile,
                &Placement3::IDENTITY,
                sweep,
                CapMode::Both,
                &mesh_policy,
            )?;
            assert!(
                native.mesh.validate_deep().is_empty() && mesh.mesh.validate_deep().is_empty(),
                "both backends must produce valid meshes"
            );
            assert!(
                native.mesh.boundary_loops()?.is_empty() && mesh.mesh.boundary_loops()?.is_empty(),
                "both backends must produce closed meshes"
            );
            assert!(
                native.report.within_tolerance(),
                "native error evidence must meet policy"
            );
            let triangles = mesh
                .mesh
                .faces()
                .map(|f| mesh.mesh.face_loop(f).count() - 2)
                .sum::<usize>();
            assert_eq!(
                native.mesh_faces.len(),
                triangles,
                "comparison requires equal angular sampling"
            );
            let mut times: [Vec<f64>; 5] = core::array::from_fn(|_| Vec::new());
            for iteration in 0..reps + 2 {
                // Rotate the execution order across repetitions.
                for offset in 0..5 {
                    let index = (offset + iteration as usize) % 5;
                    let us = match index {
                        0 => time(|| {
                            Ok(Revolution::new(
                                black_box(shared.clone()),
                                Placement3::IDENTITY,
                                sweep,
                            )?)
                        })?,
                        1 => time(|| Ok(solid.realize(black_box(&native_policy))?))?,
                        2 => time(|| {
                            Ok(tessellate_revolve(
                                &profile,
                                &Placement3::IDENTITY,
                                sweep,
                                CapMode::Both,
                                black_box(&mesh_policy),
                            )?)
                        })?,
                        3 => time(|| {
                            Ok(solid.realize_with_uvs(
                                &native_policy,
                                |_, _| 0,
                                |role, p| match role {
                                    FaceRole::Wall { .. } => chart.map_revolve_wall_triangle(p),
                                    FaceRole::Cap { .. } => chart.map_cap_triangle(p),
                                },
                            )?)
                        })?,
                        _ => time(|| {
                            Ok(tessellate_revolve_with_chart(
                                &profile,
                                &Placement3::IDENTITY,
                                sweep,
                                CapMode::Both,
                                chart,
                                black_box(&mesh_policy),
                            )?)
                        })?,
                    };
                    if iteration >= 2 {
                        times[index].push(us);
                    }
                }
            }
            let medians = times.map(median);
            println!(
                "{sweep:.6} {n} {triangles} {:.3} {:.3} {:.3} {:.3} {:.3}",
                medians[0], medians[1], medians[2], medians[3], medians[4]
            );
        }
    }
    Ok(())
}
fn allocation_profile(
    mode: &str,
    profile: &Profile2,
    shared: &exedra_profile::Profile,
    chart: SurfaceChart,
) -> Result<(), Box<dyn Error>> {
    use std::io::{self, Write};
    let n = 64;
    let sweep = core::f64::consts::TAU;
    let native_policy = RealizationPolicy {
        tolerance: 0.03,
        min_arc_segments: n,
        max_edge_segments: n,
        ..RealizationPolicy::default()
    };
    let mut mesh_policy = EvalPolicy::default();
    mesh_policy.discretize.chord_tolerance = 0.03;
    mesh_policy.discretize.min_arc_edges = n;
    mesh_policy.discretize.max_segment_edges = n;
    let solid = Revolution::new(shared.clone(), Placement3::IDENTITY, sweep)?;
    let run = || -> Result<(), Box<dyn Error>> {
        match mode {
            "native" => {
                black_box(solid.realize(&native_policy)?);
            }
            "native-uv" => {
                black_box(solid.realize_with_uvs(
                    &native_policy,
                    |_, _| 0,
                    |role, p| match role {
                        FaceRole::Wall { .. } => chart.map_revolve_wall_triangle(p),
                        FaceRole::Cap { .. } => chart.map_cap_triangle(p),
                    },
                )?);
            }
            "mesh" => {
                black_box(tessellate_revolve(
                    profile,
                    &Placement3::IDENTITY,
                    sweep,
                    CapMode::Both,
                    &mesh_policy,
                )?);
            }
            "mesh-uv" => {
                black_box(tessellate_revolve_with_chart(
                    profile,
                    &Placement3::IDENTITY,
                    sweep,
                    CapMode::Both,
                    chart,
                    &mesh_policy,
                )?);
            }
            _ => return Err("mode must be native, native-uv, mesh or mesh-uv".into()),
        }
        Ok(())
    };
    run()?;
    flush_stack_log();
    println!(
        "profile_ready pid={} mode={mode}; Enter to realize three times",
        std::process::id()
    );
    io::stdout().flush()?;
    io::stdin().read_line(&mut String::new())?;
    for _ in 0..3 {
        run()?;
    }
    flush_stack_log();
    println!("profile_done; Enter to exit");
    io::stdout().flush()?;
    io::stdin().read_line(&mut String::new())?;
    Ok(())
}
fn flush_stack_log() {
    black_box(
        (0..8192)
            .map(|i| Vec::<u8>::with_capacity(32 + i % 16))
            .collect::<Vec<_>>(),
    );
}
