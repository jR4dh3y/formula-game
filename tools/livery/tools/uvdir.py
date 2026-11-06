# blender -b --python uvdir.py -- GLB MATSUBSTR "x,y;x,y;..."
import bpy, sys, math, json
from mathutils import Vector
a = sys.argv[sys.argv.index("--")+1:]
glb, mat, pts = a[0], a[1], [tuple(map(float,p.split(","))) for p in a[2].split(";")]
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
S = 4096
tris = []
for ob in bpy.data.objects:
    if ob.type != 'MESH': continue
    me = ob.data; uv = me.uv_layers[0].data; me.calc_loop_triangles()
    mw = ob.matrix_world
    for t in me.loop_triangles:
        m = ob.material_slots[t.material_index].material if ob.material_slots else None
        if not m: continue
        names = [n.image.name for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image] if m.use_nodes else []
        if not any(mat in n for n in names+[m.name]): continue
        P = [mw @ me.vertices[me.loops[l].vertex_index].co for l in t.loops]
        T = [Vector((uv[l].uv.x*S, (1-uv[l].uv.y)*S)) for l in t.loops]
        tris.append((P, T, ob.name))
print("tris", len(tris))
def bary(p, a, b, c):
    v0, v1, v2 = b-a, c-a, p-a
    d = v0.x*v1.y - v1.x*v0.y
    if abs(d) < 1e-9: return None
    u = (v2.x*v1.y - v1.x*v2.y)/d; v = (v0.x*v2.y - v2.x*v0.y)/d
    return (u, v) if u >= -1e-4 and v >= -1e-4 and u+v <= 1+1e-4 else None
out = []
for (x, y) in pts:
    p = Vector((x, y)); hits = []
    for P, T, on in tris:
        if bary(p, *T):
            # Jacobian: d3D/dtex
            e1, e2 = T[1]-T[0], T[2]-T[0]; f1, f2 = P[1]-P[0], P[2]-P[0]
            det = e1.x*e2.y - e2.x*e1.y
            if abs(det) < 1e-9: continue
            # tex = A * [s,t]; 3D = F*[s,t] -> d3D/dtex = F * A^-1
            inv = [[e2.y/det, -e2.x/det], [-e1.y/det, e1.x/det]]
            dx = f1*inv[0][0] + f2*inv[1][0]; dy = f1*inv[0][1] + f2*inv[1][1]
            n = f1.cross(f2).normalized()
            # texture direction of a 3D axis projected onto the surface: solve dx*a+dy*b = axis_proj
            res = {}
            for nm, ax in (("X", Vector((1,0,0))), ("Y", Vector((0,1,0))), ("Z", Vector((0,0,1)))):
                ap = ax - n*ax.dot(n)
                if ap.length < 1e-6: res[nm] = None; continue
                # least squares
                g11, g12, g22 = dx.dot(dx), dx.dot(dy), dy.dot(dy)
                r1, r2 = dx.dot(ap), dy.dot(ap); D = g11*g22-g12*g12
                aa = (g22*r1-g12*r2)/D; bb = (g11*r2-g12*r1)/D
                res[nm] = round(math.degrees(math.atan2(bb, aa)), 1)
            hits.append({"obj": on, "n": [round(c,2) for c in n], "px_per_m": round(1/dx.length,0), "py_per_m": round(1/dy.length,0),
                         "shear": round(math.degrees(math.acos(max(-1,min(1,dx.normalized().dot(dy.normalized()))))),1), **res})
    out.append({"pt": (x, y), "hits": hits})
print("RESULT", json.dumps(out))
