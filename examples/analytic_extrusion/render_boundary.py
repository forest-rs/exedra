# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render the boundary_geometry GLB with Blender: -- input.glb output.png."""
import sys
import bpy
from mathutils import Vector
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)
args = sys.argv[sys.argv.index('--') + 1:]
if len(args) != 2:
    raise SystemExit('Pass -- input.glb output.png')
bpy.ops.import_scene.gltf(filepath=args[0])
# Root is in meters, as required by glTF.
scene=bpy.context.scene
scene.render.engine='CYCLES'
scene.cycles.samples=64
scene.render.resolution_x=1800
scene.render.resolution_y=900
scene.render.resolution_percentage=100
scene.world.color=(0.28,0.28,0.28)
bpy.ops.mesh.primitive_plane_add(size=2000, location=(0,0,-0.0003))
floor=bpy.context.object
mat=bpy.data.materials.new('background')
mat.diffuse_color=(0.14,0.17,0.20,1)
floor.data.materials.append(mat)
bpy.ops.object.camera_add(location=(0.10,-0.22,0.17))
cam=bpy.context.object
cam.rotation_euler=(Vector((0,0,0.01))-cam.location).to_track_quat('-Z','Y').to_euler()
cam.data.type='ORTHO'
cam.data.ortho_scale=0.205
scene.camera=cam
for pos,power,size in [((0,-0.08,0.25),15,0.20), ((-0.15,0.08,0.15),8,0.15), ((0.15,0.10,0.12),6,0.10)]:
    bpy.ops.object.light_add(type='AREA',location=pos)
    light=bpy.context.object
    light.data.energy=power
    light.data.shape='DISK'
    light.data.size=size
    light.rotation_euler=(Vector((0,0,0))-light.location).to_track_quat('-Z','Y').to_euler()
scene.view_settings.view_transform='AgX'
scene.view_settings.exposure=-4
scene.render.filepath=args[1]
bpy.ops.render.render(write_still=True)
