"""Prepare a 2026 team car GLB for the game (run with Blender in background mode).

  blender -b --disable-autoexec --factory-startup -P tools/car/prepare_gen26.py -- \
      <source.glb> <out dir> [--max-texture 4096] [--lod-texture 2048] [--lod-ratio 0.5]

The 2026 grid models (Dave Love, CC BY 4.0) merge their geometry per material
and name their wheel materials differently, so wheels and the rear-wing flap
are found from geometry instead:

* tyres are ground-touching rings about 0.7 m across; every loose part that
  fits inside a wheel's cylinder (tyre, rim, disc) becomes that wheel;
* the rear-wing flap is the widest separate part at the top of the rear wing.

Output in <out dir>:
  car.glb       full geometry and textures up to --max-texture (player car)
  car_lod1.glb  decimated by --lod-ratio, textures up to --lod-texture (opponents)
  car.json      wheel centres and radii, body extents and the flap's pivot

Nodes: body, wheel_fl, wheel_fr, wheel_rl, wheel_rr and, when found, drs_flap.
glTF space: +Z forward, +Y up, origin on the ground midway between the axles.
"""

import json
import os
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector


def parse_args():
    args = sys.argv[sys.argv.index("--") + 1 :]
    options = {"--max-texture": "4096", "--lod-texture": "2048", "--lod-ratio": "0.5"}
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
    return (
        source,
        out_dir,
        int(options["--max-texture"]),
        int(options["--lod-texture"]),
        float(options["--lod-ratio"]),
    )


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


def islands(mesh):
    """Loose parts as (face indices, low, high) in object space."""
    mesh.faces.ensure_lookup_table()
    seen = set()
    parts = []
    for start in mesh.faces:
        if start.index in seen:
            continue
        stack = [start]
        seen.add(start.index)
        faces = []
        while stack:
            face = stack.pop()
            faces.append(face.index)
            for edge in face.edges:
                for other in edge.link_faces:
                    if other.index not in seen:
                        seen.add(other.index)
                        stack.append(other)
        points = [v.co for i in faces for v in mesh.faces[i].verts]
        low = Vector((min(p.x for p in points), min(p.y for p in points), min(p.z for p in points)))
        high = Vector((max(p.x for p in points), max(p.y for p in points), max(p.z for p in points)))
        parts.append((faces, low, high))
    return parts


def separate(body, face_indices, name):
    """Moves faces of `body` into a new object called `name`."""
    select_only([body])
    bpy.ops.object.mode_set(mode="EDIT")
    data = bmesh.from_edit_mesh(body.data)
    data.faces.ensure_lookup_table()
    wanted = set(face_indices)
    for face in data.faces:
        face.select = face.index in wanted
    bmesh.update_edit_mesh(body.data)
    bpy.ops.mesh.separate(type="SELECTED")
    bpy.ops.object.mode_set(mode="OBJECT")
    new = next(obj for obj in mesh_objects() if obj is not body and obj.name not in NAMED)
    new.name = name
    NAMED.add(name)
    return new


NAMED = {"body"}


