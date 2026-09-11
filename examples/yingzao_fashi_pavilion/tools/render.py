# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render the actual GLB. Adds cameras and lights, without changing its meshes/materials."""

from pathlib import Path
import json
import math
import sys

import bpy
from mathutils import Vector


def aim(obj, target):
    obj.rotation_euler = (Vector(target) - obj.location).to_track_quat("-Z", "Y").to_euler()


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    root = Path(args[0] if args else "target/yingzao-fashi-pavilion").resolve()
    case = args[1] if len(args) > 1 else "pavilion"
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    bpy.ops.import_scene.gltf(filepath=str(root / f"{case}.glb"))
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 48
    scene.cycles.use_denoising = True
    scene.render.resolution_x = 1600
    scene.render.resolution_y = 1100
    scene.render.resolution_percentage = 100
    scene.world.use_nodes = True
    background = scene.world.node_tree.nodes["Background"]
    background.inputs["Color"].default_value = (0.55, 0.68, 0.82, 1)
    background.inputs["Strength"].default_value = 0.5
    scene.view_settings.view_transform = "AgX"
    bpy.ops.object.light_add(type="SUN", location=(-4, -6, 9))
    sun = bpy.context.object
    sun.data.energy = 2.5
    sun.data.angle = math.radians(6)
    sun.data.color = (1.0, 0.85, 0.64)
    aim(sun, (0, 0, 0))
    bpy.ops.object.light_add(type="AREA", location=(2, -5, 5))
    fill = bpy.context.object
    fill.data.energy = 450
    fill.data.shape = "DISK"
    fill.data.size = 8
    aim(fill, (0, 0, 2))
    metrics = json.loads((root / "metrics.json").read_text())
    dimensions = next(item for item in metrics if item["name"] == ("pavilion" if case in ("lacquered", "exploded") or case.endswith("-study") else case))
    width = dimensions["bays"] * dimensions["span_mm"] / 1000
    depth = dimensions["depth_mm"] / 1000
    scale = math.sqrt((width + 1.2) / 4.8)
    views = [
        ("eye-level", (8.0 * scale, -11.5 * scale, 1.75), (0, 0, 2.35), 40),
        ("brackets", (width / 2 + 2.3, -depth / 2 - 2.8, 2.4), (width / 2, -depth / 2, 3.42), 48),
        ("roof", (7 * scale, -9 * scale, 5 + 3 * scale), (0, 0, 2.4), 43),
        ("eaves", (width / 2 + 0.6, -depth / 2 - 2.6, 4.7), (width / 2 - 0.6, -depth / 2 - 0.85, 3.98), 62),
    ]
    if case == "exploded":
        views = [("layers", (10 * scale, -15 * scale, 10), (0, 0, 4.2), 38)]
    elif case == "bracket-study":
        views = [("fits", (2.5, -4.5, 3.2), (0, 0, 0.55), 48)]
    elif case == "seat-study":
        views = [("fits", (1.4, -2.3, 1.0), (0, 0, 0.08), 55),
                 ("underside", (1.1, -2.6, -0.65), (0, 0, 0.10), 58)]
    elif case == "concave-study":
        views = [("shoulders", (0.9, -1.7, 1.2), (0, 0, 0.06), 62)]
    elif case == "rafter-study":
        views = [("fits", (2.8, -3.5, 2.2), (0, 0, 0.14), 58),
                 ("underside", (2.8, -3.5, -0.8), (0, 0, 0.10), 58)]
    if len(args) > 2:
        views = [view for view in views if view[0] == args[2]]
        if not views:
            raise ValueError(f"Unknown view: {args[2]}")
    prefix = "" if case == "pavilion" else f"{case}-"
    cameras = []
    for name, location, target, lens in views:
        bpy.ops.object.camera_add(location=location)
        camera = bpy.context.object
        camera.name = name
        camera.data.lens = lens
        cameras.append(camera)
        aim(camera, target)
        scene.camera = camera
        scene.render.filepath = str(root / f"{prefix}{name}.png")
        bpy.ops.render.render(write_still=True)
    scene.camera = cameras[0]
    bpy.ops.wm.save_as_mainfile(filepath=str(root / f"{case}.blend"))


if __name__ == "__main__":
    main()
