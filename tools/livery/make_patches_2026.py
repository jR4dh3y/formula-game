"""Author tools/livery/patches_2026.json: Monoposto 2026 template -> FP04RB main body.

FP04RB's main-body UVs are three planar projections: island 19 is a side view of the car's
right side, 17 the left side and 18 a top view of the cockpit, sidepod tops and nose.
Control points are written in those views:
  S(X, Y)  outermost texel seen from the car's right side,
  T(X, Z)  topmost body texel seen from above (halo excluded),
  B(X)     the right side's bottom (floor) border at X; R the rear end.
Template coordinates describe panel boundaries on the official template. No sponsor
locations constrain the solve. The spec describes the car's right side
(template u < 1024); build_map_v3.py mirrors it.

Usage: python3 -I tools/livery/make_patches_2026.py [--out tools/livery/patches_2026.json]
       python3 -I tools/livery/make_patches_2026.py --merge-halo MAP_V3.npz  (after building)
"""
import argparse
import json
from pathlib import Path

import numpy as np
from scipy import ndimage

REPO = Path(__file__).resolve().parents[2]
BAKE = REPO / "raw/gen22/livery-mapping/bake2k/main_body.npz"
BASE = REPO / "raw/gen22/livery-mapping/map2026.npz"
OUT = REPO / "tools/livery/patches_2026.json"
STEP = 0.12  # spacing of via points in body units

# Surfaces kept on the previous mapping (the halo uses its own polar unroll).
KEEP_OLD = ("halo",)


class Body:
    def __init__(self, bake_path=BAKE):
        b = np.load(bake_path)
        self.mask, self.pos = b["mask"], b["pos"]
        lab, _ = ndimage.label(self.mask)
        self.lab = lab
        self.side_r = lab[1800, 1000]            # right-side projection island
        body = np.isin(lab, [lab[1800, 1000], lab[950, 1000], lab[1400, 1500]])
        self.body = body
        r, c = np.nonzero(body)
        self.rc = np.stack([c, r], 1)
        self.P = self.pos[r, c]
        self.N = b["nrm"][r, c]
        # Only texels on the main connected piece of build_map_v3's texel graph.
        import sys
        sys.path.insert(0, str(Path(__file__).parent))
        from build_map_v3 import Mesh
        from scipy.sparse import csgraph
        mesh = Mesh(self.pos, self.mask, self.mask)
        _, comp = csgraph.connected_components(mesh.A, directed=False)
        main = np.bincount(comp).argmax()
        self.ok = comp[mesh.node_of[r, c]] == main
        self.okgrid = np.zeros_like(self.mask)
        self.okgrid[r[self.ok], c[self.ok]] = True

    def _pick(self, a, b, x, y, key, extra=None):
        P = self.P
        for tol in (0.03, 0.06, 0.1):
            sel = (np.abs(P[:, a] - x) < tol) & (np.abs(P[:, b] - y) < tol) & self.ok
            if extra is not None:
                sel &= extra
            idx = np.flatnonzero(sel)
            if idx.size:
                return [int(v) for v in self.rc[idx[np.argmax(key[idx])]]]
        raise ValueError(f"no texel near ({x}, {y})")

    def S(self, x, y):
        # Facing the viewer: steep texels of a projection are cut off the texel graph.
        return self._pick(0, 1, x, y, self.P[:, 2], (self.P[:, 2] > 0) & (np.abs(self.N[:, 2]) > 0.35))

    def T(self, x, z):
        return self._pick(0, 2, x, z, self.P[:, 1], np.abs(self.N[:, 1]) > 0.35)

    def XYZ(self, x, y, z):
        d = np.linalg.norm(self.P - [x, y, z], axis=1)
        d[~self.ok] = np.inf
        i = int(np.argmin(d))
        return [int(v) for v in self.rc[i]]

    def B(self, x):
        """Lowest texel of the right-side island at X (the floor border)."""
        isl = (self.lab == self.side_r) & self.okgrid
        col = int(round(867.4 + 157.4 * x))
        rows = np.nonzero(isl[:, col])[0]
        return [col, int(rows.max())]

    def R(self, f):
        """Rear-end border of the right-side island, f=0 bottom (X -4.5) .. 1 spine."""
        isl = (self.lab == self.side_r) & self.okgrid
        x = -4.5 + f * (-5.10 + 4.5)
        col = int(round(867.4 + 157.4 * x))
        rows = np.nonzero(isl[:, col])[0]
        r = int(rows.max() + f * (rows.min() - rows.max()))
        return [col, r]


def run(fn, a, b, step=STEP):
    """Via points from a to b (exclusive of both ends)."""
    n = max(2, int(abs(b - a) / step) + 1)
    out = []
    for v in np.linspace(a, b, n)[1:-1]:
        try:
            out.append(fn(v))
        except ValueError:      # e.g. grazing texels cut off the graph; skip the via point
            pass
    return out


def pt(uv, t=None, edge="line"):
    d = {"uv": uv}
    if t is not None:
        d["t"] = [float(t[0]), float(t[1])]
        d["edge"] = edge
    return d


def vias(uvs):
    return [pt(uv) for uv in uvs]


# Engine cover / sidepod boundary: the shoulder line (top view, X Z), then down the
# coke-bottle flank (side view, X Y) to the floor at X -4.5.
CREASE_TOP = [(-1.1, 0.8), (-1.6, 0.75), (-2.1, 0.72), (-2.6, 0.7), (-3.0, 0.68)]
CREASE_SIDE = [(-3.5, 1.3), (-3.95, 1.0), (-4.3, 0.75)]



