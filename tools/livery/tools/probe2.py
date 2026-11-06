import bpy, sys, collections
from mathutils import Vector
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=sys.argv[sys.argv.index("--")+1])
c=Vector((0.44,-0.69,0.66)); cnt=collections.Counter(); zr=collections.defaultdict(list)
for ob in bpy.data.objects:
    if ob.type!='MESH': continue
    me=ob.data; me.calc_loop_triangles(); mw=ob.matrix_world
    for t in me.loop_triangles:
        P=[mw@me.vertices[me.loops[l].vertex_index].co for l in t.loops]; q=(P[0]+P[1]+P[2])/3
        if abs(q.y-c.y)<0.04 and abs(abs(q.x)-c.x)<0.04 and q.z>0.4:
            m=ob.material_slots[t.material_index].material.name; cnt[m]+=1; zr[m].append(round(q.z,3))
            n=(P[1]-P[0]).cross(P[2]-P[0]).normalized()
            zr[m+"_nz"].append(round(n.z,2))
for k,v in cnt.items(): print(k,v, sorted(set(zr[k]))[:6], sorted(set(zr[k]))[-3:], "nz", sorted(set(zr[k+"_nz"]))[:5], sorted(set(zr[k+"_nz"]))[-3:])
