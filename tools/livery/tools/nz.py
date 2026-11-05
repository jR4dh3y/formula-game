import bpy, sys, numpy as np
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=sys.argv[sys.argv.index("--")+1])
for ob in bpy.data.objects:
    if ob.type!='MESH': continue
    me=ob.data; me.calc_loop_triangles(); mw=ob.matrix_world
    rows=[]
    for t in me.loop_triangles:
        m=ob.material_slots[t.material_index].material
        if not any('main_body' in (n.image.name if n.type=='TEX_IMAGE' and n.image else '') for n in m.node_tree.nodes): continue
        P=[mw@me.vertices[me.loops[l].vertex_index].co for l in t.loops]; c=(P[0]+P[1]+P[2])/3
        if c.x<0 and 0.95<c.y<1.35:
            n=(P[1]-P[0]).cross(P[2]-P[0]); 
            if n.length: n.normalize(); rows.append((round(c.z,2),round(c.x,3),round(n.x,2),round(n.z,2), round(c.y,2)))
    for z in np.arange(0.45,0.92,0.025):
        sel=[r for r in rows if abs(r[0]-z)<0.0125 and abs(r[4]-1.15)<0.05]
        if sel:
            o=min(sel,key=lambda r:r[1])  # outermost
            print(round(z,3),"outer x",o[1],"nx",o[2],"nz",o[3],"count",len(sel))
