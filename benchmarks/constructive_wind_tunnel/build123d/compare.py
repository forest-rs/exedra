# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Capture matched construction, meshing, extraction, startup and process memory."""

import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import platform
import re
import statistics
import subprocess
import sys
import time
sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]


def command_output(command):
    return subprocess.check_output(command, cwd=REPO, text=True).strip()


def capture(command, output):
    stderr = output.with_suffix(".stderr")
    darwin = sys.platform == "darwin"
    timer = ["/usr/bin/time", "-l", "-p"] if darwin else ["/usr/bin/time", "-v"]
    start = time.perf_counter()
    with output.open("w") as out, stderr.open("w") as err:
        process = subprocess.run(timer + command, cwd=REPO, stdout=out, stderr=err)
    elapsed = time.perf_counter() - start
    log = stderr.read_text()
    if process.returncode:
        raise RuntimeError(f"{command[0]} failed; see {stderr}\n{log[-3000:]}")
    pattern = r"(\d+)\s+maximum resident set size" if darwin else r"Maximum resident set size \(kbytes\):\s*(\d+)"
    match = re.search(pattern, log)
    peak = int(match[1]) * (1 if darwin else 1024) if match else None
    return json.loads(output.read_text()), {
        "process_wall_seconds": elapsed,
        "peak_process_rss_bytes": peak,
    }


def rounded_area(width, height, radius):
    return width * height - (4 - math.pi) * radius * radius


def rounded_perimeter(width, height, radius):
    return 2 * (width + height - 4 * radius) + 2 * math.pi * radius


def expected_measures(spec):
    if spec["kind"] == "revolve":
        points = spec["points"]
        crosses = [a[0] * c[1] - c[0] * a[1]
                   for a, c in zip(points, points[1:] + points[:1])]
        area = sum(crosses) / 2
        radial_moment = sum((a[0] + c[0]) * cross
                            for a, c, cross in zip(points, points[1:] + points[:1], crosses)) / 6
        angle = math.radians(spec["sweep_degrees"])
        walls = sum(angle * (a[0] + c[0]) / 2 * math.dist(a, c)
                    for a, c in zip(points, points[1:] + points[:1]))
        return radial_moment * angle, walls + (0 if spec["sweep_degrees"] == 360 else 2 * area)
    area = rounded_area(spec["width"], spec["height"], spec["radius"])
    perimeter = rounded_perimeter(spec["width"], spec["height"], spec["radius"])
    if spec["kind"] == "plate":
        area -= spec["holes"] * math.pi * 1.5**2
        perimeter += spec["holes"] * 2 * math.pi * 1.5
        return area * spec["thickness"], 2 * area + perimeter * spec["thickness"]
    slot_area = 5 * (spec["slot_length"] - 5) + math.pi * 2.5**2
    slot_perimeter = 2 * (spec["slot_length"] - 5) + 5 * math.pi
    area -= spec["vents"] * slot_area + math.pi * (3.5**2 + 2 * 1.5**2)
    perimeter += spec["vents"] * slot_perimeter + 2 * math.pi * (3.5 + 2 * 1.5)
    volume = area * spec["thickness"] - 2 * rounded_area(90, 50, 3) - 4 * 4 * math.pi * 2.5**2
    surface = 2 * area + perimeter * spec["thickness"] + 2 * rounded_perimeter(90, 50, 3) + 80 * math.pi
    return volume, surface


def mesh_evidence(case, expected, tolerance):
    positions, triangles = case["positions"], case["triangles"]
    if not positions or not triangles or not all(math.isfinite(x) for p in positions for x in p):
        raise ValueError("nonempty finite mesh required")
    # OCCT splits vertices at face boundaries. Weld only for this topology check;
    # timing/output positions are untouched. Grid is far below requested error.
    lookup = {}
    welded = []
    for point in positions:
        key = tuple(round(x, 7) for x in point)
        welded.append(lookup.setdefault(key, len(lookup)))
    edges = Counter()
    volume = 0.0
    for triangle in triangles:
        if len(triangle) != 3 or min(triangle) < 0 or max(triangle) >= len(positions):
            raise ValueError("invalid triangle indices")
        ids = [welded[i] for i in triangle]
        if len(set(ids)) != 3:
            raise ValueError("collapsed triangle")
        for a, c in zip(ids, ids[1:] + ids[:1]):
            edges[a, c] += 1
        a, c, d = [positions[i] for i in triangle]
        cross = [c[1]*d[2]-c[2]*d[1], c[2]*d[0]-c[0]*d[2], c[0]*d[1]-c[1]*d[0]]
        volume += sum(x*y for x, y in zip(a, cross)) / 6
    if any(count != 1 or edges[c, a] != 1 for (a, c), count in edges.items()):
        raise ValueError("mesh is not consistently oriented and geometrically closed")
    error = abs(volume - expected[0])
    if error > expected[1] * tolerance * 1.05:
        raise ValueError("mesh volume fails independent broad geometric guard")
    return {
        "output_vertices": len(positions), "welded_vertices": len(lookup),
        "triangles": len(triangles), "native_faces": case["native_faces"],
        "mesh_volume_mm3": volume, "mesh_volume_error_mm3": error,
        "geometrically_closed_after_coordinate_weld": True,
        "reported_deviation_mm": case.get("reported_deviation"),
        "triangle_shape": triangle_shape(case),
    }