def main():
    source, out_dir, max_texture, lod_texture, lod_ratio = parse_args()
    os.makedirs(out_dir, exist_ok=True)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=source)

    objects = mesh_objects()
    select_only(objects)
    if len(objects) > 1:
        bpy.ops.object.join()
    body = bpy.context.view_layer.objects.active
    # Sketchfab exports parent the meshes under scaled, rotated empties.
    body.parent = None
    body.matrix_world = body.matrix_world.copy()
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    for obj in list(bpy.context.scene.objects):
        if obj.type != "MESH":
            bpy.data.objects.remove(obj, do_unlink=True)
    body.name = "body"

    # Length runs along the longer horizontal axis; rotate it onto Y.
    low, high = bounds(body)
    size = high - low
    if size.x > size.y:
        body.matrix_world = Matrix.Rotation(1.5707963, 4, "Z") @ body.matrix_world
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
        low, high = bounds(body)
        size = high - low

    mesh = bmesh.new()
    mesh.from_mesh(body.data)
    parts = islands(mesh)
    ground = low.z
    # Tyres: ground-touching rings about 0.6-0.85 m across, narrower than tall.
    tyres = [
        (faces, lo, hi)
        for faces, lo, hi in parts
        if lo.z < ground + 0.06
        and 0.55 < hi.z - lo.z < 0.9
        and 0.55 < hi.y - lo.y < 0.9
        and 0.12 < hi.x - lo.x < 0.55
    ]
    centre_x = (low.x + high.x) * 0.5
    centre_y = (low.y + high.y) * 0.5
    quadrants = {}
    for faces, lo, hi in tyres:
        mid = (lo + hi) * 0.5
        key = ("a" if mid.y > centre_y else "b", "l" if mid.x > centre_x else "r")
        box = quadrants.setdefault(key, [lo.copy(), hi.copy()])
        box[0] = Vector(map(min, box[0], lo))
        box[1] = Vector(map(max, box[1], hi))
    if len(quadrants) != 4:
        raise SystemExit(f"found tyres in {sorted(quadrants)} only ({len(tyres)} tyre parts)")
    # The nose overhangs its axle further than the gearbox does: that end is
    # the front, which the game wants along -Y in Blender space.
    axle_a = (quadrants[("a", "l")][0].y + quadrants[("a", "l")][1].y) * 0.5
    axle_b = (quadrants[("b", "l")][0].y + quadrants[("b", "l")][1].y) * 0.5
    front_is_a = high.y - axle_a > axle_b - low.y
    flip = front_is_a  # front at +Y: rotate 180 degrees so it faces -Y

    wheel_parts = {}
    for (end, side), (lo, hi) in quadrants.items():
        centre = (lo + hi) * 0.5
        radius = (hi.z - lo.z) * 0.5
        # Rims and hubs sit inboard of the tyre; anything within the wheel's
        # radius that far in turns with it.
        inboard = 0.18
        if side == "l":
            x_low, x_high = lo.x - inboard, hi.x + 0.01
        else:
            x_low, x_high = lo.x - 0.01, hi.x + inboard
        members = []
        for faces, plo, phi in parts:
            inside = (
                plo.x >= x_low
                and phi.x <= x_high
                and abs(plo.y - centre.y) <= radius * 1.03
                and abs(phi.y - centre.y) <= radius * 1.03
                and plo.z >= centre.z - radius * 1.03
                and phi.z <= centre.z + radius * 1.03
            )
            if inside:
                members.extend(faces)
        front = (end == "a") == front_is_a
        # Driver's left is +X when facing -Y; facing +Y before the flip it is -X.
        left = (side == "l") != flip
        wheel_parts[("f" if front else "r") + ("l" if left else "r")] = members

    # Rear-wing flap: the widest separate part high up behind the rear axle.
    rear_axle = axle_b if front_is_a else axle_a
    behind = (lambda y: y < rear_axle - 0.15) if front_is_a else (lambda y: y > rear_axle + 0.15)
    height = high.z - low.z
    used = {face for faces in wheel_parts.values() for face in faces}
    candidates = [
        (faces, lo, hi)
        for faces, lo, hi in parts
        if behind((lo.y + hi.y) * 0.5)
        and lo.z > low.z + height * 0.6
        and hi.x - lo.x > 0.8
        and 0.08 < hi.y - lo.y < 0.5
        and hi.z - lo.z < 0.3
        and len(faces) >= 300
        and not used.intersection(faces[:1])
    ]
    flap = max(candidates, key=lambda part: part[2].z, default=None)
    flap_faces = []
    if flap:
        # The seed is often only one layer of the flap (its paint or decal
        # skin). Take every face wholly inside the seed's box, padded a little
        # front, back and vertically but not sideways, so the carbon under
        # the paint moves with it and the endplates stay put.
        _, seed_lo, seed_hi = flap
        pad = Vector((0.0, 0.03, 0.03))
        box_lo, box_hi = seed_lo - pad, seed_hi + pad
        mesh.faces.ensure_lookup_table()
        for face in mesh.faces:
            if face.index in used:
                continue
            if all(
                box_lo.x <= v.co.x <= box_hi.x
                and box_lo.y <= v.co.y <= box_hi.y
                and box_lo.z <= v.co.z <= box_hi.z
                for v in face.verts
            ):
                flap_faces.append(face.index)
    mesh.free()

    wheels = {}
    for key in ("fl", "fr", "rl", "rr"):
        wheels[key] = wheel_parts[key]
    # Separate in one pass per object; face indices shift after each split, so
    # tag faces with a temporary integer layer instead of using raw indices.
    select_only([body])
    bpy.ops.object.mode_set(mode="EDIT")
    data = bmesh.from_edit_mesh(body.data)
    tag = data.faces.layers.int.new("part")
    data.faces.ensure_lookup_table()
    names = ["wheel_fl", "wheel_fr", "wheel_rl", "wheel_rr", "drs_flap"]
    groups = [wheels["fl"], wheels["fr"], wheels["rl"], wheels["rr"], flap_faces]
    for number, faces in enumerate(groups, start=1):
        for index in faces:
            data.faces[index][tag] = number
    bmesh.update_edit_mesh(body.data)
    bpy.ops.object.mode_set(mode="OBJECT")
    split = {}
    for number, name in enumerate(names, start=1):
        if not groups[number - 1]:
            continue
        select_only([body])
        bpy.ops.object.mode_set(mode="EDIT")
        data = bmesh.from_edit_mesh(body.data)
        tag = data.faces.layers.int["part"]
        for face in data.faces:
            face.select = face[tag] == number
        bmesh.update_edit_mesh(body.data)
        bpy.ops.mesh.separate(type="SELECTED")
        bpy.ops.object.mode_set(mode="OBJECT")
        new = next(o for o in mesh_objects() if o is not body and o not in split.values())
        new.name = name
        split[name] = new
    for obj in mesh_objects():
        layers = obj.data.attributes
        if "part" in layers:
            layers.remove(layers["part"])

    # Face forward along -Y, ground the car and centre it between the axles.
    turn = Matrix.Rotation(3.14159265, 4, "Z") if flip else Matrix.Identity(4)
    for obj in mesh_objects():
        obj.matrix_world = turn @ obj.matrix_world
        select_only([obj])
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    centres = {key: sum(bounds(split["wheel_" + key]), Vector()) * 0.5 for key in wheels}
    front_axle = (centres["fl"].y + centres["fr"].y) * 0.5
    back_axle = (centres["rl"].y + centres["rr"].y) * 0.5
    floor = min(bounds(o)[0].z for o in mesh_objects())
    shift = Matrix.Translation((-(centres["fl"].x + centres["fr"].x) * 0.5,
                                -(front_axle + back_axle) * 0.5, -floor))
    for obj in mesh_objects():
        obj.matrix_world = shift @ obj.matrix_world
        select_only([obj])
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)

    metadata = {"wheels": {}}
    for key in ("fl", "fr", "rl", "rr"):
        wheel = split["wheel_" + key]
        lo, hi = bounds(wheel)
        centre = (lo + hi) * 0.5
        select_only([wheel])
        bpy.context.scene.cursor.location = centre
        bpy.ops.object.origin_set(type="ORIGIN_CURSOR")
        # glTF: x stays, y_gltf = z_blender, z_gltf = -y_blender
        metadata["wheels"][key] = {
            "centre": [round(centre.x, 4), round(centre.z, 4), round(-centre.y, 4)],
            "radius": round((hi.z - lo.z) * 0.5, 4),
            "width": round(hi.x - lo.x, 4),
        }
    if "drs_flap" in split:
        flap_obj = split["drs_flap"]
        lo, hi = bounds(flap_obj)
        # Hinge along the flap's leading (front, -Y) edge.
        pivot = Vector((0.0, lo.y, (lo.z + hi.z) * 0.5))
        select_only([flap_obj])
        bpy.context.scene.cursor.location = pivot
        bpy.ops.object.origin_set(type="ORIGIN_CURSOR")
        metadata["drs"] = {
            "pivot": [round(pivot.x, 6), round(pivot.z, 6), round(-pivot.y, 6)],
            "axis": [1, 0, 0],
            "open_angle": -0.25,
        }

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

    def cap_textures(limit):
        for image in bpy.data.images:
            if image.size[0] == 0:
                continue
            largest = max(image.size)
            if largest > limit:
                scale = limit / largest
                image.scale(max(1, int(image.size[0] * scale)), max(1, int(image.size[1] * scale)))

    def export(name):
        select_only(mesh_objects())
        bpy.ops.export_scene.gltf(
            filepath=os.path.join(out_dir, name),
            export_format="GLB",
            use_selection=True,
            export_image_format="AUTO",
            export_yup=True,
            export_apply=True,
            export_tangents=False,
            export_materials="EXPORT",
        )

    cap_textures(max_texture)
    export("car.glb")
    # Opponent LOD: lighter body and textures, same wheels and flap nodes.
    if lod_ratio < 1.0:
        select_only([body])
        modifier = body.modifiers.new("decimate", "DECIMATE")
        modifier.ratio = lod_ratio
        modifier.use_collapse_triangulate = True
        bpy.ops.object.modifier_apply(modifier=modifier.name)
    cap_textures(lod_texture)
    export("car_lod1.glb")
    metadata["lod1_faces"] = {obj.name: len(obj.data.polygons) for obj in mesh_objects()}
    with open(os.path.join(out_dir, "car.json"), "w") as handle:
        json.dump(metadata, handle, indent=2)
    print("prepared", json.dumps(metadata))


main()
