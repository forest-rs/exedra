#!/usr/bin/env python3
# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Independent OCCT oracle. Install cadquery-ocp separately; exit 77 if unavailable."""
import json
import importlib.metadata
import math
import pathlib
import re
import sys

try:
    from OCP.BRepAdaptor import BRepAdaptor_Surface
    from OCP.GeomAbs import GeomAbs_Plane, GeomAbs_Cylinder, GeomAbs_Cone, GeomAbs_Torus, GeomAbs_Sphere
    from OCP.TopoDS import TopoDS
    from OCP.Precision import Precision
    from OCP.BRepClass3d import BRepClass3d_SolidClassifier
    from OCP.gp import gp_Pnt
    from OCP.BRepBndLib import BRepBndLib
    from OCP.BndLib import BndLib_AddSurface
    from OCP.BRepTools import BRepTools
    from OCP.BRepCheck import BRepCheck_Analyzer
    from OCP.BRepGProp import BRepGProp
    from OCP.Bnd import Bnd_Box
    from OCP.GProp import GProp_GProps
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.STEPControl import STEPControl_Reader
    from OCP.StepBasic import StepBasic_Product
    from OCP.TopAbs import TopAbs_IN, TopAbs_OUT, TopAbs_EDGE, TopAbs_FACE, TopAbs_SHELL, TopAbs_SOLID, TopAbs_VERTEX
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
    text = (folder / fixture["file"]).read_text()
    serialized = {
        "edges":len(re.findall(r"#\d+\s*=\s*EDGE_CURVE\(",text)),
        "vertices":len(re.findall(r"#\d+\s*=\s*VERTEX_POINT\(",text)),
        "coedges":len(re.findall(r"#\d+\s*=\s*ORIENTED_EDGE\(",text)),
        "loops":len(re.findall(r"#\d+\s*=\s*(?:EDGE_LOOP|VERTEX_LOOP)\(",text)),
    }
    for key,value in serialized.items():
        if "serialized_"+key in fixture:
            assert value == fixture["serialized_"+key], (key,value,fixture["serialized_"+key])
    reader = STEPControl_Reader()
    assert reader.ReadFile(str(folder / fixture["file"])) == IFSelect_RetDone
    reader.SetSystemLengthUnit(1.0)
    model_checks = checks(reader.WS().ModelCheckList(False))
    assert not model_checks, model_checks
    model = reader.Model()
    products = [model.Value(i) for i in range(1, model.NbEntities() + 1)
                if isinstance(model.Value(i), StepBasic_Product)]
    assert len(products) == 1
    product_name = products[0].Name().ToCString()
    assert product_name == fixture.get("product_name", pathlib.Path(fixture["file"]).stem)
    assert reader.TransferRoots() == 1
    transfer_checks = checks(reader.WS().TransferReader().TransientProcess().CheckList(False))
    assert not transfer_checks, transfer_checks
    shape = reader.OneShape()
    assert not shape.IsNull()
    assert BRepCheck_Analyzer(shape, True, False, True).IsValid(), "invalid imported boundary"
    assert count(shape, TopAbs_SOLID) == 1
    assert count(shape, TopAbs_SHELL) == fixture.get("shells", 1)
    for key, kind in [("faces", TopAbs_FACE), ("edges", TopAbs_EDGE), ("vertices", TopAbs_VERTEX)]:
        assert count(shape, kind) == fixture[key], (key, count(shape, kind), fixture[key])
    # OCCT's STEP reader uses millimetres; metres must scale all measures.
    scale = fixture["metres_per_unit"] * 1000.0
    volume = GProp_GProps()
    # Request adaptive integration explicitly: the default non-adaptive overload
    # can miss this harness's tolerance on oblique elliptic openings.
    volume_integration_error = BRepGProp.VolumeProperties_s(shape, volume, 1e-12)
    expected_volume = fixture["native_volume"] * scale**3
    close(volume.Mass(), expected_volume, absolute=0.0)
    area = GProp_GProps()
    area_integration_error = BRepGProp.SurfaceProperties_s(shape, area, 1e-12)
    expected_area = fixture["native_area"] * scale**2
    close(area.Mass(), expected_area, absolute=0.0)
    centroid = volume.CentreOfMass().Coord()
    centroid_error = None
    if "centroid" in fixture:
        centroid_error = max(abs(actual-source*scale) for actual,source in zip(centroid,fixture["centroid"]))
        assert centroid_error <= fixture["max_deviation"]*scale, ("centroid",centroid,fixture["centroid"],centroid_error,fixture["max_deviation"]*scale)
    box = Bnd_Box()
    BRepBndLib.AddOptimal_s(shape, box, False, False)
    actual_bounds = [[p.X(), p.Y(), p.Z()] for p in [box.CornerMin(), box.CornerMax()]]
    shape_bounds = actual_bounds
    bounds_method = fixture.get("bounds_method", "shape")
    if bounds_method == "analytic_cap_domains":
        assert fixture["authoring"]["constructor"] == "Revolution::spherical_cap"
        assert fixture["authoring"]["frame"] == [[1,0,0,0],[0,1,0,0],[0,0,1,0]]
        # The whole-shape bounding path can expand a large meridian by a
        # fixed parameter allowance. Bound these canonical cap faces directly
        # from the actual reader support and trim-domain limits, at zero added
        # tolerance; retain the raw whole-shape box as separate evidence.
        box = Bnd_Box()
        surfaces = TopExp_Explorer(shape, TopAbs_FACE)
        cast_face = getattr(TopoDS, "Face_s", None) or TopoDS.Face
        kinds = []
        while surfaces.More():
            face = cast_face(surfaces.Current())
            support = BRepAdaptor_Surface(face)
            kinds.append(support.GetType())
            assert support.GetType() in (GeomAbs_Plane, GeomAbs_Sphere)
            uv = BRepTools.UVBounds_s(face)
            if support.GetType() == GeomAbs_Sphere:
                assert abs(uv[1]-uv[0]-math.tau) <= 1e-10
                assert 0.0 <= uv[2] < uv[3] <= math.pi/2 + 1e-12
            BndLib_AddSurface.AddOptimal_s(support, *uv, 0.0, box)
            surfaces.Next()
        assert sorted(map(str, kinds)) == sorted(map(str, [GeomAbs_Plane, GeomAbs_Sphere]))
        actual_bounds = [[p.X(), p.Y(), p.Z()] for p in [box.CornerMin(), box.CornerMax()]]
    else:
        assert bounds_method == "shape"
    # AddOptimal uses OCCT's own confusion floor for extrema. Allow its
    # outward box padding plus comparison rounding, separately from geometry.
    confusion = Precision.Confusion() if hasattr(Precision, "Confusion") else Precision.Confusion_s()
    bounds_comparison_tolerance = 2.0 * confusion
    for actual, expected in zip(actual_bounds, fixture["bounds"]):
        for value, source in zip(actual, expected):
            close(value, source * scale, absolute=bounds_comparison_tolerance)
    surface_counts = {}
    surfaces = TopExp_Explorer(shape, TopAbs_FACE)
    cast_face = getattr(TopoDS, "Face_s", None) or TopoDS.Face
    while surfaces.More():
        support = BRepAdaptor_Surface(cast_face(surfaces.Current()))
        name = {GeomAbs_Plane:"plane", GeomAbs_Cylinder:"cylinder", GeomAbs_Cone:"cone", GeomAbs_Torus:"torus", GeomAbs_Sphere:"sphere"}.get(support.GetType(),"other")
        surface_counts[name] = surface_counts.get(name, 0) + 1
        if name == "cone":
            assert 0.0 < support.Cone().SemiAngle() < math.pi/2, "canonical cone angle survives transfer"
        surfaces.Next()
    if "surface_counts" in fixture:
        assert surface_counts == fixture["surface_counts"], (surface_counts,fixture["surface_counts"])
    witnesses = []
    for witness in fixture.get("witnesses", []):
        point = [v * scale for v in witness["point"]]
        classifier = BRepClass3d_SolidClassifier(shape, gp_Pnt(*point), fixture["uncertainty"] * scale)
        expected_state = {"inside": TopAbs_IN, "outside": TopAbs_OUT}[witness["state"]]
        assert classifier.State() == expected_state, (witness, classifier.State())
        witnesses.append({"point_mm": point, "state": witness["state"]})
    assert fixture["max_deviation"] <= fixture["uncertainty"]
    return {"file": fixture["file"], "valid": True, "solids": 1, "shells": fixture.get("shells", 1),
            "faces": fixture["faces"], "edges": fixture["edges"], "vertices": fixture["vertices"],
            "volume_mm3": volume.Mass(), "area_mm2": area.Mass(), "volume_integration_error_estimate":volume_integration_error, "area_integration_error_estimate":area_integration_error,
            "volume_relative_error":abs(volume.Mass()/expected_volume-1.0),"area_relative_error":abs(area.Mass()/expected_area-1.0),
            "centroid_mm":list(centroid),"centroid_error_mm":centroid_error,"export_deviation_mm":fixture["max_deviation"]*scale,"native_support_gap_mm":(fixture["max_native_support_gap"]*scale if "max_native_support_gap" in fixture else None),"frame_roundoff_bound_mm":(fixture["frame_roundoff_bound"]*scale if "frame_roundoff_bound" in fixture else None),"parameter_roundoff_bound_mm":(fixture["parameter_roundoff_bound"]*scale if "parameter_roundoff_bound" in fixture else None),"bounds_mm": actual_bounds,"bounds_method":bounds_method,"raw_shape_bounds_mm":shape_bounds,
            "product_name": product_name, "serialized_topology":serialized, "surface_counts":surface_counts, "bounds_comparison_tolerance_mm":bounds_comparison_tolerance, "model_checks": model_checks, "transfer_checks": transfer_checks, "material_witnesses": witnesses}


folder = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "step-fixtures")
try:
    fixtures = json.loads((folder / "expected.json").read_text())
    for reader_package in ("cadquery-ocp", "cadquery-ocp-novtk"):
        try:
            reader_version = importlib.metadata.version(reader_package)
            break
        except importlib.metadata.PackageNotFoundError:
            continue
    else:
        raise RuntimeError("OCP imported, but its installed distribution was not found")
    report = {"status": "passed", "reader": "OCCT via " + reader_package, "reader_package": reader_package, "reader_package_version": reader_version, "fixtures": [validate(folder, f) for f in fixtures]}
except Exception as error:
    print(json.dumps({"status": "failed", "reason": str(error)}))
    raise
print(json.dumps(report, indent=2))