def crease(body, forward):
    """Crease via points from C1 backwards (forward=False) or towards C1 (forward=True)."""
    pts = [body.T(x, z) for x, z in CREASE_TOP] + [body.S(x, y) for x, y in CREASE_SIDE]
    return pts if not forward else pts[::-1]


def build(body):
    T, S, B = body.T, body.S, body.B
    C1 = T(-0.55, 0.85)     # shoulder: engine cover / sidepod / cockpit side meet
    C2 = T(1.25, 0.85)      # sidepod front, inner (beside the cockpit)
    C3 = B(1.0)             # sidepod front at the floor
    C4 = B(-4.5)            # crease reaches the floor at the rear
    A = T(-0.6, 0.0)        # spine at the airbox front
    R0 = body.XYZ(-0.5, 1.97, 0.0)  # cockpit opening rear, centre
    CF = T(1.65, 0.12)      # cockpit opening front
    TIP = T(6.97, 0.0)

    rails = [T(x, 0.85) for x in np.linspace(-0.4, 1.1, 8)]           # C1 -> C2
    front = [T(1.45, 0.9), T(1.5, 1.3), S(1.5, 1.25), S(1.35, 1.0), S(1.15, 0.7)]  # C2 -> C3
    floor_back = run(B, 1.0, -4.5)                                     # C3 -> C4
    floor_nose = run(B, 6.9, 1.0)                                      # tip side -> C3
    spine = run(lambda x: T(x, 0.0), -5.08, -0.6)                      # rear -> A
    rear = [B(x) for x in np.linspace(-4.6, -5.08, 5)]                # C4 -> spine rear
    REAR = body.R(1.0)
    rim = [T(-0.45, 0.4), T(-0.3, 0.66), T(0.1, 0.68), T(0.5, 0.66), T(0.9, 0.62),
           T(1.3, 0.56), T(1.5, 0.45)]                                              # R0 -> CF
    centre = [T(2.0, 0.22), T(2.45, 0.05)] + run(lambda x: T(x, 0.0), 2.5, 6.97)                      # CF -> tip

    engine_cover = {
        "name": "engine_cover_R", "solver": "triangles", "island": 26, "mirror": True, "mirror_island": 26,
        "seed": {"uv": S(-3.0, 1.7)},
        "loop": [pt(REAR, (982, 24), "contour"), *vias(spine),
                 pt(A, (1020, 598), "line"), *vias([T(-0.6, 0.3), T(-0.58, 0.55)]),
                 pt(C1, (700, 640), "contour"), *vias(crease(body, False)),
                 pt(C4, (694, 28), "contour"), *vias(rear)],

    }
    sidepod = {
        "name": "sidepod_R", "island": 21, "mirror": True,
        "seed": {"uv": S(-1.0, 0.9)},
        "loop": [pt(C1, (657, 857), "line"), *vias(rails),
                 pt(C2, (669, 1145), "contour"), *vias(front),
                 pt(C3, (400, 1262), "contour"), *vias(floor_back[:len(floor_back) // 2]),
                 pt(B(-1.75), (262, 640), "contour"), *vias(floor_back[len(floor_back) // 2 + 1:]),
                 pt(C4, (484, 159), "contour"), *vias(crease(body, True))],

    }
    nose = {
        "name": "nose_cockpit_R", "island": 26, "mirror": True, "mirror_island": 26,
        "seed": {"uv": T(3.5, 0.15)},
        "loop": [pt(CF, (1022, 1082), "line"), *vias(centre),
                 pt(TIP, (1022, 2040), "contour"), *vias(floor_nose),
                 pt(C3, (687, 1092), "line"), *vias(front[::-1]),
                 pt(C2, (834, 1080), "line"), *vias(rails[::-1]),
                 pt(C1, (834, 650), "line"), *vias([T(-0.58, 0.55), T(-0.6, 0.3)]),
                 pt(A, (1020, 610), "line"),
                 pt(R0, (1022, 655), "contour"), *vias(rim)],
    }
    return {"patches": [engine_cover, sidepod, nose], "fallback": "nearest"}


def merge_halo(path, base_path=BASE):
    """Keep the previous mapping on texels the old map labelled as halo."""
    base = np.load(base_path)
    new = dict(np.load(path))
    names = list(base["names"])
    keep = base["main_body_halo_mask"]
    mapped = keep & np.isfinite(base["main_body_xy"][...,0])
    new["main_body_xy"][keep] = base["main_body_xy"][keep]
    patch_names = [n for n in new["main_body_names"] if n != "halo"]
    new["main_body_region"][keep] = -1
    new["main_body_region"][mapped] = len(patch_names)
    new["main_body_names"] = np.array(patch_names + ["halo"])
    new["main_body_snap"][keep] = base["main_body_snap"][keep]
    new["main_body_fill"][keep] = base["main_body_fill"][keep]
    np.savez_compressed(path, **new)
    print(f"kept the previous mapping on {int(keep.sum())} halo texels")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=OUT)
    ap.add_argument("--merge-halo", type=Path)
    ap.add_argument("--work", type=Path, default=BAKE.parent.parent)
    a = ap.parse_args()
    if a.merge_halo:
        merge_halo(a.merge_halo, a.work / "map2026.npz")
        return
    spec = build(Body(a.work / "bake2k/main_body.npz"))
    a.out.write_text(json.dumps(spec, indent=1))
    print("wrote", a.out, [len(p["loop"]) for p in spec["patches"]])


if __name__ == "__main__":
    main()
