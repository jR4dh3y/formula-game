# per-texel 3D position + normal for one texture: blender -b --python posmap.py -- GLB MAT RES OUT.npz
import bpy, sys, numpy as np
glb, mat, R, out = sys.argv[sys.argv.index("--")+1:][:4]; R = int(R)
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
pos = np.full((R, R, 3), np.nan, np.float32); nrm = np.zeros((R, R, 3), np.float32)
cnt = np.zeros((R, R), np.uint8)
for ob in bpy.data.objects:
    if ob.type != 'MESH': continue
    me = ob.data; uv = me.uv_layers[0].data; me.calc_loop_triangles(); mw = ob.matrix_world
    me.calc_normals_split() if hasattr(me, "calc_normals_split") else None
    for t in me.loop_triangles:
        m = ob.material_slots[t.material_index].material
        if not any(mat in (n.image.name if n.type == 'TEX_IMAGE' and n.image else '') for n in m.node_tree.nodes): continue
        P = np.array([tuple(mw @ me.vertices[me.loops[l].vertex_index].co) for l in t.loops])
        T = np.array([(uv[l].uv.x * R, (1 - uv[l].uv.y) * R) for l in t.loops])
        n = np.cross(P[1] - P[0], P[2] - P[0]); ln = np.linalg.norm(n)
        if ln == 0: continue
        n /= ln
        x0, y0 = np.maximum(np.floor(T.min(0)).astype(int), 0); x1, y1 = np.minimum(np.ceil(T.max(0)).astype(int), R - 1)
        if x1 < x0 or y1 < y0: continue
        xs, ys = np.meshgrid(np.arange(x0, x1 + 1) + .5, np.arange(y0, y1 + 1) + .5)
        v0 = T[1] - T[0]; v1 = T[2] - T[0]; d = v0[0] * v1[1] - v1[0] * v0[1]
        if abs(d) < 1e-12: continue
        px = xs - T[0, 0]; py = ys - T[0, 1]
        u = (px * v1[1] - v1[0] * py) / d; v = (v0[0] * py - px * v0[1]) / d
        ins = (u >= -0.02) & (v >= -0.02) & (u + v <= 1.04)
        if not ins.any(): continue
        Q = P[0] + np.outer(u[ins], P[1] - P[0]) + np.outer(v[ins], P[2] - P[0])
        X = (xs[ins] - .5).astype(int); Y = (ys[ins] - .5).astype(int)
        pos[Y, X] = Q; nrm[Y, X] = n; cnt[Y, X] = np.minimum(cnt[Y, X] + 1, 255)
np.savez_compressed(out, pos=pos, nrm=nrm, cnt=cnt)
print("done", np.isfinite(pos[..., 0]).sum())
