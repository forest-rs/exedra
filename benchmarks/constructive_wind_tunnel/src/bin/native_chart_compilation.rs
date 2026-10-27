// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native chart compilation, extraction and cache-hit workloads.

#[path = "../loft_fixture.rs"]
mod loft_fixture;
#[path = "../sweep_fixture.rs"]
mod sweep_fixture;
use exedra_analytic::{body::FaceId, revolve::Revolution};
use exedra_compile::{
    CompilePolicy, NativeSource, PartCompiler, PartId, PartInput, PartSource, PreparedSource,
};
use exedra_constructive::{
    chart::{ChartTransform, LoftStations, SurfaceChart},
    ir::Placement3,
};
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
use std::{hint::black_box, time::Instant};

fn revolution() -> NativeSource {
    let outline = Loop::new(vec![
        Segment::line([6.0, -2.0]),
        Segment::line([6.0, 2.0]),
        Segment::line([4.0, 2.0]),
        Segment::line([4.0, -2.0]),
    ])
    .unwrap();
    Revolution::new(
        Profile::new(outline, vec![], &ProfilePolicy::new(1e-9)).unwrap(),
        Placement3::IDENTITY,
        core::f64::consts::TAU,
    )
    .unwrap()
    .into()
}
fn measure(label: &str, native: &NativeSource, chart: Option<SurfaceChart>) -> serde_json::Value {
    let prepared = PreparedSource::new(PartSource::Native(native.clone()));
    let selection = native.select_face(FaceId(0)).unwrap();
    let mut policy = CompilePolicy {
        native_chart: chart,
        max_render_vertices: std::env::var("EXEDRA_RENDER_VERTICES")
            .ok()
            .map(|limit| limit.parse().expect("EXEDRA_RENDER_VERTICES is a u32")),
        ..Default::default()
    };
    policy.native.tolerance = 0.05;
    let inputs = [PartInput::prepared("component", &prepared)];
    let mut cold_ns = Vec::new();
    for pass in 0..104 {
        let mut compiler = PartCompiler::new();
        let start = Instant::now();
        let output = compiler.compile_snapshot(&inputs, &policy).unwrap();
        black_box(&output);
        let elapsed = start.elapsed().as_nanos();
        assert_eq!(
            compiler.counters().native_realizations,
            1,
            "fresh compilation realizes once"
        );
        assert_eq!(
            output
                .body(PartId(0), 0)
                .unwrap()
                .native()
                .unwrap()
                .resolve_face(&selection),
            Ok(FaceId(0)),
            "display policy retains native scope"
        );
        if pass >= 3 {
            cold_ns.push(elapsed);
        }
        drop(output);
    }
    let mut compiler = PartCompiler::new();
    let first = compiler.compile_snapshot(&inputs, &policy).unwrap();
    let body = first.body(PartId(0), 0).unwrap();
    assert_eq!(
        body.native_realization().unwrap().chart(),
        chart.as_ref(),
        "retained policy equals validated request"
    );
    let compiled = &first.compiled().part(PartId(0)).unwrap().bodies[0];
    assert!(
        compiled
            .regions
            .iter()
            .all(|region| region.has_uvs == chart.is_some()),
        "authored UV coverage is explicit"
    );
    let mut hits_ns = Vec::new();
    for pass in 0..104 {
        let start = Instant::now();
        let output = compiler.compile_snapshot(&inputs, &policy).unwrap();
        black_box(&output);
        let elapsed = start.elapsed().as_nanos();
        if pass >= 3 {
            hits_ns.push(elapsed);
        }
        drop(output);
    }
    assert_eq!(
        compiler.counters().native_realizations,
        1,
        "cache hits do not reconstruct native geometry"
    );
    serde_json::json!({"family":label,"charted":chart.is_some(),"render_vertices":compiled.tri.positions.len(),"triangles":compiled.tri.indices.len()/3,"cold_ns":cold_ns,"cache_hit_ns":hits_ns,"native_realizations_after_hits":compiler.counters().native_realizations,"native_boundary_bytes":native.boundary_storage().payload_bytes})
}
fn main() {
    let transform = ChartTransform {
        matrix: [[0.05, 0.0], [0.0, 0.05]],
        offset: [0.0; 2],
    };
    let turned = revolution();
    let profile = sweep_fixture::profile();
    let path = sweep_fixture::path();
    let swept = sweep_fixture::native(&profile, &path);
    drop(sweep_fixture::native_realization(&swept));
    drop(sweep_fixture::recipe_realization(&sweep_fixture::recipe(
        profile, path,
    )));
    let swept: NativeSource = swept.into();
    let sections = loft_fixture::sections(true);
    let loft = loft_fixture::native(&sections);
    drop(loft_fixture::native_mesh(&loft));
    drop(loft_fixture::charted_mesh(&loft));
    drop(loft_fixture::mesh_loft(&sections));
    let loft: NativeSource = loft.into();
    let mut rows = Vec::new();
    for (label, native, chart) in [
        (
            "revolution",
            turned,
            SurfaceChart::Revolve {
                reference_radius: 5.0,
                wall: transform,
                caps: transform,
            },
        ),
        (
            "sweep",
            swept,
            SurfaceChart::Sweep {
                wall: transform,
                caps: transform,
            },
        ),
        (
            "hollow loft",
            loft,
            SurfaceChart::Loft {
                reference_section: 0,
                rest_length: 100.0,
                stations: LoftStations::DatumDistance,
                wall: transform,
                caps: transform,
            },
        ),
    ] {
        for chart in [None, Some(chart)] {
            rows.push(measure(label, &native, chart));
        }
    }
    println!(
        "{}",
        serde_json::json!({"cases":rows,"max_render_vertices":std::env::var("EXEDRA_RENDER_VERTICES").ok(),"scope":"release native-source compile_snapshot with actual render extraction and retained correspondence; source installation, prepared identity, fixture preparation, query verification and output destruction excluded; cold compiler created before clock; cache-hit compiler seeded before measurements; no JS, GPU or file I/O"})
    );
}
