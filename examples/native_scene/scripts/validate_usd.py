#!/usr/bin/env python3
# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Independently inspect the example's saved layers with existing OpenUSD bindings."""
import json
import math
from pathlib import Path
import sys

try:
    from pxr import Gf, Usd, UsdGeom, UsdShade
except ImportError as error:
    print(json.dumps({"status": "unavailable", "reason": str(error)}))
    sys.exit(77)

folder = Path(sys.argv[1] if len(sys.argv) > 1 else "native-scene-output")
try:
    expected = json.loads((folder / "work-report.json").read_text())
    stage = Usd.Stage.Open(str(folder / "scene.usda"))
    assert stage and not stage.GetCompositionErrors()
    assert stage.GetDefaultPrim().GetPath() == "/World"
    assert UsdGeom.GetStageUpAxis(stage) == "Z"
    assert UsdGeom.GetStageMetersPerUnit(stage) == 0.001
    asset = stage.GetPrimAtPath(expected["publishedSource"])
    assert asset.IsA(UsdGeom.Boundable), "custom source has a standard fallback prim type"
    mesh = UsdGeom.Mesh(stage.GetPrimAtPath(str(asset.GetPath()) + "/__exedra_geometry"))
    assert mesh
    points = mesh.GetPointsAttr().Get()
    counts = mesh.GetFaceVertexCountsAttr().Get()
    indices = mesh.GetFaceVertexIndicesAttr().Get()
    valid, reason = UsdGeom.Mesh.ValidateTopology(indices, counts, len(points))
    assert valid, reason
    assert all(count >= 3 for count in counts)
    assert all(math.isfinite(float(c)) for point in points for c in point)
    assert mesh.GetSubdivisionSchemeAttr().Get() == "none"
    actual_extent = mesh.GetExtentAttr().Get()
    calculated = UsdGeom.Mesh.ComputeExtent(points)
    assert actual_extent == calculated
    parent_extent = UsdGeom.Boundable(asset).GetExtentAttr().Get()
    for axis in range(3):
        assert parent_extent[0][axis] <= expected["nativeBounds"][0][axis]
        assert parent_extent[1][axis] >= expected["nativeBounds"][1][axis]
        assert parent_extent[0][axis] <= actual_extent[0][axis]
        assert parent_extent[1][axis] >= actual_extent[1][axis]
    primvars = UsdGeom.PrimvarsAPI(mesh)
    normals = primvars.GetPrimvar("normals")
    uv = primvars.GetPrimvar("st")
    native_faces = primvars.GetPrimvar("exedra:nativeFace")
    assert normals.GetInterpolation() == "faceVarying" and len(normals.Get()) == sum(counts)
    if uv:
        assert uv.GetInterpolation() == "faceVarying" and len(uv.Get()) == sum(counts)
    assert native_faces.GetInterpolation() == "uniform" and len(native_faces.Get()) == len(counts)
    assert all(0 <= value < 31 for value in native_faces.Get()), "native IDs address this generator's 31-face panel"
    material, _ = UsdShade.MaterialBindingAPI(mesh.GetPrim()).ComputeBoundMaterial()
    assert material and str(material.GetPath()).endswith("/Looks/Gold")
    shader = UsdShade.Shader(stage.GetPrimAtPath(str(asset.GetPath()) + "/Looks/Gold/Surface"))
    assert shader.GetIdAttr().Get() == "UsdPreviewSurface"
    placements = stage.GetPrimAtPath("/World/Placements").GetChildren()
    assert len(placements) == expected["instances"] and all(p.IsInstance() for p in placements)
    if placements:
        first = stage.GetPrimAtPath("/World/Placements/P0000")
        world = UsdGeom.Xformable(first).ComputeLocalToWorldTransform(Usd.TimeCode.Default())
        assert world.ExtractTranslation() == Gf.Vec3d(200, 20, 0)
        proxy = stage.GetPrimAtPath(str(first.GetPath()) + "/__exedra_geometry")
        assert proxy.IsInstanceProxy() and UsdGeom.Mesh(proxy).GetPointsAttr().Get() == points
        proxy_material, _ = UsdShade.MaterialBindingAPI(proxy).ComputeBoundMaterial()
        assert proxy_material and str(proxy_material.GetPath()).endswith("/Looks/Gold")
    result = {"status": "passed", "reader": "OpenUSD", "readerVersion": list(Usd.GetVersion()),
              "instances": len(placements), "pointsPerSource": len(points), "polygonsPerSource": len(counts),
              "nativeFaces": len(set(native_faces.Get())), "uvAuthored": bool(uv), "units": "mm", "upAxis": "Z",
              "sourceExtent": [list(p) for p in parent_extent], "meshExtent": [list(p) for p in actual_extent],
              "material": str(material.GetPath()), "compositionErrors": [],
              "checks": ["native source fallback", "topology", "finite points", "normals/UV/native face cardinality",
                         "source enclosure", "actual mesh extent", "native instancing", "placed proxy geometry", "material/shader"]}
except Exception as error:
    print(json.dumps({"status": "failed", "reason": str(error)}))
    raise
print(json.dumps(result, indent=2))
