// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Executable wind-tunnel scenarios for assembly flattening at placement
//! scale.
//!
//! AS-1 places one compiled part (a subdivided icosphere standing in for a
//! vegetation asset) thousands of times under one frame and times assembly
//! construction and flattening in two representations: individual instances
//! (`mode=instances`) and one placement set (`mode=set`). Flattening uses the
//! default transformed-box bounds; `flatten_exact_ms` times exact per-vertex
//! bounds for comparison. Each mode asserts a deterministic flatten, and both
//! modes must agree on placed triangles and bounds, before timing.
//!
//! Run the quick profile (10k placements, the default):
//! `cargo run --release -p assembly_wind_tunnel -- --quick`
//!
//! Run the stress profile (100k placements):
//! `cargo run --release -p assembly_wind_tunnel -- --stress`

use std::hint::black_box;
use std::time::{Duration, Instant};

use exedra_assembly::{
    Assembly, BoundsPolicy, CompilePolicy, CompiledParts, FlattenOptions, PartCompiler, PartId,
    RenderList, flatten, flatten_with,
};
use exedra_constructive::ir::Placement3;
use exedra_mesh::Mesh;
use exedra_primitives::{IcosphereParams, icosphere};

fn main() {
    let placements = if std::env::args().any(|arg| arg == "--stress") {
        100_000
    } else {
        10_000
    };
    run_as1(placements);
}

/// A deterministic scatter: a square grid with keyed yaw.
fn scatter(count: usize) -> Vec<Placement3> {
    let side = (1..=count).find(|s| s * s >= count).unwrap_or(1);
    (0..count)
        .map(|i| {
            let (x, y) = ((i % side) as f64 * 7.0, (i / side) as f64 * 7.0);
            let yaw = (i as f64 * 2.399_963_229_728_653) % core::f64::consts::TAU;
            Placement3::rotate_z_then_translate(yaw, x, y, 0.0)
        })
        .collect()
}

fn tree_mesh() -> Mesh {
    icosphere(&IcosphereParams {
        radius: 3.0,
        subdivisions: 4,
    })
    .mesh
}

fn run_as1(count: usize) {
    let placements = scatter(count);
    // Mesh generation is not assembly work; build it before the timers.
    let mesh = tree_mesh();

    let started = Instant::now();
    let mut instances = Assembly::new();
    let part = instances
        .add_baked_part("tree", mesh.clone(), &[])
        .expect("baked part");
    let forest = instances
        .add_frame(None, "forest", Placement3::IDENTITY)
        .expect("frame");
    for (i, placement) in placements.iter().enumerate() {
        instances
            .add_instance(Some(forest), &format!("tree{i}"), part, *placement)
            .expect("instance");
    }
    let instances_build = started.elapsed();

    let started = Instant::now();
    let mut set = Assembly::new();
    let set_part = set.add_baked_part("tree", mesh, &[]).expect("baked part");
    let set_forest = set
        .add_frame(None, "forest", Placement3::IDENTITY)
        .expect("frame");
    set.add_placement_set(Some(set_forest), "trees", set_part, placements)
        .expect("placement set");
    let set_build = started.elapsed();

    let instance_list = report("instances", &instances, part, count, instances_build);
    let set_list = report("set", &set, set_part, count, set_build);
    assert_eq!(
        instance_list.triangle_count(),
        set_list.triangle_count(),
        "instances and a set place the same triangles"
    );
    assert_eq!(
        instance_list.bounds(),
        set_list.bounds(),
        "instances and a set have the same world bounds"
    );
}

fn report(
    mode: &str,
    assembly: &Assembly,
    part: PartId,
    count: usize,
    build: Duration,
) -> RenderList {
    let compiled = PartCompiler::new()
        .compile_parts(assembly, &CompilePolicy::default())
        .expect("compile");
    let vertices = compiled.part(part).expect("compiled").bodies[0]
        .tri
        .positions
        .len();

    // Determinism before timing.
    let first = flatten(assembly, &compiled);
    let second = flatten(assembly, &compiled);
    assert_eq!(
        first.placed_body_count(),
        count as u64,
        "every placement is flattened"
    );
    assert_eq!(first.bounds(), second.bounds(), "flatten is deterministic");
    assert_eq!(
        first.placed_body_count(),
        second.placed_body_count(),
        "flatten places the same bodies"
    );
    assert_eq!(
        first.triangle_count(),
        second.triangle_count(),
        "flatten places the same triangles"
    );
    assert!(
        first
            .items
            .iter()
            .zip(&second.items)
            .all(|(a, b)| a.path == b.path && a.world == b.world),
        "items flatten deterministically"
    );
    assert!(
        first
            .batches
            .iter()
            .zip(&second.batches)
            .all(|(a, b)| a.path == b.path && a.world == b.world),
        "batches flatten deterministically"
    );

    let flatten_time = best_of(3, || {
        black_box(flatten(black_box(assembly), black_box(&compiled)));
    });
    let exact_time = best_of(3, || {
        black_box(exact(black_box(assembly), black_box(&compiled)));
    });
    let bounds = first.bounds().expect("placed geometry");
    println!(
        "scenario=AS-1 mode={mode} placements={count} part_vertices={vertices} \
         build_ms={:.1} flatten_ms={:.1} flatten_exact_ms={:.1} \
         bounds_min={:?} bounds_max={:?}",
        ms(build),
        ms(flatten_time),
        ms(exact_time),
        bounds.min,
        bounds.max,
    );
    first
}

fn exact(assembly: &Assembly, compiled: &CompiledParts) -> RenderList {
    flatten_with(
        assembly,
        compiled,
        &FlattenOptions::default().with_bounds(BoundsPolicy::Exact),
    )
}

fn best_of(runs: usize, mut f: impl FnMut()) -> Duration {
    (0..runs)
        .map(|_| {
            let started = Instant::now();
            f();
            started.elapsed()
        })
        .min()
        .expect("at least one run")
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
