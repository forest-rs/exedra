#!/usr/bin/env python3
# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Profile three exact bore model/mesh/exchange results with the existing isolated dhat pattern."""
import argparse
import json
import os
import pathlib
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--repo", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[3])
parser.add_argument("--output", type=pathlib.Path, required=True)
parser.add_argument("--workload", choices=["bores", "refinement", "machining"], default="bores")
args = parser.parse_args()
repo, output = args.repo.resolve(), args.output.resolve()
output.mkdir(parents=True, exist_ok=True)
(output / "src").mkdir(exist_ok=True)
worker = (repo / "examples/step_exchange/src/bin/bore_study.rs" if args.workload == "bores"
          else repo / "benchmarks/constructive_wind_tunnel/src/bin/native_machining.rs" if args.workload == "machining"
          else repo / "benchmarks/constructive_wind_tunnel/src/bin/accuracy_refinement.rs")
scope = ("three bore constructions, realizations and STEP exports including retained original Extrusion records; original preauthored source, requests, containers and warmup excluded; native_only drops display/export outputs; released drops models; no compiler, JS or GPU" if args.workload == "bores" else
         "retained native through-cylinder pattern construction plus operation history; prepared source/tools and warmup excluded; native_only drops history, released drops final body; no display, STEP, compiler, JS or GPU" if args.workload == "machining" else
         "one translated-bore display realization including optional refinement; native model construction and warmup excluded; retained holds delivered mesh only; released drops mesh; no compiler, JS or GPU")
extra_dependencies = ("\n".join(
    f'{name} = {{ path = {json.dumps(str(repo / "crates" / name))}, features = ["std"] }}'
    for name in ["exedra_constructive", "exedra_mesh"])
    if args.workload == "refinement" else "")
(output / "Cargo.toml").write_text(f'''[package]
name = "exedra-bore-allocation-profile"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
exedra_analytic = {{ path = {json.dumps(str(repo / "crates/exedra_analytic"))}, features = ["std"] }}
{extra_dependencies}
exedra_math = {{ path = {json.dumps(str(repo / "crates/exedra_math"))}, features = ["std"] }}
exedra_profile = {{ path = {json.dumps(str(repo / "crates/exedra_profile"))} }}
exedra_step = {{ path = {json.dumps(str(repo / "crates/exedra_step"))}, features = ["std"] }}
serde_json = {{ version = "=1.0.151", features = ["float_roundtrip"] }}
dhat = "=0.3.3"
''')
(output / "src/main.rs").write_text(f'''#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;
#[allow(dead_code)]
#[path = {json.dumps(str(worker))}]
mod workload;
fn main() {{
    let mut profiler = None;
    let mut checkpoints = Vec::with_capacity(3);
    workload::allocation_window(|phase| match phase {{
        "ready" => profiler = Some(dhat::Profiler::builder().file_name({json.dumps(str(output / "dhat.json"))}).build()),
        "retained" => checkpoints.push((0, dhat::HeapStats::get())),
        "native_only" => checkpoints.push((1, dhat::HeapStats::get())),
        "released" => checkpoints.push((2, dhat::HeapStats::get())),
        _ => unreachable!(),
    }});
    drop(profiler);
    let checkpoints:Vec<_> = checkpoints.iter().map(|(phase,s)| serde_json::json!({{
        "phase":match phase {{ 0 => "retained", 1 => "native_only", _ => "released" }},"total_blocks":s.total_blocks,"total_bytes":s.total_bytes,
        "current_blocks":s.curr_blocks,"current_bytes":s.curr_bytes,
        "peak_blocks":s.max_blocks,"peak_bytes":s.max_bytes
    }})).collect();
    println!("{{}}",serde_json::json!({{"checkpoints":checkpoints,"scope":{json.dumps(scope)}}}));
}}
''')
env = dict(os.environ, CARGO_TARGET_DIR=str(output / "target"), CARGO_INCREMENTAL="0")
subprocess.run(["cargo", "build", "--release", "--offline", "--manifest-path", str(output / "Cargo.toml")], check=True, env=env)
result = subprocess.check_output([str(output / "target/release/exedra-bore-allocation-profile")], text=True)
(output / "counts.json").write_text(result)
print(result, end="")