def triangle_shape(case, planar_xy_only=False):
    """Angle distribution and longest edge / shortest altitude; outside timers."""
    angles = []
    worst_aspect = 0.0
    for triangle in case["triangles"]:
        a, b, c = [case["positions"][i] for i in triangle]
        if planar_xy_only and not a[2] == b[2] == c[2]:
            continue
        edges = [[q[i] - p[i] for i in range(3)] for p, q in [(a, b), (b, c), (c, a)]]
        lengths2 = [sum(x*x for x in edge) for edge in edges]
        u, v = edges[:2]
        cross = [u[1]*v[2]-u[2]*v[1], u[2]*v[0]-u[0]*v[2], u[0]*v[1]-u[1]*v[0]]
        area2 = math.hypot(*cross)
        if area2 == 0:
            raise ValueError("zero-area triangle")
        corners = [math.atan2(area2, -sum(x*y for x, y in zip(edges[i], edges[(i+1)%3])))
                   for i in range(3)]
        angles.append(math.degrees(min(corners)))
        worst_aspect = max(worst_aspect, max(lengths2) / area2)
    if not angles:
        return None
    angles.sort()
    result = {
        "triangles": len(angles), "minimum_angle_degrees": angles[0],
        "p01_minimum_angle_degrees": angles[max(0, math.ceil(len(angles) / 100) - 1)],
        "triangles_below_1_degree": sum(a < 1 for a in angles),
        "triangles_below_5_degrees": sum(a < 5 for a in angles),
        "triangles_below_10_degrees": sum(a < 10 for a in angles),
        "worst_longest_edge_to_altitude": worst_aspect,
    }
    if not planar_xy_only:
        result["constant_z_triangles"] = triangle_shape(case, planar_xy_only=True)
    return result


