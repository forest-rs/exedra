# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Matched build123d workloads; no viewer or export framework is imported."""

import argparse
import importlib.metadata
import json
import math
from pathlib import Path
import platform
import sys
import time
sys.dont_write_bytecode = True

IMPORT_START = time.perf_counter_ns()
import build123d as b
from OCP.BRepMesh import BRepMesh_IncrementalMesh
from OCP.BRepTools import BRepTools
from OCP.BRep import BRep_Tool
from OCP.TopAbs import TopAbs_FACE, TopAbs_REVERSED
from OCP.TopExp import TopExp_Explorer
from OCP.TopLoc import TopLoc_Location
from OCP.TopoDS import TopoDS
IMPORT_NS = time.perf_counter_ns() - IMPORT_START

ROOT = Path(__file__).parent
ANGULAR_TOLERANCE = 0.1  # build123d's standard angular constraint, in radians.


def info():
    return {
        "backend": "build123d",
        "version": importlib.metadata.version("build123d"),
        "ocp": importlib.metadata.version("cadquery-ocp-novtk"),
        "python": platform.python_version(),
        "import_ns": IMPORT_NS,
        "absolute_deflection": True,
        "occt_parallel_meshing": True,
        "angular_tolerance": ANGULAR_TOLERANCE,
        "packages": {d.metadata["Name"]: d.version for d in importlib.metadata.distributions()},
    }


