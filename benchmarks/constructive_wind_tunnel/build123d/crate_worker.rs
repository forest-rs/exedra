// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_triangulate::{BoundarySplits, PolygonInput, RefineParams, TriParams};
use i_triangle::float::triangulatable::Triangulatable;
use serde_json::{Value, json};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};
#[cfg(feature = "heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;
fn run(backend: &str, loops: &[Vec<[f64; 2]>]) -> (Vec<[f64; 2]>, Vec<[u32; 3]>) {
    let holes: Vec<_> = loops[1..].iter().map(Vec::as_slice).collect();
    let input = PolygonInput {
        outer: &loops[0],
        holes: &holes,
    };
    let points: Vec<_> = loops.iter().flatten().copied().collect();
    match backend {
        "exedra-ear" | "exedra-cdt" => {
            let p = if backend == "exedra-ear" {
                TriParams::ear_clip()
            } else {
                TriParams::constrained_delaunay()
            };
            let t = exedra_triangulate::triangulate_preserving_boundary(&input, &p).unwrap();
            (points, t.triangles)
        }
        "exedra-refine" => {
            let p = RefineParams::default().with_boundary_splits(BoundarySplits::Forbidden);
            let t = exedra_triangulate::refine(&input, &p).unwrap();
            (t.points, t.triangles)
        }
        "i-triangle" => {
            let t = loops.triangulate().to_triangulation::<u32>();
            (
                t.points,
                t.indices
                    .chunks_exact(3)
                    .map(|t| [t[0], t[1], t[2]])
                    .collect(),
            )
        }
        "i-cdt" | "i-refine" => {
            let t = loops.triangulate().into_delaunay();
            let t = if backend == "i-refine" {
                t.refine_with_circumcenters(0.1)
            } else {
                t
            };
            let t = t.to_triangulation::<u32>();
            (
                t.points,
                t.indices
                    .chunks_exact(3)
                    .map(|t| [t[0], t[1], t[2]])
                    .collect(),
            )
        }
        "earclip" | "earcutr" => {
            let mut bases = Vec::new();
            let mut count = 0;
            for l in loops {
                bases.push(count);
                count += l.len();
            }
            let flat: Vec<_> = points.iter().flatten().copied().collect();
            let t = if backend == "earcutr" {
                earcutr::earcut(&flat, &bases[1..], 2).unwrap()
            } else {
                earclip::earcut(&flat, &bases[1..], 2)
            };
            (
                points,
                t.chunks_exact(3)
                    .map(|t| [t[0] as u32, t[1] as u32, t[2] as u32])
                    .collect(),
            )
        }
        _ => panic!("unknown backend"),
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let backend = &args[1];
    let id = &args[2];
    let cases: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(&args[3]).unwrap()).unwrap();
    let case = cases.iter().find(|c| c["id"] == id.as_str()).unwrap();
    let loops: Vec<Vec<[f64; 2]>> = serde_json::from_value(case["loops"].clone()).unwrap();
    black_box(run(backend, &loops));
    #[cfg(feature = "heap")]
    let profiler = dhat::Profiler::builder().file_name(&args[4]).build();
    let count = if cfg!(feature = "heap") { 3 } else { 11 };
    let mut samples = Vec::with_capacity(count);
    let seconds = args.get(4).and_then(|v| v.parse::<f64>().ok());
    let start = Instant::now();
    let mut work = 0;
    loop {
        let t = Instant::now();
        let out = run(backend, &loops);
        black_box(&out);
        samples.push(t.elapsed().as_nanos() as u64);
        drop(out);
        work += 1;
        if seconds.is_none() && work >= count
            || seconds.is_some_and(|s| start.elapsed() >= Duration::from_secs_f64(s))
        {
            break;
        }
    }
    #[cfg(feature = "heap")]
    drop(profiler);
    let (points, triangles) = run(backend, &loops);
    println!(
        "{}",
        json!({"id":id,"backend":backend,"positions":points.iter().map(|p|[p[0],p[1],0.0]).collect::<Vec<_>>(),"triangles":triangles,"timings_ns":samples,"work":work})
    );
}
