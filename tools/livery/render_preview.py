"""Neutral-studio EEVEE preview renders of the FP04 Gen22 livery-mapping bodywork.

Usage (from the repo root):
  /usr/sbin/blender -b --factory-startup --disable-autoexec --python-exit-code 1 \
      --python tools/livery/render_preview.py -- TEX_DIR OUT_DIR [--obj-dir DIR]

--obj-dir defaults to raw/gen22/livery-mapping/obj under the repo root.

TEX_DIR must hold <part>.png for part in main_body, front_wing, rear_wing,
mirrors, floor, cockpit_foam. A missing PNG falls back to dark carbon.

Coordinate frame: the OBJs are Y-up with the nose at +X and +Z = car right.
The importer maps that to Blender Z-up with the nose at +X, car left at +Y,
car right at -Y. The mesh is baked to that frame before framing.
"""

import argparse
import math
import os
import sys
import time
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix, Quaternion, Vector

REPO = Path(__file__).resolve().parents[2]
DEFAULT_OBJ_DIR = REPO / "raw/gen22/livery-mapping/obj"
PARTS = [
    ("fp04rb_main_body.obj", "main_body"),
    ("fp04rb_front_wing.obj", "front_wing"),
    ("fp04rb_rear_wing.obj", "rear_wing"),
    ("fp04rb_mirrors.obj", "mirrors"),
    ("fp04rb_floor.obj", "floor"),
    ("fp04rb_cockpit_foam.obj", "cockpit_foam"),
]

RES_X, RES_Y = 1600, 900
ASPECT = RES_X / RES_Y
SAMPLES = 32
FALLBACK_RGB = (0.02, 0.02, 0.025)
BODY_ROUGHNESS = 0.35
COAT_WEIGHT = 0.3
CARBON_ROUGHNESS = 0.4
FRAME_MARGIN = 1.08  # fraction of frame kept as empty border


def parse_args():
    argv = sys.argv
    if "--" not in argv:
        raise SystemExit("usage: blender ... --python tools/livery/render_preview.py -- "
                         "TEX_DIR OUT_DIR [--obj-dir DIR]")
    ap = argparse.ArgumentParser(prog="render_preview.py")
    ap.add_argument("tex_dir", help="directory holding <part>.png textures")
    ap.add_argument("out_dir", help="directory for the rendered PNGs")
    ap.add_argument("--obj-dir", default=str(DEFAULT_OBJ_DIR),
                    help="directory holding fp04rb_<part>.obj (default: %(default)s)")
    args = ap.parse_args(argv[argv.index("--") + 1:])
    return (os.path.abspath(args.tex_dir), os.path.abspath(args.out_dir),
            os.path.abspath(args.obj_dir))


def clear_scene():
    for obj in list(bpy.data.objects):
        bpy.data.objects.remove(obj, do_unlink=True)


def import_part(filename, obj_dir):
    """Import one OBJ, keep its UVs, and bake the importer transform into the mesh."""
    before = {o.name for o in bpy.data.objects}
    bpy.ops.wm.obj_import(
        filepath=os.path.join(obj_dir, filename),
        forward_axis="NEGATIVE_Z",  # file +X (nose) -> Blender +X
        up_axis="Y",               # file +Y (up)     -> Blender +Z
    )
    objs = [o for o in bpy.data.objects if o.name not in before and o.type == "MESH"]
    for obj in objs:
        obj.data.transform(obj.matrix_world)
        obj.matrix_world = Matrix.Identity(4)
    return objs


def is_carbon(material):
    return material is not None and "carbon" in material.name.lower()


def make_part_material(part, png_path):
    """One Principled BSDF: Image Texture -> Base Color, or dark carbon fallback."""
    mat = bpy.data.materials.new(f"gen22_{part}")
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = nt.nodes["Principled BSDF"]
    bsdf.inputs["Roughness"].default_value = BODY_ROUGHNESS
    bsdf.inputs["Coat Weight"].default_value = COAT_WEIGHT
    if os.path.isfile(png_path):
        img = bpy.data.images.load(png_path, check_existing=True)
        img.colorspace_settings.name = "sRGB"
        tex = nt.nodes.new("ShaderNodeTexImage")
        tex.image = img
        tex.interpolation = "Linear"
        nt.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
        print(f"  {part}: texture={os.path.basename(png_path)} size={tuple(img.size)} "
              f"interp={tex.interpolation} colorspace={img.colorspace_settings.name} "
              f"roughness={bsdf.inputs['Roughness'].default_value} "
              f"coat={bsdf.inputs['Coat Weight'].default_value}")
    else:
        bsdf.inputs["Base Color"].default_value = (*FALLBACK_RGB, 1.0)
        print(f"  WARNING: missing {png_path}; using carbon fallback colour")
    return mat