def prepare(spec):
    kind = spec["kind"]
    if kind == "revolve":
        # Point tuples are vector-like; nested lists are flattened by build123d.
        wire = b.Polyline(*(tuple(p) for p in spec["points"]), close=True).wire()
        return b.Face(wire), []
    perimeter = b.RectangleRounded(spec["width"], spec["height"], spec["radius"]).wire()
    holes = []
    pockets = []
    if kind == "plate":
        for i in range(spec["holes"]):
            x = 8 * (i // 4 + 1) - spec["width"] / 2
            y = 8 * (i % 4) - 12
            holes.append((b.Pos(x, y) * b.Circle(1.5)).wire())
    else:
        slot = b.SlotOverall(spec["slot_length"], 5)
        holes.extend((b.Pos(-70, -30 + spec["pitch"] * row) * slot).wire()
                     for row in range(spec["vents"]))
        holes.extend((b.Pos(x, y) * b.Circle(radius)).wire()
                     for radius, x, y in [(3.5, 60, -10), (1.5, 45, 35), (1.5, 75, 35)])
        window = b.Pos(-5, 5, spec["thickness"]) * b.RectangleRounded(90, 50, 3)
        pockets.append((window.face(), -2))
        pockets.extend(((b.Pos(x, y) * b.Circle(2.5)).face(), 4)
                       for x, y in [(-88, -50), (88, -50), (-88, 50), (88, 50)])
    # Construct filled holes directly, as recommended by build123d's grid-holes
    # example, rather than paying for one boolean per through-hole.
    return b.Face(perimeter, holes), pockets


def construct(spec, prepared):
    face, pockets = prepared
    if spec["kind"] == "revolve":
        return b.revolve(face, axis=b.Axis.Y, revolution_arc=spec["sweep_degrees"])
    part = b.extrude(face, amount=spec["thickness"])
    if pockets:
        tools = [b.extrude(region, amount=depth) for region, depth in pockets]
        part = part.cut(*tools)  # One supported batch cut for all blind features.
    return part


def mesh(part, tolerance):
    # build123d's public mesh() uses relative=True. Select absolute deflection
    # explicitly to match Exedra's model-length tolerance.
    result = BRepMesh_IncrementalMesh(part.wrapped, tolerance, False, ANGULAR_TOLERANCE, True)
    if not result.IsDone():
        raise RuntimeError("OCCT meshing did not finish")
    return result


def clear_mesh(part):
    BRepTools.Clean_s(part.wrapped)


def extraction_threshold(part, tolerance):
    # OCCT's stored face deflection can exceed the mesher's requested input.
    # Public tessellate() has no read-only mode: its tolerance also decides
    # whether to remesh, using relative deflection. Find a cache acceptance
    # threshold independently; quality is surveyed against the original input.
    for _ in range(32):
        if BRepTools.Triangulation_s(part.wrapped, tolerance):
            return tolerance
        tolerance *= 2
    raise RuntimeError("no complete cached triangulation")


def extract(part, cache_threshold):
    # Never silently enter the relative-deflection branch of public mesh().
    if not BRepTools.Triangulation_s(part.wrapped, cache_threshold):
        raise RuntimeError("absolute triangulation is not reusable at requested tolerance")
    vertices, triangles = part.tessellate(cache_threshold, ANGULAR_TOLERANCE)
    return [[v.X, v.Y, v.Z] for v in vertices], triangles


def extract_direct(part):
    """Isolate OCCT coordinate extraction from build123d Vector/Face wrappers."""
    vertices, triangles = [], []
    faces = TopExp_Explorer(part.wrapped, TopAbs_FACE)
    while faces.More():
        face = TopoDS.Face(faces.Current())
        location = TopLoc_Location()
        poly = BRep_Tool.Triangulation_s(face, location)
        transform = location.Transformation()
        offset = len(vertices)
        reverse = face.Orientation() == TopAbs_REVERSED
        for i in range(1, poly.NbNodes() + 1):
            point = poly.Node(i).Transformed(transform)
            vertices.append([point.X(), point.Y(), point.Z()])
        # Indexed access avoids the binding's exception-heavy iterator path.
        for i in range(1, poly.NbTriangles() + 1):
            triangle = poly.Triangle(i)
            a, c, d = (triangle.Value(i) + offset - 1 for i in (1, 2, 3))
            triangles.append((a, d, c) if reverse else (a, c, d))
        faces.Next()
    return vertices, triangles


def measures(part):
    return [part.volume, part.area]


def measure(repetitions, action, before=None):
    samples = []
    for iteration in range(repetitions + 2):
        if before:
            before()
        start = time.perf_counter_ns()
        result = action()
        elapsed = time.perf_counter_ns() - start
        # Match the Rust worker: stop while the output is alive, drop afterward.
        del result
        if iteration >= 2:
            samples.append(elapsed)
    return samples


def case(spec, tolerance, repetitions):
    prepared = prepare(spec)
    model = construct(spec, prepared)
    if not model.is_valid or len(model.solids()) != 1 or model.volume <= 0:
        raise RuntimeError("comparison requires one valid solid")
    mesh(model, tolerance)
    cache_threshold = extraction_threshold(model, tolerance)
    positions, triangles = extract(model, cache_threshold)
    direct_positions, direct_triangles = extract_direct(model)
    if positions != direct_positions or triangles != direct_triangles:
        raise RuntimeError("public and direct extraction must preserve identical mesh buffers")
    timings = {
        "author": measure(repetitions, lambda: prepare(spec)),
        "construct": measure(repetitions, lambda: construct(spec, prepared)),
        "measures": measure(repetitions, lambda: measures(model)),
        "mesh": measure(repetitions, lambda: mesh(model, tolerance),
                        before=lambda: clear_mesh(model)),
        "extract": measure(repetitions, lambda: extract(model, cache_threshold)),
        "extract_direct": measure(repetitions, lambda: extract_direct(model)),
    }

    def end_to_end(direct=False):
        inputs = prepare(spec)
        part = construct(spec, inputs)
        triangulation = mesh(part, tolerance)
        indexed = extract_direct(part) if direct else extract(part, extraction_threshold(part, tolerance))
        return inputs, part, triangulation, indexed

    timings["end_to_end"] = measure(repetitions, end_to_end)
    timings["end_to_end_direct"] = measure(repetitions, lambda: end_to_end(True))
    return {
        "id": spec["id"], "measures": measures(model), "native_faces": len(model.faces()),
        "public_extraction_cache_threshold_mm": cache_threshold,
        "positions": positions, "triangles": triangles, "timings_ns": timings,
    }


def main():
    global ANGULAR_TOLERANCE
    parser = argparse.ArgumentParser()
    parser.add_argument("--repetitions", type=int, default=9)
    parser.add_argument("--tolerance", type=float, default=0.02)
    parser.add_argument("--case")
    parser.add_argument("--info", action="store_true")
    parser.add_argument("--angular-tolerance", type=float, default=0.1)
    args = parser.parse_args()
    if not math.isfinite(args.angular_tolerance) or not 0 < args.angular_tolerance <= math.pi:
        parser.error("angular tolerance must be in (0, pi]")
    ANGULAR_TOLERANCE = args.angular_tolerance
    if args.info:
        print(json.dumps(info()))
        return
    if args.repetitions <= 0 or not math.isfinite(args.tolerance) or args.tolerance <= 0:
        parser.error("positive repetitions and finite positive tolerance required")
    specs = json.loads((ROOT / "fixtures.json").read_text())
    specs = [s for s in specs if args.case is None or s["id"] == args.case]
    if not specs:
        parser.error("no matching case")
    print(json.dumps({
        **info(), "tolerance": args.tolerance,
        "cases": [case(spec, args.tolerance, args.repetitions) for spec in specs],
    }))


if __name__ == "__main__":
    main()
