import bpy, mathutils
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=__import__("sys").argv[__import__("sys").argv.index("--") + 1])
xs=[]
for o in bpy.data.objects:
    if o.type=='MESH':
        for c in o.bound_box: xs.append(o.matrix_world@mathutils.Vector(c))
print("dim",[round(max(v[i] for v in xs)-min(v[i] for v in xs),2) for i in range(3)])
print("min",[round(min(v[i] for v in xs),2) for i in range(3)], "max",[round(max(v[i] for v in xs),2) for i in range(3)])