def make_carbon_material():
    mat = bpy.data.materials.new("gen22_carbon")
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*FALLBACK_RGB, 1.0)
    bsdf.inputs["Roughness"].default_value = CARBON_ROUGHNESS
    return mat


def retarget_materials(obj, body_mat, carbon_mat):
    """Replace all slots: faces from 'Carbon' slots -> carbon, the rest -> body."""
    mesh = obj.data
    n_poly = len(mesh.polygons)
    old_idx = np.zeros(n_poly, dtype=np.int32)
    mesh.polygons.foreach_get("material_index", old_idx)
    # Sentinel False at the end catches any out-of-range index (e.g. empty slot).
    slot_is_carbon = np.array([is_carbon(s.material) for s in obj.material_slots] + [False])
    face_is_carbon = slot_is_carbon[np.clip(old_idx, 0, len(slot_is_carbon) - 1)]

    mesh.materials.clear()
    mesh.materials.append(body_mat)
    if face_is_carbon.any():
        mesh.materials.append(carbon_mat)
    mesh.polygons.foreach_set("material_index", face_is_carbon.astype(np.int32))
    mesh.update()
    return n_poly, int(face_is_carbon.sum())


def vertices_world(objs):
    chunks = []
    for obj in objs:
        n = len(obj.data.vertices)
        co = np.empty(n * 3, dtype=np.float32)
        obj.data.vertices.foreach_get("co", co)
        chunks.append(co.reshape(-1, 3).astype(np.float64))
    return np.concatenate(chunks)


def setup_world(scene):
    world = scene.world or bpy.data.worlds.new("gen22_world")
    scene.world = world
    world.use_nodes = True
    bg = world.node_tree.nodes["Background"]
    bg.inputs["Color"].default_value = (0.6, 0.6, 0.6, 1.0)
    bg.inputs["Strength"].default_value = 0.8


def add_lights_and_ground(scene, centre, car_top_z, car_len):
    # Large square area light above the car, pointing down (default -Z emission).
    area_data = bpy.data.lights.new("key_area", type="AREA")
    area_data.energy = 1500.0
    area_data.shape = "SQUARE"
    area_data.size = 10.0
    area = bpy.data.objects.new("key_area", area_data)
    scene.collection.objects.link(area)
    area.location = (centre[0], centre[1], car_top_z + car_len)

    # Sun from front-left, above (light travels toward -X, -Y, -Z).
    sun_data = bpy.data.lights.new("sun", type="SUN")
    sun_data.energy = 2.0
    sun = bpy.data.objects.new("sun", sun_data)
    scene.collection.objects.link(sun)
    travel = Vector((-0.5, -0.5, -1.0)).normalized()
    sun.rotation_euler = travel.to_track_quat("-Z", "Y").to_euler()

    # Large grey ground at Blender Z=0 (OBJ Y=0).
    bpy.ops.mesh.primitive_plane_add(size=1000.0, location=(0.0, 0.0, 0.0))
    ground = bpy.context.active_object
    ground.name = "ground"
    gmat = bpy.data.materials.new("gen22_ground")
    gmat.use_nodes = True
    gb = gmat.node_tree.nodes["Principled BSDF"]
    gb.inputs["Base Color"].default_value = (0.45, 0.45, 0.45, 1.0)
    gb.inputs["Roughness"].default_value = 0.8
    ground.data.materials.append(gmat)


def make_camera(scene, name, ortho):
    data = bpy.data.cameras.new(name)
    if ortho:
        data.type = "ORTHO"
    else:
        data.lens = 50.0
    cam = bpy.data.objects.new(name, data)
    scene.collection.objects.link(cam)
    return cam


def frame_camera(cam, pts, view_dir, ortho, rotation=None):
    """Position cam so the points fit the frame, looking at their centre.

    view_dir points from the subject towards the camera. rotation overrides
    the look-at orientation (used for the exact top-down view).
    """
    lo, hi = pts.min(axis=0), pts.max(axis=0)
    centre = (lo + hi) / 2.0
    if rotation is None:
        fwd_b = -Vector(view_dir).normalized()
        q = fwd_b.to_track_quat("-Z", "Y")
    else:
        q = rotation
    right = np.array(q @ Vector((1, 0, 0)))
    up = np.array(q @ Vector((0, 1, 0)))
    fwd = np.array(q @ Vector((0, 0, -1)))

    rel = pts - centre
    a = rel @ right  # horizontal offset of each point
    b = rel @ up     # vertical offset of each point
    e = rel @ fwd    # depth offset of each point

    cam.rotation_mode = "QUATERNION"
    cam.rotation_quaternion = q
    if ortho:
        half_w = float(np.abs(a).max())
        half_h = float(np.abs(b).max())
        cam.data.ortho_scale = 2.0 * FRAME_MARGIN * max(half_w, ASPECT * half_h)
        dist = 10.0 + float(np.abs(e).max())
        cam.location = Vector(centre - fwd * dist)
        cam.data.clip_start = 0.1
        cam.data.clip_end = 2.0 * dist + 10.0
        return centre, cam.data.ortho_scale
    t_h = (cam.data.sensor_width / 2.0) / cam.data.lens
    t_v = t_h / ASPECT
    th, tv = t_h / FRAME_MARGIN, t_v / FRAME_MARGIN
    # Smallest distance d such that every point satisfies |a| <= th*(e+d), |b| <= tv*(e+d).
    dist = max(float((np.maximum(np.abs(a) / th, np.abs(b) / tv) - e).max()), 0.5)
    cam.location = Vector(centre - fwd * dist)
    cam.data.clip_start = 0.05
    cam.data.clip_end = 5.0 * dist + 100.0
    return centre, dist


