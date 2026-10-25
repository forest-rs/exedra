# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Compare source mesh edges: -- comparison.glb topology.json output.png [--wire]."""
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
import bpy
from mathutils import Vector
sys.path.insert(0, str(Path(__file__).parent))
from render_boundary import prepare_scene, render

args = sys.argv[sys.argv.index('--') + 1:]
if len(args) not in (3, 4):
    raise SystemExit('Pass -- comparison.glb topology.json output.png [--wire]')
wire = len(args) == 4 and args[3] == '--wire'
scene = prepare_scene(args[0])
records = json.loads(Path(args[1]).read_text())
if wire:
    body = bpy.data.materials.new('neutral comparison surface')
    body.diffuse_color = (0.42, 0.58, 0.65, 1)
    body.use_nodes = True
    body.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value = (0.42, 0.58, 0.65, 1)
    body.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value = 0.7
    for obj in scene.objects:
        if obj.type == 'MESH' and obj.name != 'presentation ground':
            obj.data.materials.clear()
            obj.data.materials.append(body)
    ink = bpy.data.materials.new('source topology edges')
    ink.diffuse_color = (0.006, 0.012, 0.018, 1)
    ink.use_nodes = True
    ink.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value = (0.002, 0.004, 0.006, 1)
    ink.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value = 0.8
    for record in records:
        data = bpy.data.curves.new(record['name'] + ' source edges', 'CURVE')
        data.dimensions = '3D'
        data.resolution_u = 1
        data.bevel_depth = 0.000065
        data.bevel_resolution = 0
        for a, b in record['edges']:
            segment = data.splines.new('POLY')
            segment.points.add(1)
            for point, index in zip(segment.points, (a, b)):
                point.co = tuple(v * 0.001 for v in record['positions_mm'][index]) + (1,)
        obj = bpy.data.objects.new(data.name, data)
        scene.collection.objects.link(obj)
        data.materials.append(ink)
# Labels are placed in camera coordinates, after fitting only the model geometry.
cam = scene.camera
bpy.context.view_layer.update()
height = cam.data.ortho_scale / 1.8
text_material = bpy.data.materials.new('comparison captions')
text_material.diffuse_color = (0.015, 0.025, 0.04, 1)
text_material.use_nodes = True
text_material.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value = (0.015, 0.025, 0.04, 1)
for record in records:
    points = [cam.matrix_world.inverted() @ Vector(tuple(v * 0.001 for v in p)) for p in record['positions_mm']]
    x = (min(p.x for p in points) + max(p.x for p in points)) * 0.5
    label = record['name'] + '\n'
    if record.get('quad_faces', 0):
        label += f"{record['quad_faces']} quads + {record['triangular_faces']} cap triangles"
    else:
        label += f"{record['polygon_faces']} triangle faces"
    label += f"\n{record['triangles']} final triangles / {record['angular_intervals']} angular intervals"
    bpy.ops.object.text_add(location=cam.matrix_world @ Vector((x, -height * 0.47, -0.15)))
    text = bpy.context.object
    text.rotation_euler = cam.rotation_euler
    text.data.body = label
    text.data.align_x = 'CENTER'
    text.data.align_y = 'BOTTOM_BASELINE'
    text.data.size = cam.data.ortho_scale * 0.014
    text.data.space_line = 1.1
    text.data.materials.append(text_material)
render(scene, args[2])
