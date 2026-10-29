"""Prepare a car GLB for the game (run with Blender in background mode).

  blender -b --factory-startup -P tools/car/prepare_car.py -- \
      <source.glb> <out dir> --wheel-materials a,b,c [--max-faces N] [--max-texture N]

Output in <out dir>:
  car.glb    nodes: body, wheel_fl, wheel_fr, wheel_rl, wheel_rr
             glTF space: +Z forward, +Y up, origin at ground level midway
             between the axles; wheel nodes pivot at the wheel centre
  car.json   wheel centres and radii, body extents

The source cars point forward along -Y in Blender space (+Z after glTF export).
"""

import json
import os
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector


def parse_args():
    args = sys.argv[sys.argv.index("--") + 1 :]
    options = {"--max-faces": "0", "--max-texture": "1024", "--wheel-materials": ""}
    positional = []
    i = 0
    while i < len(args):
        if args[i] in options:
            options[args[i]] = args[i + 1]
            i += 2
        else:
            positional.append(args[i])
            i += 1
    source, out_dir = positional
    wheel_materials = {name for name in options["--wheel-materials"].split(",") if name}
    return source, out_dir, wheel_materials, int(options["--max-faces"]), int(options["--max-texture"])


def select_only(objects):
    bpy.ops.object.select_all(action="DESELECT")
    for obj in objects:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]


def mesh_objects():
    return [obj for obj in bpy.context.scene.objects if obj.type == "MESH"]


def bounds(obj):
    points = [obj.matrix_world @ Vector(corner) for corner in obj.bound_box]
    low = Vector((min(p.x for p in points), min(p.y for p in points), min(p.z for p in points)))
    high = Vector((max(p.x for p in points), max(p.y for p in points), max(p.z for p in points)))
    return low, high


def main():
    source, out_dir, wheel_materials, max_faces, max_texture = parse_args()
    os.makedirs(out_dir, exist_ok=True)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=source)

    objects = mesh_objects()
    select_only(objects)
    if len(objects) > 1:
        bpy.ops.object.join()
    body = bpy.context.view_layer.objects.active
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    body.name = "body"

    # Split wheel geometry out by material.
    bpy.ops.object.mode_set(mode="EDIT")
    mesh = bmesh.from_edit_mesh(body.data)
    wheel_slots = {
        index for index, slot in enumerate(body.material_slots)
        if slot.material and slot.material.name in wheel_materials
    }
    missing = wheel_materials - {slot.material.name for slot in body.material_slots if slot.material}
    if missing:
        raise SystemExit(f"wheel materials not found: {sorted(missing)}")
    for face in mesh.faces:
        face.select = face.material_index in wheel_slots
    bmesh.update_edit_mesh(body.data)
    bpy.ops.mesh.separate(type="SELECTED")
    bpy.ops.object.mode_set(mode="OBJECT")
    wheels_all = next(obj for obj in mesh_objects() if obj is not body)

    # Assign every wheel face to the nearest quadrant (left/right, front/rear).
    wheels = {}
    for key in ("fl", "fr", "rl", "rr"):
        select_only([wheels_all])
        bpy.ops.object.duplicate()
        wheel = bpy.context.view_layer.objects.active
        wheel.name = f"wheel_{key}"
        front = key[0] == "f"
        left = key[1] == "l"
        bpy.ops.object.mode_set(mode="EDIT")
        data = bmesh.from_edit_mesh(wheel.data)
        doomed = []
        for face in data.faces:
            centre = face.calc_center_median()
            # Car forward is -Y; driver's left is +X when facing -Y.
            is_front = centre.y < 0.0
            is_left = centre.x > 0.0
            if is_front != front or is_left != left:
                doomed.append(face)
        bmesh.ops.delete(data, geom=doomed, context="FACES")
        bmesh.update_edit_mesh(wheel.data)
        bpy.ops.object.mode_set(mode="OBJECT")
        wheels[key] = wheel
    bpy.data.objects.remove(wheels_all, do_unlink=True)

    # Ground the car at z=0 and centre it between the axles.
    low = Vector((min(bounds(o)[0].z for o in mesh_objects()),) * 3)
    centres = {key: sum(bounds(w), Vector()) * 0.5 for key, w in wheels.items()}
    front_axle = (centres["fl"].y + centres["fr"].y) * 0.5
    rear_axle = (centres["rl"].y + centres["rr"].y) * 0.5
    shift = Matrix.Translation((0.0, -(front_axle + rear_axle) * 0.5, -low.z))
    for obj in mesh_objects():
        obj.matrix_world = shift @ obj.matrix_world
        select_only([obj])
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)

    # Wheels pivot about their own centre.
    metadata = {"wheels": {}}
    for key, wheel in wheels.items():
        lo, hi = bounds(wheel)
        centre = (lo + hi) * 0.5
        radius = (hi.z - lo.z) * 0.5
        width = hi.x - lo.x
        select_only([wheel])
        bpy.context.scene.cursor.location = centre
        bpy.ops.object.origin_set(type="ORIGIN_CURSOR")
        # glTF: x stays, y_gltf = z_blender, z_gltf = -y_blender
        metadata["wheels"][key] = {
            "centre": [round(centre.x, 4), round(centre.z, 4), round(-centre.y, 4)],
            "radius": round(radius, 4),
            "width": round(width, 4),
        }

    if max_faces:
        for obj in mesh_objects():
            faces = len(obj.data.polygons)
            share = faces / sum(len(o.data.polygons) for o in mesh_objects())
            target = max_faces * share
            if faces > target:
                select_only([obj])
                modifier = obj.modifiers.new("decimate", "DECIMATE")
                modifier.ratio = target / faces
                modifier.use_collapse_triangulate = True
                bpy.ops.object.modifier_apply(modifier=modifier.name)

    for image in bpy.data.images:
        if image.size[0] == 0:
            continue
        largest = max(image.size)
        if largest > max_texture:
            scale = max_texture / largest
            image.scale(max(1, int(image.size[0] * scale)), max(1, int(image.size[1] * scale)))

    lo = Vector((1e9,) * 3)
    hi = Vector((-1e9,) * 3)
    for obj in mesh_objects():
        a, b = bounds(obj)
        lo = Vector(map(min, lo, a))
        hi = Vector(map(max, hi, b))
    metadata["extents"] = {
        "min": [round(lo.x, 4), round(lo.z, 4), round(-hi.y, 4)],
        "max": [round(hi.x, 4), round(hi.z, 4), round(-lo.y, 4)],
    }
    metadata["faces"] = {obj.name: len(obj.data.polygons) for obj in mesh_objects()}

    select_only(mesh_objects())
    bpy.ops.export_scene.gltf(
        filepath=os.path.join(out_dir, "car.glb"),
        export_format="GLB",
        use_selection=True,
        export_image_format="AUTO",
        export_yup=True,
        export_apply=True,
        export_tangents=False,
        export_materials="EXPORT",
    )
    with open(os.path.join(out_dir, "car.json"), "w") as handle:
        json.dump(metadata, handle, indent=2)
    print("prepared", json.dumps(metadata))


main()
