"""Bake texel -> 3D position/normal maps from FP04RB parts-mixer OBJs.

Usage: python3 -I tools/livery/bake_fp04rb.py OBJ_DIR OUT_DIR [RES]
Writes OUT_DIR/<part>.npz with pos (R,R,3) float32, nrm (R,R,3), mask (R,R) bool,
mat (R,R) int8 (index into names), names. Image rows run top-down (v flipped).
"""
import sys
from pathlib import Path
import numpy as np

PARTS = ["main_body", "front_wing", "rear_wing", "mirrors", "floor", "cockpit_foam"]


def load(path):
    vs, vts, tris, tuv, tmat, names = [], [], [], [], [], []
    mat = -1
    for line in open(path):
        if line.startswith("v "):
            vs.append(line.split()[1:4])
        elif line.startswith("vt "):
            vts.append(line.split()[1:3])
        elif line.startswith("usemtl"):
            name = line.split(None, 1)[1].strip()
            if name not in names:
                names.append(name)
            mat = names.index(name)
        elif line.startswith("f "):
            idx = [p.split("/") for p in line.split()[1:]]
            vi = [int(p[0]) - 1 for p in idx]
            ti = [int(p[1]) - 1 if len(p) > 1 and p[1] else -1 for p in idx]
            for k in range(1, len(vi) - 1):
                tris.append((vi[0], vi[k], vi[k + 1]))
                tuv.append((ti[0], ti[k], ti[k + 1]))
                tmat.append(mat)
    return (np.array(vs, float), np.array(vts, float), np.array(tris), np.array(tuv),
            np.array(tmat), names)


def raster_samples(triangles, res):
    """Exact barycentric samples at texel centres; never splat outside a triangle."""
    for face, uv in enumerate(triangles):
        lo = np.maximum(np.ceil(uv.min(0) - .5).astype(int), 0)
        hi = np.minimum(np.floor(uv.max(0) - .5).astype(int), res - 1)
        if (lo > hi).any():
            continue
        e1, e2 = uv[1] - uv[0], uv[2] - uv[0]
        det = e1[0] * e2[1] - e1[1] * e2[0]
        if abs(det) < 1e-12:
            continue
        y, x = np.mgrid[lo[1]:hi[1]+1, lo[0]:hi[0]+1]
        dx, dy = x + .5 - uv[0, 0], y + .5 - uv[0, 1]
        b = (dx * e2[1] - dy * e2[0]) / det
        c = (e1[0] * dy - e1[1] * dx) / det
        a = 1 - b - c
        inside = (a >= -1e-9) & (b >= -1e-9) & (c >= -1e-9)
        if inside.any():
            yield face, x[inside], y[inside], np.stack([a[inside], b[inside], c[inside]], -1)


def bake(path, res):
    v, vt, tris, tuv, tmat, names = load(path)
    keep = (tuv >= 0).all(1)
    tris, tuv, tmat = tris[keep], tuv[keep], tmat[keep]
    P = v[tris]                                   # (T,3,3)
    UV = vt[tuv] * [res, -res] + [0, res]         # pixel coords, y down
    n = np.cross(P[:, 1] - P[:, 0], P[:, 2] - P[:, 0])
    n /= np.linalg.norm(n, axis=1, keepdims=True) + 1e-12
    pos = np.zeros((res, res, 3), np.float32)
    nrm = np.zeros((res, res, 3), np.float32)
    mask = np.zeros((res, res), bool)
    matimg = np.full((res, res), -1, np.int8)
    for face, x, y, weights in raster_samples(UV, res):
        pos[y, x] = weights @ P[face]
        nrm[y, x] = n[face]
        mask[y, x] = True
        matimg[y, x] = tmat[face]
    return pos, nrm, mask, matimg, names


if __name__ == "__main__":
    src, out = Path(sys.argv[1]), Path(sys.argv[2])
    res = int(sys.argv[3]) if len(sys.argv) > 3 else 1024
    out.mkdir(parents=True, exist_ok=True)
    for part in PARTS:
        pos, nrm, mask, matimg, names = bake(src / f"fp04rb_{part}.obj", res)
        np.savez_compressed(out / f"{part}.npz", pos=pos, nrm=nrm, mask=mask, mat=matimg,
                            names=np.array(names))
        print(part, "coverage", round(mask.mean(), 3), names, flush=True)
