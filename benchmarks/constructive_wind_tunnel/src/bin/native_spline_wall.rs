// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Spline-wall construction, regularity proof and accuracy-driven realization.
#[path = "../spline_wall_fixture.rs"]
mod fixture;
use exedra_analytic::{body::Body, geometry::Surface, realize::RealizationPolicy};
use exedra_math::Placement3;
use std::{error::Error, hint::black_box, path::Path, time::Instant};

fn measure<T>(repetitions: usize, mut action: impl FnMut() -> T) -> f64 {
    for _ in 0..3 {
        black_box(action());
    }
    let mut times: Vec<_> = (0..repetitions)
        .map(|_| {
            let start = Instant::now();
            black_box(action());
            start.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    times.sort_unstable_by(f64::total_cmp);
    times[times.len() / 2]
}
fn wall(body: &Body) -> &exedra_analytic::geometry::Ruled {
    let Surface::Ruled(wall) = &body.faces()[2].surface else {
        unreachable!("fixture wall")
    };
    wall
}
fn policy() -> RealizationPolicy {
    RealizationPolicy {
        min_arc_segments: std::env::var("EXEDRA_SPLINE_MIN_SEGMENTS")
            .map_or(4, |s| s.parse().expect("minimum segments")),
        quality: std::env::var("EXEDRA_SPLINE_BOUNDARY_SPACING")
            .ok()
            .map(|s| {
                exedra_analytic::realize::TriangleQuality::new(s.parse().expect("boundary spacing"))
                    .with_max_grid_triangle_tests(
                        std::env::var("EXEDRA_SPLINE_GRID_TRIANGLE_TESTS")
                            .map_or(1 << 20, |s| s.parse().expect("grid triangle-test budget")),
                    )
            }),
        ..Default::default()
    }
}
/// One warmed native workload for the isolated heap profiler. Source geometry,
/// warmup and the measurement/checkpoint machinery are outside the window.
pub fn allocation_window(mut checkpoint: impl FnMut(&str)) {
    let phase = std::env::var("EXEDRA_SPLINE_PHASE").unwrap_or_else(|_| "proof".into());
    let body = fixture::body(true, 1.0);
    let curves = wall(&body).curves();
    let policy = policy();
    if phase == "realize" {
        drop(body.realize(Placement3::IDENTITY, &policy).unwrap());
    }
    if phase == "proof" {
        drop(Surface::ruled(curves[0].clone(), curves[1].clone()).unwrap());
    }
    checkpoint("ready");
    match phase.as_str() {
        "construct" => {
            let out = black_box(fixture::body(true, 1.0));
            checkpoint("retained");
            drop(out);
        }
        "proof" => {
            let out = black_box(Surface::ruled(curves[0].clone(), curves[1].clone()).unwrap());
            checkpoint("retained");
            drop(out);
        }
        "realize" => {
            let out = black_box(body.realize(Placement3::IDENTITY, &policy).unwrap());
            checkpoint("retained");
            drop(out);
        }
        _ => panic!("phase must be construct, proof or realize"),
    }
    checkpoint("released");
}
fn profile(phase: &str, iterations: usize) {
    let body = fixture::body(true, 1.0);
    let curves = wall(&body).curves();
    let policy = policy();
    println!(
        "pid={} phase={phase} iterations={iterations}",
        std::process::id()
    );
    for _ in 0..iterations {
        match phase {
            "construct" => {
                black_box(fixture::body(true, 1.0));
            }
            "proof" => {
                black_box(Surface::ruled(curves[0].clone(), curves[1].clone()).unwrap());
            }
            "realize" => {
                black_box(body.realize(Placement3::IDENTITY, &policy).unwrap());
            }
            _ => panic!("phase must be construct, proof or realize"),
        }
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|s| s == "--profile") {
        profile(
            args.get(2).map_or("proof", String::as_str),
            args.get(3).map_or(Ok(100000), |s| s.parse())?,
        );
        return Ok(());
    }
    let repetitions = args.get(1).map_or(Ok(51), |s| s.parse())?;
    assert!(repetitions > 0, "repetitions must be positive");
    let output = args.get(2).map(Path::new);
    if let Some(path) = output {
        std::fs::create_dir_all(path)?;
    }
    let mut rows = Vec::new();
    for (name, weighted, scale) in [
        ("polynomial-prism", false, 1.0),
        ("rational-prism", true, 1.0),
        ("rational-taper", true, 0.75),
    ] {
        let body = fixture::body(weighted, scale);
        let curves = wall(&body).curves();
        let author = measure(repetitions, || fixture::body(weighted, scale));
        let proof = measure(repetitions, || {
            Surface::ruled(curves[0].clone(), curves[1].clone()).unwrap()
        });
        let mut policy = policy();
        for tolerance in [0.01, 0.002] {
            policy.tolerance = tolerance;
            let time = measure(repetitions, || {
                body.realize(Placement3::IDENTITY, &policy).unwrap()
            });
            let realized = body.realize(Placement3::IDENTITY, &policy)?;
            assert!(
                realized.report.within_tolerance(),
                "fixture must meet requested geometry accuracy"
            );
            assert!(
                realized.mesh.boundary_loops()?.is_empty(),
                "fixture must be closed"
            );
            assert!(
                realized.mesh.validate_deep().is_empty(),
                "emitted mesh must pass validation"
            );
            let mut face_counts = vec![0; body.faces().len()];
            let triangles: Vec<_> = realized
                .mesh_faces
                .iter()
                .map(|&(mesh_face, native)| {
                    face_counts[native.index()] += 1;
                    let vertices: Vec<_> = realized
                        .mesh
                        .face_loop(mesh_face)
                        .map(|c| {
                            *realized
                                .mesh
                                .vertex_position(realized.mesh.to_vertex(c).unwrap())
                                .unwrap()
                        })
                        .collect();
                    (native.0, vertices)
                })
                .collect();
            let row = serde_json::json!({"case":name,"tolerance":tolerance,"min_edge_segments":policy.min_arc_segments,
                "construct_us":author,"proof_with_span_clones_us":proof,"realize_us":time,
                "triangles":triangles.len(),"per_face_triangles":face_counts,
                "edge_segments":realized.report.edge_segments,"deviation_bound":realized.report.max_deviation,
                "curve_bound":realized.report.max_curve_deviation,"surface_bound":realized.report.max_surface_deviation,
                "curve_span_bytes":size_of::<exedra_analytic::geometry::CurveSpan>(),"surface_bytes":size_of::<Surface>(),
                "quality": realized.report.quality.as_ref().map(|q|serde_json::json!({"within_target":q.within_target(),"remaining_bad_triangles":q.remaining_bad_triangles,"max_radius_edge_ratio":q.max_radius_edge_ratio,"grid_triangle_tests":q.grid_triangle_tests,"planar_steiner_points":q.planar_steiner_points,"candidate_insertions":q.candidate_insertions,"declined_insertions":q.declined_insertions,"budget_exhausted":q.budget_exhausted}))});
            println!("{row}");
            rows.push(row);
            if let Some(output) = output {
                std::fs::write(
                    output.join(format!("{name}-{tolerance}-triangles.json")),
                    serde_json::to_vec(&triangles)?,
                )?;
            }
        }
    }
    if let Some(output) = output {
        std::fs::write(
            output.join("timings.json"),
            serde_json::to_vec_pretty(&rows)?,
        )?;
    }
    Ok(())
}
