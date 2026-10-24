# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render exported evidence; geometry, normals and UVs are never repaired here.

blender --background --python render_surface_quality.py -- target/surface_quality
"""

import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector


def solid(name, color):
    material = bpy.data.materials.new(name)
    if material.node_tree is None:
        material.use_nodes = True
    shader = material.node_tree.nodes.get("Principled BSDF")
    shader.inputs["Base Color"].default_value = (*color, 1)
    shader.inputs["Roughness"].default_value = 0.7
    return material


def label(text, position, size, material):
    curve = bpy.data.curves.new(text, "FONT")
    curve.body = text
    curve.size = size
    obj = bpy.data.objects.new(text, curve)
    bpy.context.collection.objects.link(obj)
    obj.location = position
    obj.data.materials.append(material)


def import_ring(root, name, position=(0, 0, 0)):
    before = set(bpy.context.scene.objects)
    bpy.ops.import_scene.gltf(filepath=str(root / f"{name}.glb"))
    imported = set(bpy.context.scene.objects) - before
    for obj in imported:
        if obj.parent not in imported:
            obj.scale = tuple(value * 0.01 for value in obj.scale)
            obj.location = obj.location * 0.01 + Vector(position)
    return imported


def setup():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 32
    scene.cycles.use_denoising = True
    scene.world.color = (0.12, 0.12, 0.12)
    scene.view_settings.view_transform = "AgX"
    return scene


def lighting(center, scale=1):
    center = Vector(center)
    for offset, power, size in [((-1.5, -2, 4), 220, 3), ((3, 1, 3), 150, 2), ((-2, 3, 2), 100, 2)]:
        bpy.ops.object.light_add(type="AREA", location=center + Vector(offset) * scale)
        light = bpy.context.object
        light.data.energy = power * scale * scale
        light.data.shape = "DISK"
        light.data.size = size * scale
        light.rotation_euler = (center - light.location).to_track_quat("-Z", "Y").to_euler()


def render(scene, path, center, offset, scale, resolution):
    if scene.camera is None or scene.camera.name not in scene.objects:
        bpy.ops.object.camera_add()
        scene.camera = bpy.context.object
    camera = scene.camera
    center = Vector(center)
    camera.location = center + Vector(offset)
    camera.rotation_euler = (center - camera.location).to_track_quat("-Z", "Y").to_euler()
    camera.data.type = "ORTHO"
    camera.data.ortho_scale = scale
    camera.data.clip_start = 0.001
    scene.render.resolution_x, scene.render.resolution_y = resolution
    scene.render.resolution_percentage = 100
    scene.render.filepath = str(path)
    bpy.ops.render.render(write_still=True)


def main():
    args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    root = Path(args[0] if args else "target/surface_quality").resolve()
    preview = "--preview" in args
    survey = json.loads((root / "survey.json").read_text())
    stats = {case["name"]: case["quality"] for case in survey["cases"]}
    scene = setup()
    ground = solid("ground", (0.025, 0.037, 0.05))
    slate = solid("slate", (0.045, 0.060, 0.075))
    text = solid("labels", (0.85, 0.90, 0.93))
    notes = solid("notes", (0.45, 0.64, 0.70))
    bpy.ops.mesh.primitive_plane_add(size=100, location=(1.9, 0.9, -0.13))
    bpy.context.object.data.materials.append(ground)
    cards = [
        ("lost-creases", "Lost sharp edges", "Same positions and UVs", 0, 1.9),
        ("projected", "Retained sharp edges", "Toroidal coordinates", 1.9, 1.9),
        ("construction", "Construction coordinates", "Sweep chart + cutter charts", 3.8, 1.9),
        ("baked-normals", "Baked normal transport", "Sharp edges absent; normals retained", 0, 0),
        ("projected-quality", "Toroidal anisotropy", "projected", 1.9, 0),
        ("construction-quality", "Construction anisotropy", "construction", 3.8, 0),
    ]
    for name, title, note, x, y in cards:
        import_ring(root, name, (x, y, 0))
        bpy.ops.mesh.primitive_cube_add(size=1, location=(x, y, -0.12))
        tile = bpy.context.object
        tile.dimensions = (1.7, 1.7, 0.025)
        tile.data.materials.append(slate)
        label(title, (x - 0.76, y - 0.69, -0.105), 0.095, text)
        if note in stats:
            area = stats[note]["outside_limits_area_fraction"] * 100
            note = f">8:1 area: {area:.9f}%" if area < 0.001 else f">8:1 area: {area:.3f}%"
        label(note, (x - 0.76, y - 0.81, -0.105), 0.064, notes)
    label("TEXTURE QUALITY THROUGH MODELING", (-0.76, 3.04, 0), 0.22, text)
    label("ONE 21-CUT RING  /  MATCHED RENDER TRIANGLES  /  STORED UV METRICS", (-0.74, 2.80, 0), 0.077, notes)
    label("ANISOTROPY UPPER BOUND", (0.96, -0.99, -0.10), 0.075, text)
    for index, (color, title) in enumerate(zip(survey["quality_colors"], ["1.05", "1.25", "1.5", "2", "3", "4", "8", "16", ">16", "invalid"])):
        x = 1.1 + index * 0.32
        bpy.ops.mesh.primitive_cube_add(size=1, location=(x, -1.12, -0.11))
        obj = bpy.context.object
        obj.dimensions = (0.28, 0.11, 0.015)
        obj.data.materials.append(solid(title, color))
        label(title, (x - 0.10, -1.30, -0.10), 0.065, notes)
    lighting((1.9, 1, 0), 2.2)
    if preview:
        scene.cycles.samples = 12
    render(scene, root / "comparison.png", (1.9, 0.98, 0), (0, -5, 13), 6.6, (1400, 1100) if preview else (2400, 1900))

    if "--comparison-only" in args:
        return

    scene = setup()
    import_ring(root, "construction")
    ground = solid("ground", (0.025, 0.037, 0.05))
    bpy.ops.mesh.primitive_plane_add(size=100, location=(0, 0, -0.108))
    bpy.context.object.data.materials.append(ground)
    lighting((0, 0, 0))
    if preview:
        scene.cycles.samples = 12
    render(scene, root / "construction-detail.png", (0.01, -0.51, 0), (0.08, -1.3, 1.2), 0.47, (1200, 900) if preview else (1800, 1350))
    render(scene, root / "construction.png", (0, 0, 0), (0.35, -1.8, 2.8), 1.48, (1400, 1100) if preview else (2400, 1900))


if __name__ == "__main__":
    main()
