#!/usr/bin/env python3
# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Profile bridge edits or a fixed native workload with isolated dhat."""
import argparse
import json
import pathlib
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--repo", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[2])
parser.add_argument("--output", type=pathlib.Path, required=True)
parser.add_argument("--workload", choices=["authoring", "render_vertices", "native_poles"], default="authoring")
parser.add_argument("--prepare-only", action="store_true", help="write the isolated profiler without building or running it")
args = parser.parse_args()
repo, output = args.repo.resolve(), args.output.resolve()
output.mkdir(parents=True, exist_ok=True)
(output / "src").mkdir(exist_ok=True)
bridge = repo / "apps/exedra_authoring_web_bridge"
worker_name = {"authoring": "authoring_bridge.rs", "render_vertices": "render_vertex_budget.rs", "native_poles": "native_poles.rs"}[args.workload]
worker = pathlib.Path(__file__).resolve().parent / "src/bin" / worker_name
scope = {
    "authoring": "three bridge edits including Rust host metadata/report parsing, output staging and destruction; requests and warmup excluded; not Wasm JS copies",
    "render_vertices": "one render extraction; fixed 1000-triangle source and profiler setup excluded; custom-only normals avoid global normal derivation; output destruction included",
    "native_poles": "one native sphere realization, R20/tolerance0.025; retained source constructor/profile and profiler setup excluded; topology, correspondence, normals and release included; no compilation/extraction/STEP/Wasm/JS/GPU",
}[args.workload]
extra_dependencies = ""
if args.workload == "native_poles":
    extra_dependencies = "\n".join(f'{crate} = {{ path = {json.dumps(str(repo / "crates" / crate))} }}' for crate in ["exedra_analytic", "exedra_constructive", "exedra_profile"]) + "\n"
(output / "Cargo.toml").write_text(f'''[package]
name = "exedra-authoring-allocation-profile"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
exedra_authoring_web_bridge = {{ path = {json.dumps(str(bridge))} }}
exedra_mesh = {{ path = {json.dumps(str(repo / "crates/exedra_mesh"))} }}
serde_json = {{ version = "=1.0.151", features = ["float_roundtrip"] }}
dhat = "=0.3.3"
{extra_dependencies}''')
(output / "src/main.rs").write_text(f'''#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;
#[allow(dead_code)]
#[path = {json.dumps(str(worker))}]
mod workload;
fn main() {{
    let mut profiler = None;
    let mut checkpoints = Vec::with_capacity(2);
    workload::allocation_window(|phase| match phase {{
        "ready" => profiler = Some(dhat::Profiler::builder().file_name({json.dumps(str(output / "dhat.json"))}).build()),
        "retained" | "released" => checkpoints.push((phase == "retained", dhat::HeapStats::get())),
        _ => unreachable!(),
    }});
    drop(profiler);
    let checkpoints:Vec<_> = checkpoints.iter().map(|(phase,s)| serde_json::json!({{
        "phase":if *phase {{ "retained" }} else {{ "released" }},"total_blocks":s.total_blocks,"total_bytes":s.total_bytes,
        "current_blocks":s.curr_blocks,"current_bytes":s.curr_bytes,
        "peak_blocks":s.max_blocks,"peak_bytes":s.max_bytes
    }})).collect();
    println!("{{}}",serde_json::json!({{"checkpoints":checkpoints,"scope":{json.dumps(scope)}}}));
}}
''')
if args.prepare_only:
    print(output / "Cargo.toml")
    sys.exit(0)
subprocess.run(["cargo", "build", "--release", "--offline", "--manifest-path", str(output / "Cargo.toml")], check=True)
result = subprocess.check_output([str(output / "target/release/exedra-authoring-allocation-profile")], text=True)
(output / "counts.json").write_text(result)
print(result, end="")
