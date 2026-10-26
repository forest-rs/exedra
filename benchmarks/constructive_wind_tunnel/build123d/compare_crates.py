# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Compare cap triangulators in an isolated Cargo workspace; optionally capture dhat."""

import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import statistics
import subprocess
import sys

sys.dont_write_bytecode = True
from compare import triangle_shape

ROOT = Path(__file__).resolve().parent


def cap_loops(case):
    points = case["positions"]
    height = min(p[2] for p in points)
    triangles = [t for t in case["triangles"] if all(points[i][2] == height for i in t)]
    edges = Counter((a, b) for t in triangles for a, b in zip(t, t[1:] + t[:1]))
    boundary = {a: b for a, b in edges if (b, a) not in edges}
    loops = []
    while boundary:
        start = min(boundary)
        point = start
        loop = []
        while True:
            loop.append(points[point][:2])
            point = boundary.pop(point)
            if point == start:
                break
        loops.append(loop)
    loops.sort(key=lambda loop: abs(area2(loop)), reverse=True)
    for index, loop in enumerate(loops):
        if (area2(loop) > 0) != (index == 0):
            loop.reverse()
    return loops


def area2(loop):
    return sum(a[0]*b[1] - a[1]*b[0] for a, b in zip(loop, loop[1:] + loop[:1]))


def evidence(capture, loops):
    points = capture["positions"]
    boundary = Counter()
    triangle_area2 = 0.0
    for triangle in capture["triangles"]:
        a, b, c = [points[i] for i in triangle]
        signed_area2 = area2([a, b, c])
        if not math.isfinite(signed_area2) or signed_area2 <= 0:
            raise ValueError("cap contains a degenerate or reversed triangle")
        triangle_area2 += signed_area2
        for left, right in zip(triangle, triangle[1:] + triangle[:1]):
            boundary[tuple(sorted((tuple(points[left][:2]), tuple(points[right][:2]))))] += 1
    source = {tuple(sorted((tuple(a), tuple(b)))) for loop in loops
              for a, b in zip(loop, loop[1:] + loop[:1])}
    emitted = {edge for edge, count in boundary.items() if count == 1}
    if any(count > 2 for count in boundary.values()):
        raise ValueError("nonmanifold cap edge")
    expected = sum(area2(loop) for loop in loops)
    if abs(triangle_area2 - expected) > max(1e-7, expected * 1e-6):
        raise ValueError("cap area mismatch")
    return {
        "input_vertices": sum(map(len, loops)), "output_vertices": len(points),
        "boundary_coordinates_and_edges_identical": source == emitted,
        "absolute_area_error": abs(triangle_area2 - expected) / 2,
        "triangle_shape": triangle_shape(capture),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, required=True,
                        help="Exedra JSON capture from the matched solid worker")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", default="plate_64")
    parser.add_argument("--heap", action="store_true")
    parser.add_argument("--triangulator", type=Path,
                        help="optional exedra_triangulate checkout for a controlled baseline")
    args = parser.parse_args()
    output = args.output.resolve()
    project = output / "worker"
    (project / "src").mkdir(parents=True, exist_ok=True)
    cases = json.loads(args.capture.read_text())["cases"]
    selected = next(c for c in cases if c["id"] == args.case)
    loops = cap_loops(selected)
    fixtures = output / "caps.json"
    fixtures.write_text(json.dumps([{"id": args.case, "loops": loops}]) + "\n")
    repo = ROOT.parents[2]
    triangulator = (args.triangulator or repo / "crates/exedra_triangulate").resolve()
    manifest = f'''[workspace]
[package]
name = "exedra-cap-comparison"
version = "0.0.0"
edition = "2024"
[features]
heap = ["dep:dhat"]
[dependencies]
exedra_triangulate = {{ path = {json.dumps(str(triangulator))} }}
i_triangle = "=0.50.1"
earclip = "=1.8.0"
earcutr = "=0.5.0"
serde_json = {{ version = "=1.0.154", features = ["float_roundtrip"] }}
dhat = {{ version = "=0.3.3", optional = true }}
[profile.release]
debug = 1
'''
    (project / "Cargo.toml").write_text(manifest)
    (project / "src/main.rs").write_text((ROOT / "crate_worker.rs").read_text())
    subprocess.run(["cargo", "build", "--release"], cwd=project, check=True)
    binary = project / "target/release/exedra-cap-comparison"
    results = []
    for backend in ["exedra-ear", "exedra-cdt", "exedra-refine", "earclip", "earcutr",
                    "i-triangle", "i-cdt", "i-refine"]:
        command = [str(binary), backend, args.case, str(fixtures)]
        try:
            result = subprocess.run(command, capture_output=True, text=True, timeout=30, check=True)
            capture = json.loads(result.stdout)
            (output / f"{backend}.json").write_text(result.stdout)
            report = {"backend": backend, "geometry": evidence(capture, loops),
                      "median_ns": statistics.median(capture["timings_ns"]),
                      "samples_ns": capture["timings_ns"]}
            print(backend, report["median_ns"] / 1e6, "ms", flush=True)
        except (subprocess.TimeoutExpired, subprocess.CalledProcessError, ValueError) as error:
            report = {"backend": backend, "failed": str(error)}
        results.append(report)
    if args.heap:
        subprocess.run(["cargo", "build", "--release", "--features", "heap"], cwd=project, check=True)
        subprocess.run([str(binary), "exedra-cdt", args.case, str(fixtures),
                        str(output / "dhat.json")], stdout=subprocess.DEVNULL, check=True)
    source_hash = hashlib.sha256()
    for path in sorted((triangulator / "src").rglob("*.rs")):
        source_hash.update(str(path.relative_to(triangulator)).encode())
        source_hash.update(path.read_bytes())
    report = {"case": args.case, "fixture_sha256": hashlib.sha256(fixtures.read_bytes()).hexdigest(),
              "worker_sha256": hashlib.sha256((ROOT / "crate_worker.rs").read_bytes()).hexdigest(),
              "triangulator_source_sha256": source_hash.hexdigest(),
              "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip(),
              "worktree_status": subprocess.check_output(["git", "status", "--short"], cwd=repo, text=True).strip(),
              "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
              "cargo_lock": (project / "Cargo.lock").read_text(), "results": results}
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
