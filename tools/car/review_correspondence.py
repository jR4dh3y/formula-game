"""Review the authored UVs on original source objects, without runtime repacking.

Invoke Blender with --disable-autoexec and the original source blend. This script
changes only the in-memory review scene and never saves the downloaded blend.
"""

import sys
import argparse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bpy, author_uv
from render_gen22 import studio, shot
from mathutils import Vector

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--source", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
objects = list(bpy.data.collections["fp04_tr_model"].all_objects)
author_uv.author(objects, args.output)
for o in list(bpy.data.objects):
    if o not in objects:
        bpy.data.objects.remove(o, do_unlink=True)
image = bpy.data.images.load(str(args.source.resolve()))
for o in objects:
    if o.type != "MESH":
        continue
    m = bpy.data.materials.new("source UV review " + o.name)
    m.use_nodes = True
    bs = m.node_tree.nodes.get("Principled BSDF")
    bs.inputs["Base Color"].default_value = (0.03, 0.03, 0.03, 1)
    bs.inputs["Roughness"].default_value = 0.5
    if o.name in author_uv.json.loads(author_uv.PROFILE.read_text())["objects"]:
        tex = m.node_tree.nodes.new("ShaderNodeTexImage")
        tex.image = image
        tex.extension = "CLIP"
        uv = m.node_tree.nodes.new("ShaderNodeUVMap")
        uv.uv_map = author_uv.LAYER
        m.node_tree.links.new(uv.outputs["UV"], tex.inputs["Vector"])
        m.node_tree.links.new(tex.outputs["Color"], bs.inputs["Base Color"])
    o.data.materials.clear()
    o.data.materials.append(m)
    for p in o.data.polygons:
        p.material_index = 0
scene, camera = studio(args.output)
shot(scene, camera, args.output / "left.png", (8, 1.8, 2.3), (0.05, 1.8, 0.55), 45)
shot(scene, camera, args.output / "right.png", (-8, 1.8, 2.3), (0.05, 1.8, 0.55), 45)
