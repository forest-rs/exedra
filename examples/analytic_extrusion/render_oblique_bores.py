# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Render solid or triangle-overlay bore studies: -- input.glb output.png [wire]."""
import pathlib
import sys
import bpy

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from render_boundary import prepare_scene, render

args = sys.argv[sys.argv.index("--") + 1:]
scene = prepare_scene(args[0])
if len(args) > 2 and args[2] == "wire":
    material = bpy.data.materials.new("display triangle edges")
    material.diffuse_color = (0.008, 0.012, 0.016, 1.0)
    material.use_nodes = True
    material.node_tree.nodes.get("Principled BSDF").inputs["Base Color"].default_value = material.diffuse_color
    for obj in list(scene.objects):
        if obj.type != "MESH" or obj.name == "presentation ground":
            continue
        overlay = obj.copy()
        overlay.data = obj.data.copy()
        scene.collection.objects.link(overlay)
        overlay.data.materials.clear()
        overlay.data.materials.append(material)
        for polygon in overlay.data.polygons:
            polygon.material_index = 0
        wire = overlay.modifiers.new("actual display triangles", "WIREFRAME")
        wire.thickness = 0.00007 / min(abs(v) for v in obj.matrix_world.to_scale())
        wire.offset = 0.0
        wire.use_replace = True
        wire.use_even_offset = True
render(scene, args[1])
