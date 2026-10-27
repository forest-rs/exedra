// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Actual browser sample graph through the native bridge, with phase timings.

#[cfg(not(target_arch = "wasm32"))]
mod host {
    use exedra_authoring_web_bridge::AuthoringKernel;
    use std::{
        hint::black_box,
        io::{self, Write},
        time::Instant,
    };

    fn request(width: f64) -> String {
        serde_json::json!({
        "units":"mm","separation":1e-6,
        "profiles":[
            {"kind":"roundedRectangle","width":width,"height":80,"radius":6},
            {"kind":"tagged","profile":0,"name":"panel"},
            {"kind":"slot","length":22,"radius":3},
            {"kind":"tagged","profile":2,"name":"vents"},
            {"kind":"cutouts","profile":1,"template":3,"positions":[[-36,22],[-12,22],[12,22],[36,22]]},
            {"kind":"slot","length":48,"radius":8},
            {"kind":"tagged","profile":5,"name":"recess"},
            {"kind":"circle","radius":3},
            {"kind":"tagged","profile":7,"name":"pin"}
        ],
        "nodes":[
            {"kind":"extrude","profile":4,"height":12},
            {"kind":"pocket","base":0,"profile":6,"end":"end","depth":5,"at":[0,-10]},
            {"kind":"onFace","host":1,"face":{"kind":"floor","feature":"recess"},"profile":8,"height":7,"offset":0.5,"at":[0,-10]}
        ],"roots":[{"name":"panel","node":1},{"name":"pin","node":2}]
    }).to_string()
    }
    #[derive(Debug)]
    struct Sample {
        build_decode_lower_metadata_ns: u128,
        native_host_metadata_parse_ns: u128,
        realize_stage_buffers_ns: u128,
        native_take_buffers_report_parse_ns: u128,
        release_previous_ns: u128,
        total_ns: u128,
        triangles: usize,
        buffer_bytes: usize,
        cache_hits: u64,
        native_realizations: u64,
    }
    fn revision(
        kernel: &mut AuthoringKernel,
        previous: &mut Option<u32>,
        request: &str,
        tolerance: f64,
    ) -> Sample {
        let start = Instant::now();
        let metadata = kernel
            .build_document(request)
            .expect("valid sample authoring");
        let build = start.elapsed().as_nanos();
        let phase = Instant::now();
        let metadata: serde_json::Value = serde_json::from_str(&metadata).expect("native metadata");
        let id = u32::try_from(metadata["id"].as_u64().expect("document ID")).expect("u32 ID");
        let metadata_parse = phase.elapsed().as_nanos();
        let options = format!("{{\"tolerance\":{tolerance},\"maxVertices\":250000}}");
        let mut realize = 0;
        let mut transfer = 0;
        let mut buffers = Vec::new();
        let mut triangles = 0;
        let mut bytes = 0;
        let mut hits = 0;
        let mut realizations = 0;
        for part in 0..2 {
            let phase = Instant::now();
            let mut raw = kernel.realize(id, part, &options).expect("valid display");
            realize += phase.elapsed().as_nanos();
            let phase = Instant::now();
            let positions = raw.take_positions();
            let normals = raw.take_normals();
            let indices = raw.take_indices();
            let faces = raw.take_native_faces();
            let report: serde_json::Value =
                serde_json::from_str(&raw.report_json()).expect("display report");
            transfer += phase.elapsed().as_nanos();
            assert_eq!(
                faces.len() * 3,
                indices.len(),
                "complete native triangle correspondence"
            );
            assert_eq!(
                report["withinTolerance"], true,
                "sample accuracy must be met"
            );
            assert!(
                indices.iter().all(|&i| i as usize * 3 < positions.len()),
                "indices address owned positions"
            );
            triangles += faces.len();
            bytes += (positions.len() + normals.len() + indices.len() + faces.len()) * 4;
            hits += report["cacheHits"].as_u64().expect("work counter");
            realizations += report["nativeRealizations"].as_u64().expect("work counter");
            buffers.push((positions, normals, indices, faces));
        }
        let phase = Instant::now();
        if let Some(previous) = previous.replace(id) {
            kernel.release_document(previous);
        }
        let release = phase.elapsed().as_nanos();
        black_box(&buffers);
        let total = start.elapsed().as_nanos();
        drop(buffers);
        Sample {
            build_decode_lower_metadata_ns: build,
            native_host_metadata_parse_ns: metadata_parse,
            realize_stage_buffers_ns: realize,
            native_take_buffers_report_parse_ns: transfer,
            release_previous_ns: release,
            total_ns: total,
            triangles,
            buffer_bytes: bytes,
            cache_hits: hits,
            native_realizations: realizations,
        }
    }
    fn flush_stack_log() {
        let blocks: Vec<_> = (0..8192)
            .map(|i| Vec::<u8>::with_capacity(32 + i % 16))
            .collect();
        black_box(blocks);
    }
    fn pause(message: &str) {
        println!("{message} pid={}", std::process::id());
        io::stdout().flush().expect("stdout");
        io::stdin().read_line(&mut String::new()).expect("stdin");
    }
    /// Runs three real edits after warmup, then releases the final native owners.
    /// Checkpoints permit an external allocator harness without workspace dependencies.
    /// Requests are prepared before the window; host metadata/report parsing stays included.
    pub fn allocation_window(mut checkpoint: impl FnMut(&str)) {
        let mut kernel = AuthoringKernel::new();
        let mut previous = None;
        black_box(revision(&mut kernel, &mut previous, &request(120.0), 0.12));
        let requests = [request(120.01), request(120.02), request(120.03)];
        checkpoint("ready");
        for input in &requests {
            black_box(revision(&mut kernel, &mut previous, input, 0.12));
        }
        checkpoint("retained");
        if let Some(id) = previous.take() {
            kernel.release_document(id);
        }
        checkpoint("released");
    }

