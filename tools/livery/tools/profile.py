import bpy, sys
from mathutils import Vector
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=sys.argv[sys.argv.index("--")+1])
S=4096; bins={}
for ob in bpy.data.objects:
    if ob.type!='MESH': continue
    me=ob.data; uv=me.uv_layers[0].data; me.calc_loop_triangles(); mw=ob.matrix_world
    for t in me.loop_triangles:
        m=ob.material_slots[t.material_index].material
        if not any('main_body' in (n.image.name if n.type=='TEX_IMAGE' and n.image else '') for n in m.node_tree.nodes): continue
        P=[mw@me.vertices[me.loops[l].vertex_index].co for l in t.loops]
        n=(P[1]-P[0]).cross(P[2]-P[0])
        if n.length==0: continue
        n.normalize(); c=(P[0]+P[1]+P[2])/3
        if c.x>0 or abs(n.x)<0.15: continue
        T=[(uv[l].uv.x*S,(1-uv[l].uv.y)*S) for l in t.loops]
        k=round(c.y,1)
        bins.setdefault(k,[]).append((round(c.z,2),round(sum(a for a,b in T)/3),round(sum(b for a,b in T)/3)))
for k in sorted(bins):
    if 0.4<=k<=1.6:
        zs=sorted(bins[k]); top=zs[-1]
        print(k, "zmax",top, "zmin",zs[0], "n",len(zs))
