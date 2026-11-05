import bpy, sys
from mathutils import Vector
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=sys.argv[sys.argv.index("--")+1])
c=Vector((0.456,-0.69,0.62))
for ob in bpy.data.objects:
    if ob.type!='MESH': continue
    me=ob.data; me.calc_loop_triangles(); mw=ob.matrix_world; n=0
    for t in me.loop_triangles:
        P=[mw@me.vertices[me.loops[l].vertex_index].co for l in t.loops]; q=(P[0]+P[1]+P[2])/3
        if abs(q.y-c.y)<0.05 and abs(abs(q.x)-c.x)<0.05 and q.z>0.45:
            m=ob.material_slots[t.material_index].material
            uvs=[tuple(round(v,3) for v in me.uv_layers[0].data[l].uv) for l in t.loops]
            if n<12: print(ob.name, m.name, [round(v,3) for v in q], uvs, "uv1" , [tuple(round(v,3) for v in me.uv_layers[1].data[l].uv) for l in t.loops] if len(me.uv_layers)>1 else None)
            n+=1
    print(ob.name,"hits",n)
