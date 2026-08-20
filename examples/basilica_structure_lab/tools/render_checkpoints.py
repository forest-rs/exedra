# Copyright 2026 the Exedra Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render the structural lab's deterministic semantic OBJ checkpoints."""

import argparse
import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector


def parse_args() -> argparse.Namespace:
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifact-dir", required=True)
    parser.add_argument("--render-dir", required=True)
    return parser.parse_args(argv)


def reset_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for material in tuple(bpy.data.materials):
        bpy.data.materials.remove(material)


def import_obj(path: Path) -> None:
    bpy.ops.wm.obj_import(filepath=str(path), forward_axis="Y", up_axis="Z")
    for material in bpy.data.materials:
        alpha = 1.0
        if "roof-transparent" in material.name:
            alpha = 0.26
        elif "boarding-transparent" in material.name:
            alpha = 0.34
        elif "diagnostic-context" in material.name:
            alpha = 0.32
        material.diffuse_color[3] = alpha
        material.use_nodes = True
        principled = material.node_tree.nodes.get("Principled BSDF")
        if principled is not None:
            principled.inputs["Roughness"].default_value = 0.72
            principled.inputs["Metallic"].default_value = 0.0
            principled.inputs["Alpha"].default_value = alpha
        if alpha < 1.0:
            if hasattr(material, "surface_render_method"):
                material.surface_render_method = "BLENDED"
            elif hasattr(material, "blend_method"):
                material.blend_method = "BLEND"


def point_camera(camera: bpy.types.Object, target: tuple[float, float, float]) -> None:
    direction = Vector(target) - camera.location
    camera.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()


def add_camera(
    location: tuple[float, float, float],
    target: tuple[float, float, float],
) -> None:
    bpy.ops.object.camera_add(location=location)
    camera = bpy.context.object
    camera.data.lens = 52
    point_camera(camera, target)
    bpy.context.scene.camera = camera


def add_lighting() -> None:
    bpy.ops.object.light_add(type="AREA", location=(1.5, -4.0, 19.0))
    key = bpy.context.object
    key.data.energy = 1450
    key.data.shape = "DISK"
    key.data.size = 8.0
    key.rotation_euler = (math.radians(18), 0.0, math.radians(24))

    bpy.ops.object.light_add(type="AREA", location=(7.0, 6.0, 13.0))
    fill = bpy.context.object
    fill.data.energy = 850
    fill.data.size = 7.0
    point_camera(fill, (2.0, 0.0, 11.0))

    bpy.ops.object.light_add(type="SUN", location=(0.0, 0.0, 18.0))
    sun = bpy.context.object
    sun.data.energy = 1.2
    sun.rotation_euler = (math.radians(25), math.radians(-18), math.radians(130))


def add_ground() -> None:
    bpy.ops.mesh.primitive_plane_add(size=34.0, location=(2.0, 0.0, -0.13))
    ground = bpy.context.object
    material = bpy.data.materials.new("checkpoint-ground")
    material.diffuse_color = (0.055, 0.065, 0.075, 1.0)
    ground.data.materials.append(material)


def configure_scene(output: Path) -> None:
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE"
    scene.render.resolution_x = 960
    scene.render.resolution_y = 720
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.filepath = str(output)
    scene.render.film_transparent = False
    scene.world.color = (0.018, 0.022, 0.032)
    scene.view_settings.look = "AgX - Medium High Contrast"


def focus_bearing_view(output: Path) -> None:
    """Keep bearing checkpoints crisp and scoped to the named interface."""
    focus = {
        "bearing_south_close.png": (
            "bearing-principal-south-east-on-wall-plate",
        ),
        "bearing_north_close.png": (
            "bearing-principal-north-east-on-wall-plate",
        ),
        "bearing_ridge_close.png": (
            "bearing-ridge-purlin-south-on-principal-east",
        ),
    }.get(output.name)
    if focus is None:
        return
    for obj in bpy.context.scene.objects:
        if obj.name.startswith(("roof-covering-", "boarding-")):
            obj.hide_render = True
        if obj.name.startswith("bearing-"):
            if not any(token in obj.name for token in focus):
                obj.hide_render = True
            elif obj.data.vertices:
                center = (
                    sum((vertex.co for vertex in obj.data.vertices), Vector())
                    / len(obj.data.vertices)
                )
                for vertex in obj.data.vertices:
                    vertex.co = center + (vertex.co - center) * 1.8


def render_view(
    obj: Path,
    output: Path,
    location: tuple[float, float, float],
    target: tuple[float, float, float],
) -> None:
    reset_scene()
    import_obj(obj)
    focus_bearing_view(output)
    add_ground()
    add_lighting()
    add_camera(location, target)
    configure_scene(output)
    bpy.ops.render.render(write_still=True)


def main() -> None:
    args = parse_args()
    artifacts = Path(args.artifact_dir).resolve()
    renders = Path(args.render_dir).resolve()
    renders.mkdir(parents=True, exist_ok=True)
    checkpoints = [
        (
            "structure.obj",
            "structure_oblique.png",
            (9.2, -13.0, 10.0),
            (2.0, 0.0, 11.4),
        ),
        (
            "load-path.obj",
            "load_path_oblique.png",
            (9.4, 12.0, 15.5),
            (2.0, 0.0, 9.8),
        ),
        (
            "transparent-roof.obj",
            "transparent_roof_oblique.png",
            (-7.0, -12.0, 17.0),
            (2.0, 0.0, 11.5),
        ),
        (
            "bearings.obj",
            "bearing_south_close.png",
            (7.0, -8.0, 12.0),
            (4.0, -4.5, 11.0),
        ),
        (
            "bearings.obj",
            "bearing_north_close.png",
            (7.0, 8.0, 12.0),
            (4.0, 4.5, 11.0),
        ),
        (
            "bearings.obj",
            "bearing_ridge_close.png",
            (6.7, -3.8, 15.0),
            (4.0, 0.0, 13.6),
        ),
    ]
    for obj_name, output_name, location, target in checkpoints:
        render_view(artifacts / obj_name, renders / output_name, location, target)


if __name__ == "__main__":
    main()