def quality_survey(case, boundary, triangle_budget):
    positions, triangles = case["positions"], case["triangles"]
    selected = sorted({i * (len(triangles) - 1) // max(1, triangle_budget - 1)
                       for i in range(min(triangle_budget, len(triangles)))})
    samples = set()
    for index in selected:
        a, c, d = [positions[i] for i in triangles[index]]
        samples.add(tuple((a[j] + c[j] + d[j]) / 3 for j in range(3)))
        for p, q in [(a, c), (c, d), (d, a)]:
            samples.add(tuple((p[j] + q[j]) / 2 for j in range(3)))
    return {
        "survey_points": len(samples),
        "sampled_max_boundary_distance_mm": max(boundary.distance_to(p) for p in samples),
        "certificate": False,
        "method": "deterministic triangle midpoints/centroids against the exact OCCT boundary shells",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust-bin", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--repetitions", type=int, default=9)
    parser.add_argument("--tolerance", type=float, default=0.02)
    parser.add_argument("--native-tolerance", type=float,
                        help="optional distinct native target for an accuracy sensitivity study")
    parser.add_argument("--quality-triangles", type=int, default=64)
    parser.add_argument("--angular-tolerance", type=float, default=0.1)
    parser.add_argument("--case")
    args = parser.parse_args()
    if min(args.rounds, args.repetitions, args.quality_triangles) <= 0:
        parser.error("positive rounds, repetitions and quality budget required")
    if not math.isfinite(args.tolerance) or args.tolerance <= 0:
        parser.error("finite positive tolerance required")
    if args.native_tolerance is not None and (
            not math.isfinite(args.native_tolerance) or args.native_tolerance <= 0):
        parser.error("finite positive native tolerance required")
    tolerance = {"exedra": args.native_tolerance or args.tolerance, "build123d": args.tolerance}
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    rust = str(args.rust_bin.resolve())
    commands = {
        "exedra": [rust],
        "build123d": [sys.executable, str(ROOT / "build123d_worker.py"),
                     "--angular-tolerance", str(args.angular_tolerance)],
    }
    environment = {
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(), "machine": platform.machine(),
        "python": platform.python_version(),
        "rustc": command_output(["rustc", "--version"]),
        "exedra_revision": command_output(["git", "rev-parse", "HEAD"]),
        "worktree_status": command_output(["git", "status", "--short"]),
        "fixtures_sha256": hashlib.sha256((ROOT / "fixtures.json").read_bytes()).hexdigest(),
        "rust_binary_sha256": hashlib.sha256(Path(rust).read_bytes()).hexdigest(),
        "python_worker_sha256": hashlib.sha256((ROOT / "build123d_worker.py").read_bytes()).hexdigest(),
        "tolerance_mm": args.tolerance,
        "native_tolerance_mm": tolerance["exedra"],
        "occt_angular_tolerance_radians": args.angular_tolerance,
        "creation_timers_exclude_output_destruction": True,
        "workers_execute_sequentially": True,
        "warmup_calls_per_phase": 2,
    }
    captures = {name: [] for name in commands}
    resources = {name: [] for name in commands}
    startup = {name: [] for name in commands}
    for round_index in range(args.rounds):
        order = ["exedra", "build123d"] if round_index % 2 == 0 else ["build123d", "exedra"]
        for backend in order:
            metadata, metric = capture(commands[backend] + ["--info"],
                                       output / f"{backend}-startup-{round_index}.json")
            if backend == "exedra" and metadata.get("debug_assertions", True):
                raise ValueError("build the Rust worker with --release before comparing")
            startup[backend].append({**metadata, **metric})
            case_args = ["--case", args.case] if args.case else []
            data, metric = capture(commands[backend] + [
                "--repetitions", str(args.repetitions), "--tolerance", str(tolerance[backend]),
            ] + case_args, output / f"{backend}-round-{round_index}.json")
            captures[backend].append(data)
            resources[backend].append(metric)
            print(f"captured {backend} round {round_index + 1}/{args.rounds}", flush=True)
    # Quality work is deliberately outside all measured worker processes.
    from build123d_worker import b, prepare, construct
    specs = json.loads((ROOT / "fixtures.json").read_text())
    specs = [s for s in specs if args.case is None or s["id"] == args.case]
    cases = []
    for spec in specs:
        expected = expected_measures(spec)
        model = construct(spec, prepare(spec))
        # A shell compound measures distance to the boundary, including points
        # inside the solid. Distance to the Solid itself could incorrectly be zero.
        boundary = b.Compound(model.shells())
        result = {"id": spec["id"], "expected_measures": expected, "backends": {}}
        for backend, runs in captures.items():
            matching = [next(c for c in run["cases"] if c["id"] == spec["id"]) for run in runs]
            first = matching[0]
            for measured, wanted in zip(first["measures"], expected):
                if abs(measured - wanted) > max(1e-7, abs(wanted) * 1e-10):
                    raise ValueError(f"{backend} {spec['id']}: exact measures disagree")
            evidence = mesh_evidence(first, expected, tolerance[backend])
            evidence["all_rounds_geometry"] = [mesh_evidence(c, expected, tolerance[backend]) for c in matching]
            evidence.update(quality_survey(first, boundary, args.quality_triangles))
            evidence["sampled_tolerance_met"] = evidence["sampled_max_boundary_distance_mm"] <= tolerance[backend] + 1e-5
            if not evidence["sampled_tolerance_met"]:
                raise ValueError(f"{backend} {spec['id']}: sampled distance exceeds requested tolerance")
            timings = {}
            for phase in first["timings_ns"]:
                values = [v for c in matching for v in c["timings_ns"][phase]]
                ordered = sorted(values)
                timings[phase] = {
                    "median_ns": statistics.median(values),
                    "p10_ns": ordered[(len(ordered)-1)//10],
                    "p90_ns": ordered[9*(len(ordered)-1)//10],
                    "samples_ns": values,
                }
            result["backends"][backend] = {"geometry": evidence, "timings": timings}
        cases.append(result)
        print(f"validated {spec['id']}", flush=True)
    report = {"environment": environment, "startup": startup,
              "whole_benchmark_process_resources": resources, "cases": cases}
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print("\nmedian milliseconds; output counts and quality are in report.json")
    print("case phase exedra_ms build123d_ms build123d_over_exedra")
    for case in cases:
        for phase in ["author", "construct", "measures", "mesh", "extract", "end_to_end"]:
            a = case["backends"]["exedra"]["timings"][phase]["median_ns"] / 1e6
            c = case["backends"]["build123d"]["timings"][phase]["median_ns"] / 1e6
            ratio = f"{c/a:.2f}" if a > 0 else "below-clock-resolution"
            print(f"{case['id']} {phase} {a:.6f} {c:.6f} {ratio}")


if __name__ == "__main__":
    main()