def main():
    tex_dir, out_dir, obj_dir = parse_args()
    os.makedirs(out_dir, exist_ok=True)
    scene = bpy.context.scene
    t_start = time.time()

    clear_scene()
    print(f"Blender {bpy.app.version_string}")
    print(f"TEX_DIR={tex_dir}\nOUT_DIR={out_dir}")

    # Import, retarget materials per object.
    carbon_mat = make_carbon_material()
    car_objs = []
    for filename, part in PARTS:
        t0 = time.time()
        objs = import_part(filename, obj_dir)
        body_mat = make_part_material(part, os.path.join(tex_dir, f"{part}.png"))
        for obj in objs:
            n_poly, n_carbon = retarget_materials(obj, body_mat, carbon_mat)
            print(f"{part}: object={obj.name} faces={n_poly} carbon_faces={n_carbon} "
                  f"slots={[s.name for s in obj.material_slots]}  ({time.time() - t0:.1f}s)")
        car_objs.extend(objs)

    # Drop orphaned import materials, images, and the default cube.
    for mat in list(bpy.data.materials):
        if mat.users == 0:
            bpy.data.materials.remove(mat)
    for img in list(bpy.data.images):
        if img.users == 0:
            bpy.data.images.remove(img)
    for mesh in list(bpy.data.meshes):
        if mesh.users == 0:
            bpy.data.meshes.remove(mesh)

    pts = vertices_world(car_objs)
    lo, hi = pts.min(axis=0), pts.max(axis=0)
    centre = (lo + hi) / 2.0
    car_len = float(hi[0] - lo[0])
    print(f"car bounds min={np.round(lo, 3)} max={np.round(hi, 3)} length_x={car_len:.3f}")

    # Nose plus cockpit subset for the close-up: everything forward of the cockpit foam's rear edge.
    cockpit_pts = vertices_world([o for o in car_objs if "cockpit" in o.name.lower()])
    nose_pts = pts[pts[:, 0] >= cockpit_pts[:, 0].min()]
    print(f"nose/cockpit subset x_min={cockpit_pts[:, 0].min():.3f} n={len(nose_pts)}")

    setup_world(scene)
    add_lights_and_ground(scene, centre, float(hi[2]), car_len)

    # Engine and render settings.
    engines = {e.identifier for e in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items}
    engine = "BLENDER_EEVEE_NEXT" if "BLENDER_EEVEE_NEXT" in engines else "BLENDER_EEVEE"
    scene.render.engine = engine
    scene.eevee.taa_render_samples = SAMPLES
    scene.render.resolution_x = RES_X
    scene.render.resolution_y = RES_Y
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    scene.view_settings.view_transform = "Standard"
    print(f"engine={engine} samples={SAMPLES} res={RES_X}x{RES_Y}")

    identity = Quaternion()
    views = [
        # name, view direction (subject -> camera), ortho, rotation override, framing points
        ("side_left", (0.0, 1.0, 0.12), False, None, pts),
        ("side_right", (0.0, -1.0, 0.12), False, None, pts),
        ("top", (0.0, 0.0, 1.0), True, identity, pts),
        ("front", (1.0, 0.0, 0.10), False, None, pts),
        ("rear", (-1.0, 0.0, 0.10), False, None, pts),
        ("front34", (0.7, 0.7, 0.3), False, None, pts),
        ("rear34", (-0.7, -0.7, 0.3), False, None, pts),
        ("nose_top", (0.8, 0.0, 0.8), False, None, nose_pts),
    ]
    for name, view_dir, ortho, rotation, frame_pts in views:
        cam = make_camera(scene, name, ortho)
        _, fit = frame_camera(cam, frame_pts, view_dir, ortho, rotation)
        scene.camera = cam
        scene.render.filepath = os.path.join(out_dir, f"{name}.png")
        t0 = time.time()
        bpy.ops.render.render(write_still=True)
        print(f"rendered {name}.png  fit={fit:.3f} cam={np.round(np.array(cam.location), 2)}  "
              f"({time.time() - t0:.1f}s)")

    print(f"DONE total={time.time() - t_start:.1f}s")


main()
