"""Viewer only: render car.glb with replacement textures from several angles.

blender -b --factory-startup --disable-autoexec --python preview.py -- GLB OUT_PREFIX VIEWS NAME=PNG [NAME=PNG ...]
NAME is the GLB image name (e.g. textures_audi_main_body_stickers). VIEWS is comma separated or "all".
A PNG of STOCK saves that embedded image to OUT_PREFIX-NAME.png instead.
"""
import bpy, sys
from mathutils import Vector

args = sys.argv[sys.argv.index("--") + 1:]
glb, out, views_arg, swaps = args[0], args[1], args[2], args[3:]
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
for swap in swaps:
    name, png = swap.split("=", 1)
    old = bpy.data.images[name]
    if png == "STOCK":
        old.filepath_raw = f"{out}-{name}.png"
        old.file_format = "PNG"
        old.save()
        continue
    new = bpy.data.images.load(png)
    for material in bpy.data.materials:
        if material.use_nodes:
            for node in material.node_tree.nodes:
                if node.type == "TEX_IMAGE" and node.image == old:
                    node.image = new

scene = bpy.context.scene
scene.render.engine = "BLENDER_EEVEE"
scene.eevee.taa_render_samples = 16
scene.view_settings.view_transform = "Standard"
scene.world = bpy.data.worlds.new("w")
scene.world.use_nodes = True
scene.world.node_tree.nodes["Background"].inputs[0].default_value = (0.55, 0.57, 0.6, 1)
scene.world.node_tree.nodes["Background"].inputs[1].default_value = 1.6

camera = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
scene.collection.objects.link(camera)
camera.data.type = "ORTHO"
scene.camera = camera
# The car is along Y with its nose towards -Y; +X is the side seen as "side-a".
V = {
    #  name: (look-at centre, direction to camera, ortho scale, width, height)
    "side-a":  ((0, -0.4, 0.5), (1, 0, 0), 5.8, 1400, 700),
    "side-b":  ((0, -0.4, 0.5), (-1, 0, 0), 5.8, 1400, 700),
    "top":     ((0, -0.4, 0.5), (0, 0, 1), 5.8, 1400, 700),
    "front34": ((0, -0.8, 0.4), (0.7, -1, 0.55), 4.6, 1400, 900),
    "rear34":  ((0, 0.6, 0.5), (0.7, 1, 0.55), 4.6, 1400, 900),
    "fwing":   ((0, -2.75, 0.15), (0.35, -1, 0.75), 2.3, 1400, 900),
    "fwing-top": ((0, -2.7, 0.15), (0, -0.05, 1), 2.3, 1400, 900),
    "rwing":   ((0, 2.15, 0.8), (0.45, 1, 0.3), 1.7, 1400, 900),
    "rwing-side": ((0, 2.15, 0.75), (1, 0, 0), 1.5, 1200, 900),
    "halo-out": ((0, -0.35, 0.95), (1, -0.25, 0.35), 1.4, 1400, 900),
    "halo-in":  ((0, -0.35, 0.95), (-0.25, 0.9, 0.75), 1.4, 1400, 900),
    "halo-top": ((0, -0.35, 0.95), (0.05, 0.1, 1), 1.4, 1400, 900),
    "top-rear": ((0, 0.4, 0.8), (0, 0, 1), 2.4, 1400, 1000),
    "top-mid":  ((0, -0.5, 0.8), (0, 0, 1), 2.4, 1400, 1000),
    "photo":   ((0, -0.3, 0.45), (1, -0.35, 0.55), 5.0, 1600, 900),
    "fwing-up": ((0, -2.7, 0.2), (0.15, -0.6, 1), 2.3, 1400, 900),
    "halo-out": ((0, -0.35, 0.95), (1, -0.25, 0.35), 1.4, 1400, 900),
    "halo-in":  ((0, -0.35, 0.95), (-0.25, 0.9, 0.75), 1.4, 1400, 900),
    "halo-top": ((0, -0.35, 0.95), (0.05, 0.1, 1), 1.4, 1400, 900),
    "top-rear": ((0, 0.4, 0.8), (0, 0, 1), 2.4, 1400, 1000),
    "top-mid":  ((0, -0.5, 0.8), (0, 0, 1), 2.4, 1400, 1000),
    "photo":   ((0, -0.3, 0.45), (1, -0.35, 0.55), 5.0, 1600, 900),
    "fwing-up": ((0, -2.7, 0.2), (0.15, -0.6, 1), 2.3, 1400, 900),
    "rwing-front": ((0, 2.1, 0.85), (0.25, -1, 0.45), 1.6, 1400, 900),
}
names = list(V) if views_arg == "all" else views_arg.split(",")
for name in names:
    centre, direction, scale, w, h = V[name]
    centre, direction = Vector(centre), Vector(direction).normalized()
    scene.render.resolution_x, scene.render.resolution_y = w, h
    camera.location = centre + direction * 12
    camera.data.ortho_scale = scale
    up = "Y" if name not in ("top", "fwing-top") else "X"
    camera.rotation_euler = (centre - camera.location).to_track_quat("-Z", up).to_euler()
    if name in ("top", "fwing-top"):
        camera.rotation_euler = (0, 0, 1.5708)
    scene.render.filepath = f"{out}-{name}.png"
    bpy.ops.render.render(write_still=True)
