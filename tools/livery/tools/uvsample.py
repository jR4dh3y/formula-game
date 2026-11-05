import bpy, sys, json
from mathutils import Vector
a = sys.argv[sys.argv.index("--")+1:]
glb, mat, rows, x0, x1, step = a[0], a[1], [float(v) for v in a[2].split(",")], int(a[3]), int(a[4]), int(a[5])
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
S=4096; tris=[]
for ob in bpy.data.objects:
    if ob.type!='MESH': continue
    me=ob.data; uv=me.uv_layers[0].data; me.calc_loop_triangles(); mw=ob.matrix_world
    for t in me.loop_triangles:
        m=ob.material_slots[t.material_index].material
        nm=[n.image.name for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image]
        if not any(mat in n for n in nm): continue
        P=[mw@me.vertices[me.loops[l].vertex_index].co for l in t.loops]
        T=[Vector((uv[l].uv.x*S,(1-uv[l].uv.y)*S)) for l in t.loops]
        tris.append((P,T))
def find(p):
    out=[]
    for P,T in tris:
        a,b,c=T; v0,v1,v2=b-a,c-a,p-a; d=v0.x*v1.y-v1.x*v0.y
        if abs(d)<1e-9: continue
        u=(v2.x*v1.y-v1.x*v2.y)/d; v=(v0.x*v2.y-v2.x*v0.y)/d
        if u>=0 and v>=0 and u+v<=1:
            q=P[0]+(P[1]-P[0])*u+(P[2]-P[0])*v; out.append(tuple(round(c,3) for c in q))
    return out
for y in rows:
    print("ROW",y)
    for x in range(x0,x1,step): print(" ",x,find(Vector((x,y))))
