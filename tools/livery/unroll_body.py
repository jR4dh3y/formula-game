"""Per-texel unrolled body coordinates for FP04RB main_body.

s = surface (geodesic) distance from the top centreline, seeded on the spine and the
cockpit rim (rim seeds start at their lateral offset), signed by side (+Z = left).
Usage: python3 -I tools/livery/unroll_body.py OBJ OUT.npz [RES]
"""
import sys
import numpy as np
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import dijkstra

sys.path.insert(0, str(__import__("pathlib").Path(__file__).resolve().parent))
import bake_fp04rb as bake_obj  # noqa: E402

obj, out = sys.argv[1], sys.argv[2]
res = int(sys.argv[3]) if len(sys.argv) > 3 else 1024
v, vt, tris, tuv, tmat, names = bake_obj.load(obj)
n = len(v)
e = np.concatenate([tris[:, [0, 1]], tris[:, [1, 2]], tris[:, [2, 0]]])
e = np.unique(np.sort(e, 1), axis=0)
w = np.linalg.norm(v[e[:, 0]] - v[e[:, 1]], axis=1)

# Vertex normals for "top facing" test.
fn = np.cross(v[tris[:, 1]] - v[tris[:, 0]], v[tris[:, 2]] - v[tris[:, 0]])
vn = np.zeros_like(v)
for k in range(3):
    np.add.at(vn, tris[:, k], fn)
vn /= np.linalg.norm(vn, axis=1, keepdims=True) + 1e-12

X, Y, Z = v.T
spine = (np.abs(Z) < 0.03) & (vn[:, 1] > 0.3)
# Keep only the upper-most spine vertex band at each X (exclude the chassis underside).
bins = np.round(X / 0.05).astype(int)
top = np.zeros(n, bool)
for b in np.unique(bins[spine]):
    idx = np.nonzero(spine & (bins == b))[0]
    top[idx[Y[idx] > Y[idx].max() - 0.08]] = True
# Boundary (open) edges: cockpit rim and other openings. Rim = boundary verts inside cockpit X range,
# high up and near the centre.
fe = np.concatenate([tris[:, [0, 1]], tris[:, [1, 2]], tris[:, [2, 0]]])
fe = np.sort(fe, 1)
_, inv, cnt = np.unique(fe, axis=0, return_inverse=True, return_counts=True)
bnd = np.unique(fe[cnt[inv] == 1])
rimmask = np.zeros(n, bool)
rimmask[bnd] = True
rimmask &= (np.abs(Z) < 0.55) & (Y > 1.4) & (X > -0.45) & (X < 1.5) & (Y < 1.7)
top &= ~((X > -0.45) & (X < 1.5))
seeds = np.nonzero(top | rimmask)[0]
init = np.where(top[seeds], 0.0, np.abs(Z[seeds]))
print("verts", n, "spine seeds", top.sum(), "rim seeds", rimmask.sum(), flush=True)

src = n
rows = np.concatenate([e[:, 0], e[:, 1], np.full(len(seeds), src)])
cols = np.concatenate([e[:, 1], e[:, 0], seeds])
wts = np.concatenate([w, w, init + 1e-9])
G = coo_matrix((wts, (rows, cols)), shape=(n + 1, n + 1)).tocsr()
d = dijkstra(G, directed=True, indices=src)[:n]
# Smooth graph-metric zig-zags; seeds stay fixed.
from scipy.sparse import diags
A = coo_matrix((np.ones(2 * len(e)), (np.r_[e[:, 0], e[:, 1]], np.r_[e[:, 1], e[:, 0]])), shape=(n, n)).tocsr()
deg = np.asarray(A.sum(1)).ravel()
Pm = diags(1 / np.maximum(deg, 1)) @ A
fixed = np.zeros(n, bool); fixed[seeds] = True
for _ in range(60):
    d = np.where(fixed, d, Pm @ d)
s = np.sign(Z + 1e-9) * d
print("s range", np.nanmin(s), np.nanmax(s), flush=True)

# Rasterise s into texel space, matching bake_obj's conventions.
keep = (tuv >= 0).all(1)
tris_k, tuv_k = tris[keep], tuv[keep]
UV = vt[tuv_k] * [res, -res] + [0, res]
S = s[tris_k]
img = np.full((res, res), np.nan, np.float32)
for face, x, y, weights in bake_obj.raster_samples(UV, res):
    img[y, x] = weights @ S[face]
np.savez_compressed(out, s=img, vs=s, spine=top, rim=rimmask)
