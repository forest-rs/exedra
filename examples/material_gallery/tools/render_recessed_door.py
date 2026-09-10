# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render exported recess geometry and normals without modifying the mesh.

blender --background --python render_recessed_door.py -- target/edge-finishes
"""

from pathlib import Path
import sys

import bpy
from mathutils import Vector


def load(root, name, offset=0):
    before = set(bpy.context.scene.objects)
    bpy.ops.import_scene.gltf(filepath=str(root / f"{name}.glb"))
    for obj in set(bpy.context.scene.objects) - before:
        if obj.parent is None:
            obj.location.x += offset


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    root = Path(args[0] if args else "target/edge-finishes").resolve()
    names = ["door-recess", "door-recess-chamfer", "door-recess-fillet"]
    for view in ["comparison", "chamfer-close", "fillet-close"]:
        bpy.ops.object.select_all(action="SELECT")
        bpy.ops.object.delete(use_global=False)
        close = view != "comparison"
        if close:
            load(root, "door-recess-chamfer" if view == "chamfer-close" else "door-recess-fillet")
            center = Vector((0.389, 0.005, 0.639))
        else:
            for i, name in enumerate(names):
                load(root, name, i * 0.55)
            center = Vector((0.775, 0.012, 0.35))
        scene = bpy.context.scene
        scene.render.engine = "CYCLES"
        scene.cycles.samples = 48
        scene.cycles.use_denoising = True
        scene.render.resolution_x = 1400 if close else 1800
        scene.render.resolution_y = 1000
        scene.render.resolution_percentage = 100
        scene.world.use_nodes = True
        scene.world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.22, 0.22, 0.22, 1)
        scene.world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.6
        scene.view_settings.view_transform = "AgX"
        for offset, energy, size in [((-0.5, -1, 1.2), 110, 0.8), ((0.8, -0.3, 0.5), 75, 0.6)]:
            bpy.ops.object.light_add(type="AREA", location=center + Vector(offset))
            light = bpy.context.object
            light.data.energy, light.data.size = energy, size
            light.rotation_euler = (center - light.location).to_track_quat("-Z", "Y").to_euler()
        offset = Vector((0.075, -0.16, 0.09)) if close else Vector((0.45, -3, 1.1))
        bpy.ops.object.camera_add(location=center + offset)
        camera = bpy.context.object
        camera.rotation_euler = (center - camera.location).to_track_quat("-Z", "Y").to_euler()
        camera.data.type = "ORTHO"
        camera.data.ortho_scale = 0.07 if close else 1.8
        camera.data.clip_start = 0.001
        scene.camera = camera
        scene.render.filepath = str(root / f"recess-{view}.png")
        bpy.ops.render.render(write_still=True)


if __name__ == "__main__":
    main()
