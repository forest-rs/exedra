# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Render the junction probe OBJs with Blender's Workbench engine.

    blender --background --python examples/constructive_probe/tools/render_junctions.py -- target/junctions

Writes two images beside each OBJ:

- `<name>.png`: flat shading, tubes grey and junction skin green, with
  wireframe edges so the face structure is visible;
- `<name>-textured.png`: smooth shading with a checker texture over the OBJ's
  UVs, so shading creases and texture seams are visible.
"""

import pathlib
import sys

import bpy
from mathutils import Vector


def setup_scene(obj_path, suffix):
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
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
    scene.render.filepath = str(obj_path.with_name(obj_path.stem + suffix + ".png"))
    bpy.ops.render.render(write_still=True)


def load(obj_path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.wm.obj_import(
        filepath=str(obj_path), forward_axis="Y", up_axis="Z", use_split_groups=True
    )
    return list(bpy.context.scene.objects)


def render_structure(obj_path):
    for obj in load(obj_path):
        skin = "junction" in obj.name
        obj.color = (0.35, 0.62, 0.38, 1.0) if skin else (0.62, 0.6, 0.57, 1.0)
        modifier = obj.modifiers.new("edges", "WIREFRAME")
        modifier.thickness = 0.006
        modifier.use_replace = False
    bpy.context.scene.display.shading.color_type = "OBJECT"
    setup_scene(obj_path, "")


def render_textured(obj_path):
    objects = load(obj_path)
    # Join tube and skin so smooth shading averages normals across the rings.
    bpy.ops.object.select_all(action="SELECT")
    bpy.context.view_layer.objects.active = objects[0]
    if len(objects) > 1:
        bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.remove_doubles(threshold=1e-6)
    bpy.ops.object.mode_set(mode="OBJECT")
    for polygon in obj.data.polygons:
        polygon.use_smooth = True
    image = bpy.data.images.new("checker", 1024, 1024)
    image.generated_type = "COLOR_GRID"
    material = bpy.data.materials.new("checker")
    material.use_nodes = True
    texture = material.node_tree.nodes.new("ShaderNodeTexImage")
    texture.image = image
    bsdf = material.node_tree.nodes["Principled BSDF"]
    material.node_tree.links.new(texture.outputs["Color"], bsdf.inputs["Base Color"])
    obj.data.materials.clear()
    obj.data.materials.append(material)
    bpy.context.scene.display.shading.color_type = "TEXTURE"
    setup_scene(obj_path, "-textured")


out = pathlib.Path(sys.argv[sys.argv.index("--") + 1])
for obj_path in sorted(out.glob("*.obj")):
    render_structure(obj_path)
    render_textured(obj_path)
