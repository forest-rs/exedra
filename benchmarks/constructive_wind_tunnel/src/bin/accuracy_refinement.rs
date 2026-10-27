// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Matched release workloads for accuracy refinement and ordinary native families.

#[path = "../loft_fixture.rs"]
mod loft_fixture;
#[path = "../sweep_fixture.rs"]
mod sweep_fixture;
use exedra_analytic::{
    bore::{Bore, BorePolicy, BoredPlate},
    extrusion::Extrusion,
    realize::{RealizationPolicy, RealizedBody},
    revolve::Revolution,
};
use exedra_constructive::ir::Placement3;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
use std::{hint::black_box, time::Instant};

fn profile(points: &[[f64; 2]], size: f64) -> Profile {
    Profile::new(
        Loop::new(
            points
                .iter()
                .map(|p| Segment::line(p.map(|v| v * size)))
                .collect(),
        )
        .unwrap(),
        vec![],
        &ProfilePolicy::new(1e-8 * size),
    )
    .unwrap()
}
fn bore(angle: f64, size: f64, frame: Placement3) -> BoredPlate {
    let source = Extrusion::new(
        profile(
            &[[35.0, -25.0], [35.0, 25.0], [-35.0, 25.0], [-35.0, -25.0]],
            size,
        ),
        frame,
        20.0 * size,
    )
    .unwrap();
    let angle = angle.to_radians();
    source
        .with_through_bore(
            Bore::new(
                [-10.0 * size * angle.tan(), 0.0, 0.0],
                [angle.sin(), 0.0, angle.cos()],
                6.0 * size,
            )
            .unwrap(),
            &BorePolicy {
                separation: 0.1 * size,
                geometry_gap: 1e-8 * size,
                min_axis_dot: 0.1,
            },
        )
        .unwrap()
}
fn geometry_fingerprint(output: &RealizedBody) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    let mut word = |word: u64| {
        hash ^= word;
        hash = hash.wrapping_mul(0x100000001b3);
    };
    for vertex in output.mesh.vertices() {
        for value in output.mesh.vertex_position(vertex).unwrap() {
            word(u64::from(value.to_bits()));
        }
    }
    for &(face, native) in &output.mesh_faces {
        word(u64::from(native.0));
        for corner in output.mesh.face_loop(face) {
            word(u64::from(output.mesh.from_vertex(corner).unwrap().index()));
        }
    }
    for edge in output.mesh.half_edges() {
        word(u64::from(
            output.mesh.edge_sharpness(edge).unwrap_or(0.0).to_bits(),
        ));
    }
    hash
}
fn measure(label: &str, action: impl Fn() -> RealizedBody) -> serde_json::Value {
    let output = action();
    assert!(
        output.mesh.validate_deep().is_empty(),
        "workload must emit valid topology"
    );
    assert!(
        output.mesh.boundary_loops().unwrap().is_empty(),
        "workload must stay closed"
    );
    let fingerprint = geometry_fingerprint(&output);
    let report = format!("{:?}", output.report);
    let (vertices, triangles, deviation, met) = (
        output.mesh.vertices().count(),
        output.mesh_faces.len(),
        output.report.max_deviation,
        output.report.within_tolerance(),
    );
    drop(output);
    for _ in 0..3 {
        drop(action());
    }
    let samples: Vec<_> = (0..101)
        .map(|_| {
            let start = Instant::now();
            let output = action();
            black_box(&output);
            let elapsed = start.elapsed().as_nanos();
            drop(output);
            elapsed
        })
        .collect();
    serde_json::json!({"label":label,"vertices":vertices,"triangles":triangles,"geometry_fingerprint":format!("0x{fingerprint:016x}"),"max_deviation":deviation,"within_tolerance":met,"report_debug":report,"samples_ns":samples})
}
/// Display allocation window; native authoring and one warm realization precede it.
pub fn allocation_window(mut checkpoint: impl FnMut(&str)) {
    let size = 1e-6;
    let model = bore(
        30.0,
        size,
        Placement3::rotate_y_then_translate(0.35, 1000.0 * size, -700.0 * size, 300.0 * size),
    );
    let policy = RealizationPolicy {
        tolerance: 0.025 * size,
        geometry_gap: 1e-8 * size,
        ..Default::default()
    };
    drop(model.body().realize(model.frame(), &policy).unwrap());
    checkpoint("ready");
    let output = model.body().realize(model.frame(), &policy).unwrap();
    black_box(&output);
    checkpoint("retained");
    drop(output);
    checkpoint("released");
}

fn main() {
    let mut cases = Vec::new();
    for angle in [0.0, 30.0, 60.0] {
        let model = bore(angle, 1.0, Placement3::IDENTITY);
        let policy = RealizationPolicy {
            tolerance: 0.025,
            geometry_gap: 1e-8,
            ..Default::default()
        };
        cases.push(measure(&format!("bore {angle}"), || {
            model.body().realize(model.frame(), &policy).unwrap()
        }));
    }
    let size = 1e-6;
    let model = bore(
        30.0,
        size,
        Placement3::rotate_y_then_translate(0.35, 1000.0 * size, -700.0 * size, 300.0 * size),
    );
    for maximum in [4096, 37] {
        let policy = RealizationPolicy {
            tolerance: 0.025 * size,
            geometry_gap: 1e-8 * size,
            max_edge_segments: maximum,
            ..Default::default()
        };
        cases.push(measure(
            if maximum == 37 {
                "translated bore, capped edge budget"
            } else {
                "translated bore"
            },
            || model.body().realize(model.frame(), &policy).unwrap(),
        ));
    }
    let turned = Revolution::new(
        profile(&[[5.0, 0.0], [10.0, 0.0], [10.0, 20.0], [5.0, 20.0]], 1.0),
        Placement3::IDENTITY,
        core::f64::consts::TAU,
    )
    .unwrap();
    cases.push(measure("revolution", || {
        turned.realize(&RealizationPolicy::default()).unwrap()
    }));
    let profile = sweep_fixture::profile();
    let path = sweep_fixture::path();
    let swept = sweep_fixture::native(&profile, &path);
    // Exercise the existing matched fixture's mesh counterpart outside timing.
    drop(sweep_fixture::recipe_realization(&sweep_fixture::recipe(
        profile, path,
    )));
    cases.push(measure("sweep", || {
        sweep_fixture::native_realization(&swept)
    }));
    for hollow in [false, true] {
        let sections = loft_fixture::sections(hollow);
        let loft = loft_fixture::native(&sections);
        drop(loft_fixture::charted_mesh(&loft));
        drop(loft_fixture::mesh_loft(&sections));
        cases.push(measure(
            if hollow { "hollow loft" } else { "solid loft" },
            || loft_fixture::native_mesh(&loft),
        ));
    }
    println!(
        "{}",
        serde_json::json!({"cases":cases,"scope":"preauthored native source, validity/hash checks, warmup, output destruction, mesh counterpart preparation and serialization excluded from timing; each timed realization retains all returned buffers at stop; no compiler cache, JS or GPU"})
    );
}
