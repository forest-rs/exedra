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


def focus_joint_view(output: Path) -> None:
    """Isolate fitted timber members and apply any requested explosion."""
    offsets: dict[str, Vector] = {}
    if output.name == "heel_south_exploded.png":
        visible = ("tie-beam-east", "principal-rafter-south-east")
        offsets = {"principal-rafter-south-east": Vector((-0.55, -0.20, 0.24))}
    elif output.name == "king_post_tie_exploded.png":
        visible = ("tie-beam-east", "king-post-east", "king-post-to-tie-east-key")
        offsets = {"king-post-east": Vector((0.0, 0.0, 0.42))}
    elif output.name in ("truss_assembled.png", "truss_exploded.png"):
        visible = (
            "tie-beam-east",
            "principal-rafter-south-east",
            "principal-rafter-north-east",
            "king-post-east",
            "king-post-to-tie-east-key",
            "strut-south-east",
            "strut-north-east",
        )
        if output.name == "truss_exploded.png":
            # Pull members in the truss plane and lift the keyed post as a
            # whole. The offsets are large enough to expose every bearing
            # face without changing the exported meshes themselves.
            offsets = {
                "tie-beam-east": Vector((0.0, 0.0, -0.28)),
                "principal-rafter-south-east": Vector((0.0, -0.42, 0.25)),
                "principal-rafter-north-east": Vector((0.0, 0.42, 0.25)),
                "king-post-east": Vector((0.34, 0.0, 0.22)),
                "king-post-to-tie-east-key": Vector((0.62, 0.0, 0.0)),
                "strut-south-east": Vector((-0.30, -0.24, 0.0)),
                "strut-north-east": Vector((-0.30, 0.24, 0.0)),
            }
    elif output.name == "purlin_trench_exploded.png":
        # One principal/purlin pair makes the trench bottom and the intact
        # purlin section readable without unrelated roof members in front.
        visible = ("principal-rafter-south-east", "purlin-south-mid")
        offsets = {"purlin-south-mid": Vector((0.0, -0.34, 0.38))}
    elif output.name in (
        "secondary_roof_assembled.png",
        "secondary_roof_exploded.png",
    ):
        visible = (
            "principal-rafter-south-west",
            "principal-rafter-south-east",
            "purlin-south-",
            "common-rafter-south-",
        )
        if output.name == "secondary_roof_exploded.png":
            # Separate the three structural layers along the roof normal-ish
            # view direction. The exported cut faces are untouched; only whole
            # objects move for inspection.
            offsets = {
                "purlin-south-": Vector((0.0, -0.26, 0.34)),
                "common-rafter-south-": Vector((0.0, -0.52, 0.72)),
            }
    else:
        return

    for obj in bpy.context.scene.objects:
        if obj.type != "MESH":
            continue
        obj.hide_render = not any(token in obj.name for token in visible)
        for token, offset in offsets.items():
            if token not in obj.name:
                continue
            # Move the complete fitted member, not its vertices, so its cut
            # faces and normals remain exactly as exported.
            obj.location += offset
            break


def render_view(
    obj: Path,
    output: Path,
    location: tuple[float, float, float],
    target: tuple[float, float, float],
) -> None:
    reset_scene()
    import_obj(obj)
    focus_bearing_view(output)
    focus_joint_view(output)
    if "exploded" not in output.name:
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
            "structure.obj",
            "purlin_trench_exploded.png",
            (6.7, -7.2, 14.8),
            (4.0, -2.5, 13.0),
        ),
        (
            "structure.obj",
            "heel_south_exploded.png",
            (5.8, -6.5, 12.4),
            (4.0, -4.5, 11.3),
        ),
        (
            "structure.obj",
            "king_post_tie_exploded.png",
            (5.7, -2.6, 12.3),
            (4.0, 0.0, 11.5),
        ),
        (
            "structure.obj",
            "truss_assembled.png",
            (8.8, -11.5, 13.6),
            (4.0, 0.0, 12.0),
        ),
        (
            "structure.obj",
            "truss_exploded.png",
            (8.8, -11.5, 13.8),
            (4.0, 0.0, 12.0),
        ),
        (
            "structure.obj",
            "secondary_roof_assembled.png",
            (8.4, -12.8, 15.6),
            (2.0, -2.8, 12.8),
        ),
        (
            "structure.obj",
            "secondary_roof_exploded.png",
            (8.4, -13.2, 16.3),
            (2.0, -2.8, 13.1),
        ),
    ]
    for obj_name, output_name, location, target in checkpoints:
        render_view(artifacts / obj_name, renders / output_name, location, target)


if __name__ == "__main__":
    main()
