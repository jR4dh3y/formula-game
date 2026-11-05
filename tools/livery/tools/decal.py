# Side-projected decals: blender -b --python decal.py -- GLB SPEC.json OUTDIR
# spec: [{"name","logo","Y","Z","width","side":-1|1}]  side=-1 -> right side (x<0)
import bpy, sys, json, os, numpy as np
from mathutils import Vector
glb, spec, outdir = sys.argv[sys.argv.index("--")+1:][:3]
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
S = 4096
P3 = []; UV = []
for ob in bpy.data.objects:
    if ob.type != 'MESH': continue
    me = ob.data; uv = me.uv_layers[0].data; me.calc_loop_triangles(); mw = ob.matrix_world
    for t in me.loop_triangles:
        m = ob.material_slots[t.material_index].material
        if not any('main_body' in (n.image.name if n.type == 'TEX_IMAGE' and n.image else '') for n in m.node_tree.nodes): continue
        P3.append([tuple(mw @ me.vertices[me.loops[l].vertex_index].co) for l in t.loops])
        UV.append([(uv[l].uv.x * S, (1 - uv[l].uv.y) * S) for l in t.loops])
P3 = np.array(P3); UV = np.array(UV)
N = np.cross(P3[:, 1] - P3[:, 0], P3[:, 2] - P3[:, 0]); N /= np.linalg.norm(N, axis=1)[:, None] + 1e-12
C = P3.mean(axis=1)
def load(path):
    im = bpy.data.images.load(path); w, h = im.size
    a = np.array(im.pixels[:], dtype=np.float32).reshape(h, w, 4)[::-1]  # top-down
    return a
os.makedirs(outdir, exist_ok=True)
report = []
for d in json.load(open(spec)):
    logo = load(os.path.abspath(d["logo"])); lh, lw = logo.shape[:2]
    W = d["width"]; H = W * lh / lw
    if "dir" in d:
        D = np.array(d["dir"], float); D /= np.linalg.norm(D); B = np.array(d["base"], float)
        B -= D * B.dot(D); B /= np.linalg.norm(B); C0 = np.array(d["center"], float)
    else:
        side = d["side"]; D = np.array([side, 0.0, 0.0]); B = np.array([0.0, side, 0.0]); C0 = np.array([0.0, d["Y"], d["Z"]])
    U = np.cross(D, B)
    rel = C - C0
    sel = (N @ D > d.get("min_facing", 0.35)) & (np.abs(rel @ B) < W / 2 + 0.03) & (np.abs(rel @ U) < H / 2 + 0.03)
    if "dir" not in d:
        sel &= (C[:, 0] * side > 0.02)
    idx = np.nonzero(sel)[0]
    if len(idx):
        depth = C[idx] @ D; keep = depth > depth.max() - d.get("depth", 0.40)
        idx = idx[keep]
    acc = {}
    for i in idx:
        T = UV[i]; P = P3[i]
        x0, y0 = np.floor(T.min(0)).astype(int); x1, y1 = np.ceil(T.max(0)).astype(int)
        xs, ys = np.meshgrid(np.arange(x0, x1 + 1) + 0.5, np.arange(y0, y1 + 1) + 0.5)
        v0 = T[1] - T[0]; v1 = T[2] - T[0]; den = v0[0] * v1[1] - v1[0] * v0[1]
        if abs(den) < 1e-9: continue
        px = xs - T[0][0]; py = ys - T[0][1]
        u = (px * v1[1] - v1[0] * py) / den; v = (v0[0] * py - px * v0[1]) / den
        inside = (u >= -0.01) & (v >= -0.01) & (u + v <= 1.02)
        if not inside.any(): continue
        u = u[inside]; v = v[inside]
        Q = P[0] + np.outer(u, P[1] - P[0]) + np.outer(v, P[2] - P[0])
        s = ((Q - C0) @ B) / W + 0.5
        t = 0.5 - ((Q - C0) @ U) / H
        ok = (s >= 0) & (s < 1) & (t >= 0) & (t < 1)
        if not ok.any(): continue
        fx = s[ok] * lw - 0.5; fy = t[ok] * lh - 0.5
        ix = np.clip(fx.astype(int), 0, lw - 2); iy = np.clip(fy.astype(int), 0, lh - 2)
        ax = np.clip(fx - ix, 0, 1)[:, None]; ay = np.clip(fy - iy, 0, 1)[:, None]
        col = (logo[iy, ix] * (1 - ax) * (1 - ay) + logo[iy, ix + 1] * ax * (1 - ay) +
               logo[iy + 1, ix] * (1 - ax) * ay + logo[iy + 1, ix + 1] * ax * ay)
        tx = (xs[inside][ok] - 0.5).astype(int); ty = (ys[inside][ok] - 0.5).astype(int)
        for a, b, c in zip(tx, ty, col):
            if (a, b) not in acc or c[3] > acc[(a, b)][3]: acc[(a, b)] = c
    if not acc:
        report.append({"name": d["name"], "error": "no surface"}); continue
    ks = np.array(list(acc.keys())); X0, Yt0 = ks.min(0); X1, Yt1 = ks.max(0)
    w, h = X1 - X0 + 1, Yt1 - Yt0 + 1
    img = np.zeros((h, w, 4), np.float32)
    for (a, b), c in acc.items(): img[b - Yt0, a - X0] = c
    # premultiplied-safe: colour from logo, alpha as is
    out = bpy.data.images.new(d["name"], w, h, alpha=True)
    out.pixels[:] = img[::-1].ravel()
    path = os.path.join(outdir, d["name"].replace(" ", "_") + ".png")
    out.filepath_raw = path; out.file_format = 'PNG'; out.save()
    report.append({"name": d["name"], "path": path, "x": int(X0), "y": int(Yt0), "w": int(w), "h": int(h)})
print("REPORT", json.dumps(report))
