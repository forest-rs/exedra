// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Full-turn pole construction, bounded realization and triangle quality.
//! Timings are native CPU only, excluding output destruction and all JS/GPU work.
//! Pass `0` repetitions for deterministic quality inspection without timing,
//! or `sample` for a fixed-source CPU sampling window.

use exedra_analytic::{realize::RealizationPolicy, revolve::Revolution};
use exedra_constructive::ir::Placement3;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
use serde_json::{Value, json};
use std::{error::Error, hint::black_box, time::Instant};

fn profile(kind: &str, radius: f64) -> Profile {
    let segments = match kind {
        "sphere" => vec![
            Segment::line([0.0, -radius]),
            Segment::arc([0.0, radius], 1.0),
        ],
        "cone" => vec![
            Segment::line([radius, 0.0]),
            Segment::line([0.0, 1.5 * radius]),
            Segment::line([0.0, 0.0]),
        ],
        _ => panic!("unknown pole fixture"),
    };
    Profile::new(
        Loop::new(segments).unwrap(),
        vec![],
        &ProfilePolicy::new(radius * 1e-9),
    )
    .unwrap()
}
fn percentile(mut values: Vec<f64>, percent: usize) -> f64 {
    values.sort_unstable_by(f64::total_cmp);
    values[((values.len() - 1) * percent + 50) / 100]
}
fn quality(mesh: &exedra_mesh::Mesh) -> Value {
    let mut angles = Vec::new();
    let mut aspects = Vec::new();
    let mut volume = 0.0;
    for face in mesh.faces() {
        let corners: Vec<_> = mesh.face_loop(face).collect();
        assert_eq!(corners.len(), 3, "native realization emits triangles");
        let points: [[f64; 3]; 3] = core::array::from_fn(|i| {
            mesh.vertex_position(mesh.to_vertex(corners[i]).unwrap())
                .unwrap()
                .map(f64::from)
        });
        let sub = |a: [f64; 3], b: [f64; 3]| core::array::from_fn(|i| a[i] - b[i]);
        let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
        let cross = |a: [f64; 3], b: [f64; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let length = |p: [f64; 3]| p[0].hypot(p[1]).hypot(p[2]);
        let lengths =
            core::array::from_fn::<_, 3, _>(|i| length(sub(points[(i + 1) % 3], points[i])));
        let twice_area = length(cross(sub(points[1], points[0]), sub(points[2], points[0])));
        assert!(
            twice_area > 0.0,
            "pole realization must not emit zero-area triangles"
        );
        for i in 0..3 {
            let a = sub(points[(i + 1) % 3], points[i]);
            let b = sub(points[(i + 2) % 3], points[i]);
            angles.push(length(cross(a, b)).atan2(dot(a, b)).to_degrees());
        }
        let max = lengths.into_iter().fold(0.0_f64, f64::max);
        // Longest edge / corresponding altitude, 2/sqrt(3) for equilateral.
        aspects.push(max * max / twice_area);
        volume += dot(points[0], cross(points[1], points[2])) / 6.0;
    }
    json!({"min_angle_degrees":percentile(angles.clone(),0),"p01_angle_degrees":percentile(angles,1),"max_aspect":percentile(aspects.clone(),100),"p99_aspect":percentile(aspects,99),"mesh_volume":volume})
}
/// One fixed-source realization for an existing external allocation profiler.
/// Constructor/source buffers and checkpoint machinery are outside the window.
pub fn allocation_window(mut checkpoint: impl FnMut(&str)) {
    let solid = Revolution::new(
        profile("sphere", 20.0),
        Placement3::IDENTITY,
        core::f64::consts::TAU,
    )
    .unwrap();
    let policy = RealizationPolicy {
        tolerance: 0.025,
        ..Default::default()
    };
    checkpoint("ready");
    let output = black_box(solid.realize(&policy).unwrap());
    checkpoint("retained");
    drop(output);
    checkpoint("released");
}
fn main() -> Result<(), Box<dyn Error>> {
    let mode = std::env::args().nth(1);
    if mode.as_deref() == Some("sample") {
        use std::io::{self, Write};
        let solid = Revolution::new(
            profile("sphere", 20.0),
            Placement3::IDENTITY,
            core::f64::consts::TAU,
        )?;
        let policy = RealizationPolicy {
            tolerance: 0.025,
            ..Default::default()
        };
        println!(
            "sample_ready pid={}; press Enter to realize for 15 seconds",
            std::process::id()
        );
        io::stdout().flush()?;
        io::stdin().read_line(&mut String::new())?;
        let start = Instant::now();
        let mut count = 0_u64;
        while start.elapsed().as_secs() < 15 {
            drop(black_box(solid.realize(black_box(&policy))?));
            count += 1;
        }
        println!("sample_done realizations={count}");
        return Ok(());
    }
    let reps = mode.map_or(Ok(9_usize), |s| s.parse())?;
    let mut rows = Vec::new();
    for kind in ["sphere", "cone"] {
        let source = profile(kind, 20.0);
        let solid = Revolution::new(source.clone(), Placement3::IDENTITY, core::f64::consts::TAU)?;
        for tolerance in [0.4, 0.1, 0.025] {
            let policy = RealizationPolicy {
                tolerance,
                ..Default::default()
            };
            let output = solid.realize(&policy)?;
            assert!(
                output.mesh.validate_deep().is_empty(),
                "realized topology must be valid"
            );
            assert!(
                output.mesh.boundary_loops()?.is_empty(),
                "native solid mesh must be closed"
            );
            assert!(
                output.report.within_tolerance(),
                "benchmark requires achieved requested accuracy"
            );
            let mut construct = Vec::new();
            let mut realize = Vec::new();
            for pass in 0..if reps == 0 { 0 } else { reps + 2 } {
                let start = Instant::now();
                let model = black_box(Revolution::new(
                    black_box(source.clone()),
                    Placement3::IDENTITY,
                    core::f64::consts::TAU,
                )?);
                let us = start.elapsed().as_secs_f64() * 1e6;
                if pass >= 2 {
                    construct.push(us);
                }
                drop(model);
                let start = Instant::now();
                let mesh = black_box(solid.realize(black_box(&policy))?);
                let us = start.elapsed().as_secs_f64() * 1e6;
                if pass >= 2 {
                    realize.push(us);
                }
                drop(mesh);
            }
            let timing = |values: Vec<f64>, percent| {
                (!values.is_empty()).then(|| percentile(values, percent))
            };
            rows.push(json!({"kind":kind,"radius":20.0,"tolerance":tolerance,"vertices":output.mesh_vertices.len(),"triangles":output.mesh_faces.len(),"pole_corners":output.pole_corners.len(),"max_deviation":output.report.max_deviation,"native_volume":solid.volume(),"native_area":solid.surface_area(),"construct_us_p50":timing(construct.clone(),50),"construct_us_p95":timing(construct,95),"realize_us_p50":timing(realize.clone(),50),"realize_us_p95":timing(realize,95),"quality":quality(&output.mesh)}));
        }
    }
    println!(
        "{}",
        json!({"scope":"native CPU construction includes Profile clone; realization uses fixed retained constructor; output destruction, diagnostics/quality measurement, STEP, Wasm/JS/GPU excluded; topology and achieved accuracy asserted before timing", "cases":rows})
    );
    Ok(())
}