    pub(super) fn run() {
        if std::env::args().nth(1).as_deref() == Some("allocations") {
            allocation_window(|phase| {
                flush_stack_log();
                pause(phase);
            });
            return;
        }
        let mut kernel = AuthoringKernel::new();
        let mut previous = None;
        let medium = 0.12;
        black_box(revision(
            &mut kernel,
            &mut previous,
            &request(120.0),
            medium,
        ));
        match std::env::args().nth(1).as_deref() {
            Some("sample") => {
                pause("sample_ready; press Enter to run 15 seconds");
                let start = Instant::now();
                let mut count = 0_u32;
                while start.elapsed().as_secs() < 15 {
                    let input = request(120.01 + f64::from(count % 4096) * 0.005);
                    black_box(revision(&mut kernel, &mut previous, &input, medium));
                    count += 1;
                }
                println!("sample_done revisions={count}");
            }
            None => {
                let mut results = Vec::new();
                for tolerance in [0.45, 0.12, 0.035] {
                    for i in 0..3 {
                        black_box(revision(
                            &mut kernel,
                            &mut previous,
                            &request(120.01 + f64::from(i) * 0.01),
                            tolerance,
                        ));
                    }
                    let samples: Vec<_> = (0..101)
                        .map(|i| {
                            revision(
                                &mut kernel,
                                &mut previous,
                                &request(121.0 + f64::from(i) * 0.01),
                                tolerance,
                            )
                        })
                        .collect();
                    let samples:Vec<_>=samples.iter().map(|s| serde_json::json!({
                    "build_decode_lower_metadata_ns":s.build_decode_lower_metadata_ns,
                    "native_host_metadata_parse_ns":s.native_host_metadata_parse_ns,
                    "realize_stage_buffers_ns":s.realize_stage_buffers_ns,
                    "native_take_buffers_report_parse_ns":s.native_take_buffers_report_parse_ns,
                    "release_previous_ns":s.release_previous_ns,"total_ns":s.total_ns,
                    "triangles":s.triangles,"buffer_bytes":s.buffer_bytes,"cache_hits":s.cache_hits,"native_realizations":s.native_realizations
                })).collect();
                    results.push(serde_json::json!({"tolerance":tolerance,"samples":samples}));
                }
                println!(
                    "{}",
                    serde_json::json!({"workload":"actual authorPanel graph; replace old document after two displays","results":results,"ownership":serde_json::from_str::<serde_json::Value>(&kernel.stats_json()).unwrap(),"limits":"native host parses metadata/reports; take_* moves Vec, excludes Wasm JS array copies, SDK graph encoding, UI debounce/scene/GPU, output buffer release; request prepared outside timing"})
                );
            }
            _ => panic!("expected sample or allocations"),
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub use host::allocation_window;

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    host::run();
    #[cfg(target_arch = "wasm32")]
    println!("Native host benchmark; use the browser instrumentation for Wasm measurements.");
}
