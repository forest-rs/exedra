# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render exported door UVs without unwrapping or changing the imported geometry.

blender --background --python render_textured_finishes.py -- target/textured-finishes
"""

from pathlib import Path
import sys

import bpy
from mathutils import Vector


def material(kind):
    mat = bpy.data.materials.new(kind)
    mat.use_nodes = True
    nodes, links = mat.node_tree.nodes, mat.node_tree.links
    shader = nodes.get("Principled BSDF")
    shader.inputs["Roughness"].default_value = 0.38
    uv = nodes.new("ShaderNodeTexCoord").outputs["UV"]
    if kind == "checker":
        texture = nodes.new("ShaderNodeTexChecker")
        texture.inputs["Scale"].default_value = 8
        texture.inputs["Color1"].default_value = (0.035, 0.075, 0.105, 1)
        texture.inputs["Color2"].default_value = (0.85, 0.72, 0.48, 1)
        links.new(uv, texture.inputs["Vector"])
        links.new(texture.outputs["Color"], shader.inputs["Base Color"])
    else:
        stretch = nodes.new("ShaderNodeVectorMath")
        stretch.operation = "MULTIPLY"
        stretch.inputs[1].default_value = (35, 0.6, 1)
        links.new(uv, stretch.inputs[0])
        texture = nodes.new("ShaderNodeTexNoise")
        texture.inputs["Scale"].default_value = 3
        texture.inputs["Detail"].default_value = 3
        links.new(stretch.outputs["Vector"], texture.inputs["Vector"])
        ramp = nodes.new("ShaderNodeValToRGB")
        ramp.color_ramp.elements[0].position = 0.2
        ramp.color_ramp.elements[0].color = (0.055, 0.018, 0.005, 1)
        ramp.color_ramp.elements[1].position = 0.8
        ramp.color_ramp.elements[1].color = (0.58, 0.32, 0.11, 1)
        links.new(texture.outputs["Fac"], ramp.inputs["Fac"])
        links.new(ramp.outputs["Color"], shader.inputs["Base Color"])
    return mat


def load_door(root, name, mat, offset=0):
    before = set(bpy.context.scene.objects)
    bpy.ops.import_scene.gltf(filepath=str(root / f"{name}.glb"))
    imported = set(bpy.context.scene.objects) - before
    for obj in imported:
        if obj.parent is None:
            obj.location.x += offset
        if obj.type == "MESH":
            assert obj.data.uv_layers.active is not None, f"{name}: missing exported UVs"
            assert len(obj.data.uv_layers.active.data) == len(obj.data.loops)
            # Replace only the caller's material; preserve UVs and imported normals.
            for i in range(len(obj.data.materials)):
                obj.data.materials[i] = mat


def setup(center, close):
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
    for offset, energy, size in [((-0.5, -1, 1.2), 110, 0.8), ((0.8, 0.3, 0.5), 75, 0.6)]:
        bpy.ops.object.light_add(type="AREA", location=center + Vector(offset))
        light = bpy.context.object
        light.data.energy, light.data.size = energy, size
        light.rotation_euler = (center - light.location).to_track_quat("-Z", "Y").to_euler()
    offset = Vector((0.075, -0.16, 0.09)) if close else Vector((0.45, -3, 1.1))
    bpy.ops.object.camera_add(location=center + offset)
    camera = bpy.context.object
    camera.rotation_euler = (center - camera.location).to_track_quat("-Z", "Y").to_euler()
    camera.data.type = "ORTHO"
    camera.data.ortho_scale = 0.095 if close else 1.8
    camera.data.clip_start = 0.001
    scene.camera = camera
    return scene


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    root = Path(args[0] if args else "target/textured-finishes").resolve()
    for kind in ["checker", "woodgrain"]:
        for view in ["fillet-close", "comparison", "chamfer-close"]:
            bpy.ops.object.select_all(action="SELECT")
            bpy.ops.object.delete(use_global=False)
            mat = material(kind)
            if view == "comparison":
                for name, offset in [("door-sharp", 0), ("door-chamfer", 0.55), ("door-fillet", 1.1)]:
                    load_door(root, name, mat, offset)
                center = Vector((0.775, 0.012, 0.35))
            else:
                load_door(root, "door-fillet" if view == "fillet-close" else "door-chamfer", mat)
                center = Vector((0.433, 0.008, 0.68))
            scene = setup(center, view != "comparison")
            scene.render.filepath = str(root / f"{kind}-{view}.png")
            bpy.ops.render.render(write_still=True)


if __name__ == "__main__":
    main()
