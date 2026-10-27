#!/usr/bin/env python3
# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Run the same public native workload against two source trees in release mode."""
import argparse
import hashlib
import json
import os
import pathlib
import statistics
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--before", type=pathlib.Path, required=True)
parser.add_argument("--after", type=pathlib.Path, required=True)
parser.add_argument("--output", type=pathlib.Path, required=True)
parser.add_argument("--before-target", type=pathlib.Path)
parser.add_argument("--after-target", type=pathlib.Path)
args = parser.parse_args()
output = args.output.resolve()
output.mkdir(parents=True, exist_ok=True)
worker = pathlib.Path(__file__).resolve().parent / "src/bin/accuracy_refinement.rs"
binaries = {}
identity = {}
for label, repo, target in [("before", args.before.resolve(), args.before_target),
                            ("after", args.after.resolve(), args.after_target)]:
    package = output / label
    package.mkdir(exist_ok=True)
    target = target.resolve() if target else package / "target"
    dependencies = "\n".join(f'{name} = {{ path = {json.dumps(str(repo / "crates" / name))}, features = ["std"] }}'
                             for name in ["exedra_analytic", "exedra_constructive", "exedra_mesh"])
    (package / "Cargo.toml").write_text(f'''[package]
name = "exedra-accuracy-comparison-{label}"
version = "0.0.0"
edition = "2024"
[workspace]
[[bin]]
name = "exedra-accuracy-comparison-{label}"
path = {json.dumps(str(worker))}
[dependencies]
{dependencies}
exedra_profile = {{ path = {json.dumps(str(repo / "crates/exedra_profile"))} }}
serde_json = "=1.0.151"
''')
    subprocess.run(["cargo", "build", "--release", "--offline", "--manifest-path", str(package / "Cargo.toml")],
                   env=dict(os.environ, CARGO_INCREMENTAL="0", CARGO_TARGET_DIR=str(target)), check=True)
    binaries[label] = target / "release" / f"exedra-accuracy-comparison-{label}"
    patch = subprocess.check_output(["git", "diff", "--", "crates/exedra_analytic"], cwd=repo)
    identity[label] = {"head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip(),
                       "analytic_patch_sha256": hashlib.sha256(patch).hexdigest(),
                       "realize_rs_sha256": hashlib.sha256((repo / "crates/exedra_analytic/src/realize.rs").read_bytes()).hexdigest()}
runs = {"before": [], "after": []}
for round in range(3):
    for label in (["before", "after"] if round % 2 == 0 else ["after", "before"]):
        data = json.loads(subprocess.check_output([str(binaries[label])], text=True))
        runs[label].append(data)
        (output / f"{label}-{round}.json").write_text(json.dumps(data, indent=2))
rows = []
for index, before in enumerate(runs["before"][0]["cases"]):
    after = runs["after"][0]["cases"][index]
    assert before["label"] == after["label"]
    if before["label"] != "translated bore":
        assert before["geometry_fingerprint"] == after["geometry_fingerprint"], before["label"]
        assert before["max_deviation"] == after["max_deviation"], before["label"]
    else:
        assert not before["within_tolerance"] and after["within_tolerance"]
    row = {"label": before["label"]}
    for label, metrics in [("before", before), ("after", after)]:
        samples = sorted(sample for run in runs[label] for sample in run["cases"][index]["samples_ns"])
        row[label] = {key: metrics[key] for key in metrics if key != "samples_ns"}
        row[label].update(p50_ns=statistics.median(samples), p95_ns=samples[int(0.95 * (len(samples)-1))])
    rows.append(row)
summary = {"source": identity, "cases": rows, "scope": runs["after"][0]["scope"],
           "method": "same public-API workload, release; three alternating process rounds; 101 measurements after warmup per case per round; geometry fingerprints include f32 positions, native face correspondence, triangle topology and edge sharpness"}
(output / "summary.json").write_text(json.dumps(summary, indent=2))
print(json.dumps(summary, indent=2))
