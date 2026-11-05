# frames at texture points: texture px vectors for 1 m along "baseline" and "up" on the surface
import bpy, sys, json
from mathutils import Vector
a=sys.argv[sys.argv.index("--")+1:]
glb, mat, mode, pts = a[0], a[1], a[2], [tuple(map(float,p.split(","))) for p in a[3].split(";")]
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
        tris.append(([mw@me.vertices[me.loops[l].vertex_index].co for l in t.loops],[Vector((uv[l].uv.x*S,(1-uv[l].uv.y)*S)) for l in t.loops]))
res=[]
for (x,y) in pts:
    p=Vector((x,y)); acc=[]
    for P,T in tris:
        a_,b_,c_=T; v0,v1,v2=b_-a_,c_-a_,p-a_; d=v0.x*v1.y-v1.x*v0.y
        if abs(d)<1e-9: continue
        u=(v2.x*v1.y-v1.x*v2.y)/d; v=(v0.x*v2.y-v2.x*v0.y)/d
        if not(u>=0 and v>=0 and u+v<=1): continue
        q=P[0]+(P[1]-P[0])*u+(P[2]-P[0])*v
        f1,f2=P[1]-P[0],P[2]-P[0]; n=f1.cross(f2).normalized()
        if mode=="side" and n.x*q.x<0: n=-n
        if mode=="top" and n.z<0: n=-n
        up=Vector((0,0,1)) if mode=="side" else Vector((0,1,0))
        up=(up-n*up.dot(n)).normalized(); base=up.cross(n).normalized()
        # 3D = f1*s + f2*t ; tex = v0*s + v1*t  -> solve s,t for 3D vec w (least squares), tex delta = v0*s+v1*t
        def tex(w):
            g11,g12,g22=f1.dot(f1),f1.dot(f2),f2.dot(f2); r1,r2=f1.dot(w),f2.dot(w); D=g11*g22-g12*g12
            s=(g22*r1-g12*r2)/D; t=(g11*r2-g12*r1)/D; dv=v0*s+v1*t; return [round(dv.x,1),round(dv.y,1)]
        acc.append({"q":[round(c,3) for c in q],"n":[round(c,2) for c in n],"base":tex(base),"up":tex(up)})
    res.append({"pt":[x,y],"hits":acc})
print("RESULT",json.dumps(res))
