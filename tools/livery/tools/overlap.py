# texels used by >1 distinct surface (3D positions > 4 cm apart) -> magenta; single -> grey; unused -> black
import bpy, sys, numpy as np
glb, out = sys.argv[sys.argv.index("--")+1:][:2]
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
R = 1024; S = R
first = np.full((R, R, 3), np.nan, np.float32); multi = np.zeros((R, R), bool); used = np.zeros((R, R), bool)
for ob in bpy.data.objects:
    if ob.type != 'MESH': continue
    me = ob.data; uv = me.uv_layers[0].data; me.calc_loop_triangles(); mw = ob.matrix_world
    for t in me.loop_triangles:
        m = ob.material_slots[t.material_index].material
        if not any('main_body' in (n.image.name if n.type == 'TEX_IMAGE' and n.image else '') for n in m.node_tree.nodes): continue
        P = np.array([tuple(mw @ me.vertices[me.loops[l].vertex_index].co) for l in t.loops])
        T = np.array([(uv[l].uv.x * S, (1 - uv[l].uv.y) * S) for l in t.loops])
        x0, y0 = np.maximum(np.floor(T.min(0)).astype(int), 0); x1, y1 = np.minimum(np.ceil(T.max(0)).astype(int), R - 1)
        if x1 < x0 or y1 < y0: continue
        xs, ys = np.meshgrid(np.arange(x0, x1 + 1) + .5, np.arange(y0, y1 + 1) + .5)
        v0 = T[1] - T[0]; v1 = T[2] - T[0]; d = v0[0] * v1[1] - v1[0] * v0[1]
        if abs(d) < 1e-12: continue
        px = xs - T[0, 0]; py = ys - T[0, 1]
        u = (px * v1[1] - v1[0] * py) / d; v = (v0[0] * py - px * v0[1]) / d
        ins = (u >= 0) & (v >= 0) & (u + v <= 1)
        if not ins.any(): continue
        Q = P[0] + np.outer(u[ins], P[1] - P[0]) + np.outer(v[ins], P[2] - P[0])
        X = (xs[ins] - .5).astype(int); Y = (ys[ins] - .5).astype(int)
        f = first[Y, X]; new = np.isnan(f[:, 0])
        first[Y[new], X[new]] = Q[new]
        far = (~new) & (np.linalg.norm(f - Q, axis=1) > 0.04)
        multi[Y[far], X[far]] = True; used[Y, X] = True
img = np.zeros((R, R, 4), np.float32); img[..., 3] = 1
img[used] = (0.55, 0.55, 0.55, 1); img[multi] = (1, 0, 1, 1)
print("overlap texels", multi.sum(), "used", used.sum())
im = bpy.data.images.new("ov", R, R); im.pixels[:] = img[::-1].ravel(); im.filepath_raw = out; im.file_format = 'PNG'; im.save()
