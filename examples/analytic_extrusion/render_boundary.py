# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render an analytic study GLB with Blender: -- input.glb output.png."""
import sys
import bpy
from mathutils import Vector


def prepare_scene(path):
    """Import geometry and frame its projected bounds with even margins."""
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)
    bpy.ops.import_scene.gltf(filepath=path)
    bounds = [obj.matrix_world @ Vector(corner) for obj in bpy.context.scene.objects
              if obj.type == 'MESH' for corner in obj.bound_box]
    lower = Vector(tuple(min(p[i] for p in bounds) for i in range(3)))
    upper = Vector(tuple(max(p[i] for p in bounds) for i in range(3)))
    target = (lower + upper) * 0.5
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.cycles.samples = 64
    scene.render.resolution_x = 1800
    scene.render.resolution_y = 1000
    scene.render.resolution_percentage = 100
    scene.world.color = (0.28, 0.28, 0.28)
    bpy.ops.object.camera_add(location=target + Vector((0.10, -0.22, 0.15)))
    cam = bpy.context.object
    cam.rotation_euler = (target - cam.location).to_track_quat('-Z', 'Y').to_euler()
    cam.data.type = 'ORTHO'
    # Model units are meters after GLB import; Blender's 0.1 m default clips
    # the foreground of small studies and exposes the black world background.
    cam.data.clip_start = max((upper - lower).length * 1e-5, 1e-6)
    bpy.context.view_layer.update()
    projected = [cam.matrix_world.inverted() @ p for p in bounds]
    left, right = min(p.x for p in projected), max(p.x for p in projected)
    bottom, top = min(p.y for p in projected), max(p.y for p in projected)
    cam.location += cam.rotation_euler.to_matrix() @ Vector(((left + right) * 0.5, (bottom + top) * 0.5, 0))
    cam.data.ortho_scale = max(right - left, (top - bottom) * 1.8) * 1.18
    scene.camera = cam
    bpy.ops.mesh.primitive_plane_add(size=2000, location=(0, 0, -0.0003))
    floor = bpy.context.object
    floor.name = 'presentation ground'
    mat = bpy.data.materials.new('background')
    mat.diffuse_color = (0.14, 0.17, 0.20, 1)
    floor.data.materials.append(mat)
    for pos, power, size in [((0, -0.08, 0.25), 15, 0.20),
                             ((-0.15, 0.08, 0.15), 8, 0.15),
                             ((0.15, 0.10, 0.12), 6, 0.10)]:
        bpy.ops.object.light_add(type='AREA', location=pos)
        light = bpy.context.object
        light.data.energy = power
        light.data.shape = 'DISK'
        light.data.size = size
        light.rotation_euler = (target - light.location).to_track_quat('-Z', 'Y').to_euler()
    scene.view_settings.view_transform = 'AgX'
    scene.view_settings.exposure = -4
    return scene


def render(scene, path):
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)


if __name__ == '__main__':
    args = sys.argv[sys.argv.index('--') + 1:]
    if len(args) != 2:
        raise SystemExit('Pass -- input.glb output.png')
    render(prepare_scene(args[0]), args[1])
