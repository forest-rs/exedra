#!/usr/bin/env python3
# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Profile three real bridge edits with the existing isolated dhat pattern."""
import argparse
import json
import pathlib
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--repo", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[2])
parser.add_argument("--output", type=pathlib.Path, required=True)
args = parser.parse_args()
repo, output = args.repo.resolve(), args.output.resolve()
output.mkdir(parents=True, exist_ok=True)
(output / "src").mkdir(exist_ok=True)
bridge = repo / "apps/exedra_authoring_web_bridge"
worker = pathlib.Path(__file__).resolve().parent / "src/bin/authoring_bridge.rs"
(output / "Cargo.toml").write_text(f'''[package]
name = "exedra-authoring-allocation-profile"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
exedra_authoring_web_bridge = {{ path = {json.dumps(str(bridge))} }}
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
    println!("{{}}",serde_json::json!({{"checkpoints":checkpoints,"scope":"three bridge edits including Rust host metadata/report parsing, output staging and destruction; requests and warmup excluded; not Wasm JS copies"}}));
}}
''')
subprocess.run(["cargo", "build", "--release", "--offline", "--manifest-path", str(output / "Cargo.toml")], check=True)
result = subprocess.check_output([str(output / "target/release/exedra-authoring-allocation-profile")], text=True)
(output / "counts.json").write_text(result)
print(result, end="")
