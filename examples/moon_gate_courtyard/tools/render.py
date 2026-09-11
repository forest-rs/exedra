# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Light and photograph the exported courtyard GLB without altering its assets."""
from pathlib import Path
import math
import sys
import bpy
from mathutils import Vector


def aim(obj, target):
    obj.rotation_euler = (Vector(target) - obj.location).to_track_quat("-Z", "Y").to_euler()


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    root = Path(args[0] if args else "target/moon-gate-courtyard").resolve()
    requested_views = args[1:] or ["gate"]
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    bpy.ops.import_scene.gltf(filepath=str(root / "courtyard.glb"))
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 48
    scene.cycles.use_denoising = True
    scene.render.resolution_x = 1600
    scene.render.resolution_y = 1200
    scene.render.resolution_percentage = 100
    scene.world.use_nodes = True
    background = scene.world.node_tree.nodes["Background"]
    background.inputs["Color"].default_value = (0.64, 0.74, 0.88, 1)
    background.inputs["Strength"].default_value = 0.45
    scene.view_settings.view_transform = "AgX"
    bpy.ops.object.light_add(type="SUN", location=(-5, -4, 8))
    sun = bpy.context.object
    sun.data.energy = 2.6
    sun.data.angle = math.radians(4)
    sun.data.color = (1.0, 0.88, 0.70)
    aim(sun, (0, 4, 0))
    bpy.ops.object.light_add(type="AREA", location=(0, -3, 6))
    light = bpy.context.object
    light.data.energy = 300
    light.data.shape = "DISK"
    light.data.size = 8
    aim(light, (0, 3, 1))
    views = {
        "gate": ((-0.6, -4.9, 1.65), (0.35, 3.8, 1.7), 32),
        "garden": ((-1.6, 1.0, 1.65), (1.35, 5.4, 1.5), 32),
        "overview": ((11, -13, 10), (0, 4.0, 0.8), 42),
        "wall-cap": ((1.2, -1.6, 5.0), (0.0, 0.21, 3.76), 48),
        "wall-corner": ((7.4, 11.9, 4.7), (5.3, 9.7, 2.98), 50),
        "masonry": ((-2.5, -2.2, 1.8), (-0.9, 0.15, 1.3), 40),
    }
    for view in requested_views:
        location, target, lens = views[view]
        bpy.ops.object.camera_add(location=location)
        camera = bpy.context.object
        camera.name = view
        camera.data.lens = lens
        aim(camera, target)
        scene.camera = camera
        scene.render.filepath = str(root / f"{view}.png")
        bpy.ops.render.render(write_still=True)
    bpy.ops.wm.save_as_mainfile(filepath=str(root / "courtyard.blend"))


if __name__ == "__main__":
    main()
