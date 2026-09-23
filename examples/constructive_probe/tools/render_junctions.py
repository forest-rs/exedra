# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Render the junction probe OBJs with Blender's Workbench engine.

    blender --background --python examples/constructive_probe/tools/render_junctions.py -- target/junctions

Writes `<name>.png` beside each OBJ: tubes grey, junction skin green, with
wireframe edges so the quad structure is visible.
"""

import math
import pathlib
import sys

import bpy
from mathutils import Vector

out = pathlib.Path(sys.argv[sys.argv.index("--") + 1])
for obj_path in sorted(out.glob("*.obj")):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.wm.obj_import(
        filepath=str(obj_path), forward_axis="Y", up_axis="Z", use_split_groups=True
    )
    for obj in bpy.context.scene.objects:
        skin = "junction" in obj.name
        obj.color = (0.35, 0.62, 0.38, 1.0) if skin else (0.62, 0.6, 0.57, 1.0)
        modifier = obj.modifiers.new("edges", "WIREFRAME")
        modifier.thickness = 0.006
        modifier.use_replace = False
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.display.shading.color_type = "OBJECT"
    scene.display.shading.show_cavity = True
    scene.display.shading.light = "STUDIO"
    scene.render.resolution_x = 900
    scene.render.resolution_y = 900
    scene.world = bpy.data.worlds.new("world")
    scene.world.color = (0.8, 0.8, 0.8)
    camera = bpy.data.objects.new("camera", bpy.data.cameras.new("camera"))
    scene.collection.objects.link(camera)
    scene.camera = camera
    location = Vector((3.2, -3.6, 1.6))
    camera.location = location
    camera.rotation_euler = (-location).to_track_quat("-Z", "Y").to_euler()
    camera.data.lens = 50
    scene.render.filepath = str(obj_path.with_suffix(".png"))
    bpy.ops.render.render(write_still=True)
