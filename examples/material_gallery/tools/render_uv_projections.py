# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render the exported projection studies without altering their UVs or geometry.

blender --background --python render_uv_projections.py -- target/uv_projections
"""

import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector


def solid(name, color, roughness=0.65):
    material = bpy.data.materials.new(name)
    shader = material.node_tree.nodes.get("Principled BSDF")
    shader.inputs["Base Color"].default_value = (*color, 1)
    shader.inputs["Roughness"].default_value = roughness
    return material


def label(text, position, size, material):
    curve = bpy.data.curves.new(text, "FONT")
    curve.body = text
    curve.size = size
    curve.extrude = 0
    obj = bpy.data.objects.new(text, curve)
    bpy.context.collection.objects.link(obj)
    obj.location = position
    obj.data.materials.append(material)


def camera(center, offset, scale):
    obj = bpy.context.scene.camera
    obj.location = center + offset
    obj.rotation_euler = (center - obj.location).to_track_quat("-Z", "Y").to_euler()
    obj.data.ortho_scale = scale


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    root = Path(args[0] if args else "target/uv_projections").resolve()
    manifest = json.loads((root / "gallery.json").read_text())
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    bpy.ops.import_scene.gltf(filepath=str(root / "uv-projections.glb"))
    for obj in list(bpy.context.scene.objects):
        if obj.type == "MESH":
            assert obj.data.uv_layers.active is not None, f"{obj.name}: no imported UVs"
            assert len(obj.data.uv_layers.active.data) == len(obj.data.loops)

    slate = solid("slate", (0.055, 0.075, 0.09))
    ground = solid("ground", (0.025, 0.037, 0.048))
    text = solid("labels", (0.82, 0.88, 0.9))
    muted = solid("notes", (0.44, 0.6, 0.65))
    bpy.ops.mesh.primitive_plane_add(size=200, location=(6.75, 2.4, -0.08))
    bpy.context.object.data.materials.append(ground)
    for card in manifest["cards"]:
        x, y, _ = card["position"]
        bpy.ops.mesh.primitive_cube_add(size=1, location=(x, y, -0.025))
        obj = bpy.context.object
        obj.dimensions = (4.0, 4.0, 0.08)
        obj.data.materials.append(slate)
        label(card["label"], (x - 1.8, y - 1.52, 0.021), 0.20, text)
        stats = card["mapping"]
        note = f"{stats['seam_corners']} seam lifts | {stats['pole_corners']} pole repairs"
        label(note, (x - 1.8, y - 1.83, 0.02), 0.12, muted)
    label("UV PROJECTION STUDIES", (-1.8, 7.5, 0), 0.48, text)
    label("ONE EMBEDDED GRID  /  IMPORTED MESHES  /  RECIPE-AUTHORED UVs", (-1.75, 7.04, 0), 0.18, muted)

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 48
    scene.cycles.use_denoising = True
    scene.render.resolution_x = 2400
    scene.render.resolution_y = 1700
    scene.render.resolution_percentage = 100
    scene.world.color = (0.2, 0.2, 0.2)
    scene.view_settings.view_transform = "AgX"
    for location, energy, size in [((0, -5, 12), 2200, 8), ((12, 4, 10), 1800, 7)]:
        bpy.ops.object.light_add(type="AREA", location=location)
        light = bpy.context.object
        light.data.energy = energy
        light.data.shape = "DISK"
        light.data.size = size
        light.rotation_euler = (Vector((6.75, 2.4, 0)) - light.location).to_track_quat("-Z", "Y").to_euler()
    bpy.ops.object.camera_add()
    scene.camera = bpy.context.object
    scene.camera.data.type = "ORTHO"
    scene.camera.data.clip_start = 0.01
    camera(Vector((6.75, 2.7, 0.45)), Vector((0, -12, 28)), 19.5)
    scene.render.filepath = str(root / "gallery.png")
    bpy.ops.render.render(write_still=True)
    scene.render.resolution_x, scene.render.resolution_y = 1400, 1100
    for name, center, offset, scale in [
        ("sphere-pole", (0, 4.8, 1.05), (0.5, -2, 3.5), 2.65),
        ("torus-seams", (4.5, 4.8, 0.5), (0.5, -1.6, 3.5), 2.75),
    ]:
        camera(Vector(center), Vector(offset), scale)
        scene.render.filepath = str(root / f"{name}.png")
        bpy.ops.render.render(write_still=True)


if __name__ == "__main__":
    main()
