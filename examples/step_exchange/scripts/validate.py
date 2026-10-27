#!/usr/bin/env python3
# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Independent OCCT oracle. Install cadquery-ocp separately; exit 77 if unavailable."""
import json
import math
import pathlib
import sys

try:
    from OCP.BRepBndLib import BRepBndLib
    from OCP.BRepCheck import BRepCheck_Analyzer
    from OCP.BRepGProp import BRepGProp
    from OCP.Bnd import Bnd_Box
    from OCP.GProp import GProp_GProps
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.STEPControl import STEPControl_Reader
    from OCP.TopAbs import TopAbs_EDGE, TopAbs_FACE, TopAbs_SHELL, TopAbs_SOLID, TopAbs_VERTEX
    from OCP.TopExp import TopExp_Explorer
except ImportError as error:
    print(json.dumps({"status": "unavailable", "reason": str(error)}))
    sys.exit(77)


def checks(iterator):
    result = []
    iterator.Start()
    while iterator.More():
        check = iterator.Value()
        for label, count, message in [("failure", check.NbFails(), check.CFail),
                                       ("warning", check.NbWarnings(), check.CWarning)]:
            for i in range(1, count + 1):
                result.append({"entity": iterator.Number(), "kind": label, "message": str(message(i))})
        iterator.Next()
    return result


def count(shape, kind):
    explorer = TopExp_Explorer(shape, kind)
    unique = []
    while explorer.More():
        current = explorer.Current()
        if not any(current.IsSame(previous) for previous in unique):
            unique.append(current)
        explorer.Next()
    return len(unique)


def close(actual, expected, absolute=1e-7):
    assert math.isfinite(actual) and math.isclose(actual, expected, rel_tol=2e-10, abs_tol=absolute), (actual, expected)


def validate(folder, fixture):
    reader = STEPControl_Reader()
    assert reader.ReadFile(str(folder / fixture["file"])) == IFSelect_RetDone
    model_checks = checks(reader.WS().ModelCheckList(False))
    assert not model_checks, model_checks
    assert reader.TransferRoots() == 1
    transfer_checks = checks(reader.WS().TransferReader().TransientProcess().CheckList(False))
    assert not transfer_checks, transfer_checks
    shape = reader.OneShape()
    assert not shape.IsNull()
    assert BRepCheck_Analyzer(shape).IsValid(), "invalid imported boundary"
    assert count(shape, TopAbs_SOLID) == 1
    assert count(shape, TopAbs_SHELL) == 1
    for key, kind in [("faces", TopAbs_FACE), ("edges", TopAbs_EDGE), ("vertices", TopAbs_VERTEX)]:
        assert count(shape, kind) == fixture[key], (key, count(shape, kind), fixture[key])
    # OCCT's STEP reader uses millimetres; metres must scale all measures.
    scale = fixture["metres_per_unit"] * 1000.0
    volume = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, volume)
    close(volume.Mass(), fixture["native_volume"] * scale**3)
    area = GProp_GProps()
    BRepGProp.SurfaceProperties_s(shape, area)
    close(area.Mass(), fixture["native_area"] * scale**2)
    box = Bnd_Box()
    BRepBndLib.AddOptimal_s(shape, box, False, False)
    actual_bounds = [[p.X(), p.Y(), p.Z()] for p in [box.CornerMin(), box.CornerMax()]]
    for actual, expected in zip(actual_bounds, fixture["bounds"]):
        for value, source in zip(actual, expected):
            close(value, source * scale)
    assert fixture["max_deviation"] <= fixture["uncertainty"]
    return {"file": fixture["file"], "valid": True, "solids": 1, "shells": 1,
            "faces": fixture["faces"], "edges": fixture["edges"], "vertices": fixture["vertices"],
            "volume_mm3": volume.Mass(), "area_mm2": area.Mass(), "bounds_mm": actual_bounds,
            "model_checks": model_checks, "transfer_checks": transfer_checks}


folder = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "step-fixtures")
try:
    fixtures = json.loads((folder / "expected.json").read_text())
    report = {"status": "passed", "reader": "OCCT via cadquery-ocp", "fixtures": [validate(folder, f) for f in fixtures]}
except Exception as error:
    print(json.dumps({"status": "failed", "reason": str(error)}))
    raise
print(json.dumps(report, indent=2))
