import bpy, sys
from mathutils import Vector
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=sys.argv[sys.argv.index("--")+1])
S=4096
for ob in bpy.data.objects:
    if ob.type!='MESH': continue
    me=ob.data
    mats=[s.material for s in ob.material_slots]
    fw=[i for i,m in enumerate(mats) if m and any('front_wing' in (n.image.name if n.type=='TEX_IMAGE' and n.image else '') for n in m.node_tree.nodes)]
    if not fw: continue
    print("OBJ", ob.name, len(me.polygons))
    # vertex groups / separate loose parts: use attribute 'element' if present
    print("  attrs", [a.name for a in me.attributes])
