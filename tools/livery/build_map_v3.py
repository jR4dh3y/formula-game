"""Build the main_body template map from harmonic patches with 3D control loops.

Usage (from the repo root):
  python3 -I tools/livery/build_map_v3.py --spec tools/livery/patches_2026.json \
      [--out raw/gen22/livery-mapping/map_v3.npz] [--res 2048]

Only main_body is re-solved. Every other part is copied verbatim from --base (map2026.npz).

Spec (JSON):
  {"patches": [{"name": str, "island": int (template id, car-right side),
                "mirror": bool (also generate the car-left copy),
                "seed": {"uv": [x, y]} or {"xyz": [X, Y, Z]},      # inside the patch
                "loop": [{"uv"|"xyz": ..., "t": [u, v],             # closed, in order
                          "edge": "line" | "contour"}, ...],         # edge = this -> next
                "interior": [{"uv"|"xyz": ..., "t": [u, v]}, ...]}],  # optional hard constraints
   "fallback": "nearest" | "none"}
uv = FP04RB texel px (2048 space, x right, y down); t = template px (2048 space).
"line" edges interpolate t linearly by 3D arc length; "contour" edges walk the boundary of
the patch's template island, the shorter way round, by arc length.

Texel graph: masked main_body texels, 4-neighbour edges (cut where 3D length >= 3x the
median), plus seam links between 3D-close texels on UV-island borders. Harmonic weights
are cotangents of the local 3D triangles, integrating surface Dirichlet energy;
path weights are the 3D lengths. Loop geodesics are 3D shortest paths between control
texels, so control points must be dense enough that each path stays on its own chart.

Outputs: <out>.npz, <out stem>_uv_patches.png, <out stem>_template_coverage.png and
<out>.patches.json (per-patch stats). main_body_region holds the patch index in the
expanded patch list (mirrored patches follow their source patch).
"""
import argparse
import colorsys
import json
import time
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage, sparse
from scipy.sparse import csgraph
from scipy.sparse.linalg import cg
from scipy.spatial import cKDTree

REPO = Path(__file__).resolve().parents[2]
DEFAULT_BAKE = REPO / "raw/gen22/livery-mapping/bake2k/main_body.npz"
DEFAULT_ISLANDS = REPO / "raw/gen22/livery-mapping/islands.npy"
DEFAULT_BASE = REPO / "raw/gen22/livery-mapping/map2026.npz"
DEFAULT_OUT = REPO / "raw/gen22/livery-mapping/map_v3.npz"

CUT_FACTOR = 3.0     # 4-neighbour edges longer than this x median are UV cuts (grazing projection texels stay linked)
SEAM_FACTOR = 1.5     # seam links reach this x median 3D spacing
MAX_LEAK = 0.6        # a flood fill covering more of the mesh means the loop is open
CG_RTOL = 1e-8
CG_MAXITER = 50000


# ---------------------------------------------------------------- texel graph

def _grid_pairs(mask, node_of):
    """Node ids of all 4-neighbour pairs inside the mask (horizontal, then vertical)."""
    r, c = np.nonzero(mask[:, :-1] & mask[:, 1:])
    r2, c2 = np.nonzero(mask[:-1] & mask[1:])
    a = np.concatenate([node_of[r, c], node_of[r2, c2]])
    b = np.concatenate([node_of[r, c + 1], node_of[r2 + 1, c2]])
    return a, b


def surface_weights(pos, node_of, med, seam_a, seam_b):
    """Cotangent FEM weights integrate Dirichlet energy on the actual 3D surface.

    Inverse edge length alone treats unequal planar-UV sampling as physical
    stretch. Triangles supply the missing dual area and shear terms.
    """
    p = node_of[:-1, :-1].ravel()
    q = node_of[:-1, 1:].ravel()
    r = node_of[1:, :-1].ravel()
    s = node_of[1:, 1:].ravel()
    tris = np.concatenate([np.stack([p,q,s],1), np.stack([p,s,r],1)])
    tris = tris[(tris >= 0).all(1)]
    points = pos[node_of >= 0].astype(float)
    xyz = points[tris]
    edges = xyz - np.roll(xyz, 1, axis=1)
    keep = (np.linalg.norm(edges,axis=-1) < CUT_FACTOR * med * 1.5).all(1)
    weights = cotangent_weights(points, tris[keep])
    rows, cols = np.r_[seam_a,seam_b], np.r_[seam_b,seam_a]
    return weights + sparse.csr_matrix((np.ones(len(rows)),(rows,cols)),shape=weights.shape)


def cotangent_weights(points, triangles):
    """Symmetric FEM edge weights; signed cotangents retain the surface metric."""
    xyz = points[triangles]
    rows, cols, weights = [], [], []
    for corner in range(3):
        j, k = (corner+1)%3, (corner+2)%3
        u, v = xyz[:,j]-xyz[:,corner], xyz[:,k]-xyz[:,corner]
        area2 = np.linalg.norm(np.cross(u,v),axis=1)
        cot = np.einsum("ij,ij->i",u,v) / np.maximum(area2,1e-15) / 2
        rows.extend([triangles[:,j],triangles[:,k]])
        cols.extend([triangles[:,k],triangles[:,j]])
        weights.extend([cot,cot])
    n = len(points)
    return sparse.csr_matrix((np.concatenate(weights),(np.concatenate(rows),np.concatenate(cols))),shape=(n,n))


class Mesh:
    """Graph over the masked texels of one grid. stride maps grid index -> 2048 px."""

    def __init__(self, pos, mask, paint, stride=1):
        self.stride = stride
        self.shape = mask.shape
        self.node_of = np.full(mask.shape, -1, np.int64)
        r, c = np.nonzero(mask)
        self.N = r.size
        self.node_of[r, c] = np.arange(self.N)
        self.pix = np.stack([r, c], 1)
        self.P = pos[r, c].astype(np.float64)
        self.paint = paint[r, c]
        self.edt_idx = None

        a, b = _grid_pairs(mask, self.node_of)
        length = np.linalg.norm(self.P[a] - self.P[b], axis=1)
        self.med = float(np.median(length))
        keep = length < CUT_FACTOR * self.med
        self.n_cut = int((~keep).sum())

        # Seam links: 3D-close border texels that are not UV neighbours.
        border = mask & ~ndimage.binary_erosion(mask)
        border_nodes = np.unique(np.concatenate([self.node_of[border], a[~keep], b[~keep]]))
        pairs = cKDTree(self.P[border_nodes]).query_pairs(SEAM_FACTOR * self.med, output_type="ndarray")
        sa, sb = border_nodes[pairs[:, 0]], border_nodes[pairs[:, 1]]
        uv_gap = np.abs(self.pix[sa] - self.pix[sb]).max(1)
        ok = uv_gap > 1
        sa, sb = sa[ok], sb[ok]
        existing = np.unique(np.minimum(a[keep], b[keep]) * self.N + np.maximum(a[keep], b[keep]))
        new = ~np.isin(np.minimum(sa, sb) * self.N + np.maximum(sa, sb), existing)
        sa, sb = sa[new], sb[new]
        self.n_seam = int(sa.size)

        ea = np.concatenate([a[keep], sa])
        eb = np.concatenate([b[keep], sb])
        el = np.maximum(np.linalg.norm(self.P[ea] - self.P[eb], axis=1), 1e-6 * self.med)
        rows, cols = np.concatenate([ea, eb]), np.concatenate([eb, ea])
        ln = np.concatenate([el, el])
        shape = (self.N, self.N)
        self.G_len = sparse.csr_matrix((ln, (rows, cols)), shape=shape)
        self.G_harm = surface_weights(pos, self.node_of, self.med, sa, sb)
        # One-pixel chart slivers have no triangles. Retain their graph links.
        isolated = np.asarray(self.G_harm.getnnz(axis=1)) == 0
        sliver = isolated[rows] | isolated[cols]
        self.G_harm += sparse.csr_matrix((np.ones(sliver.sum()), (rows[sliver], cols[sliver])), shape=shape)
        self.A = sparse.csr_matrix((np.ones(ln.size, np.int8), (rows, cols)), shape=shape)
        # Snap points only to the main connected piece (grazing projection texels are cut off).
        _, comp = csgraph.connected_components(self.A, directed=False)
        self.main_nodes = np.flatnonzero(comp == np.bincount(comp).argmax())
        self.kd = cKDTree(self.P[self.main_nodes])

    def node_from_uv(self, uv):
        """Node at 2048-space pixel uv = (x, y); masked-nearest if that pixel is not in the mesh."""
        R, C = self.shape
        r = int(np.clip(round(uv[1] / self.stride), 0, R - 1))
        c = int(np.clip(round(uv[0] / self.stride), 0, C - 1))
        n = self.node_of[r, c]
        if n < 0:
            if self.edt_idx is None:
                _, self.edt_idx = ndimage.distance_transform_edt(self.node_of < 0, return_indices=True)
            n = self.node_of[self.edt_idx[0][r, c], self.edt_idx[1][r, c]]
        return int(n)

    def node_from_xyz(self, xyz):
        return int(self.main_nodes[self.kd.query(np.asarray(xyz, float))[1]])


def walk(pred, src, dst):
    out = [dst]
    while out[-1] != src:
        p = int(pred[out[-1]])
        if p < 0:
            raise RuntimeError("broken predecessor chain")
        out.append(p)
    return out[::-1]


def path_graph(mesh):
    """G_len plus UV-diagonal links. On a 4-neighbour grid every monotone staircase has
    the same length, so Dijkstra returns coarse L-shaped steps that show on the car as a
    sawtooth patch border. Diagonals let paths follow the straight 3D route."""
    if getattr(mesh, "G_path", None) is None:
        no = mesh.node_of
        pairs = []
        for a, b, c, d in ((no[:-1, :-1], no[1:, 1:], no[:-1, 1:], no[1:, :-1]),
                           (no[:-1, 1:], no[1:, :-1], no[:-1, :-1], no[1:, 1:])):
            ok = (a >= 0) & (b >= 0) & ((c >= 0) | (d >= 0))
            pairs.append((a[ok], b[ok]))
        ea = np.concatenate([p[0] for p in pairs]); eb = np.concatenate([p[1] for p in pairs])
        el = np.linalg.norm(mesh.P[ea] - mesh.P[eb], axis=1)
        keep = el < CUT_FACTOR * mesh.med * np.sqrt(2)
        ea, eb, el = ea[keep], eb[keep], np.maximum(el[keep], 1e-6 * mesh.med)
        diag = sparse.csr_matrix((np.r_[el, el], (np.r_[ea, eb], np.r_[eb, ea])), shape=mesh.G_len.shape)
        # Keep existing (4-neighbour and seam) lengths where both are present.
        mesh.G_path = mesh.G_len + diag - diag.multiply(mesh.G_len.astype(bool))
    return mesh.G_path


def four_connect(mesh, path):
    """Insert a shared 4-neighbour between UV-diagonal steps so the loop stays a barrier
    for the 4-connected flood fill."""
    out = [path[0]]
    for a, b in zip(path[:-1], path[1:]):
        (ra, ca), (rb, cb) = mesh.pix[a], mesh.pix[b]
        if abs(ra - rb) == 1 and abs(ca - cb) == 1:
            corners = [int(mesh.node_of[r, c]) for r, c in ((ra, cb), (rb, ca))]
            corners = [n for n in corners if n >= 0]
            linked = [n for n in corners if mesh.A[a, n] and mesh.A[n, b]]
            if linked or corners:
                out.append((linked or corners)[0])
        out.append(int(b))
    return np.array(out)


def mesh_path(mesh, a, b, name):
    texels = hasattr(mesh, "node_of")  # refine_mesh.py passes a triangle-vertex graph
    graph = path_graph(mesh) if texels else mesh.G_len
    dist, pred = csgraph.dijkstra(graph, directed=True, indices=a, return_predecessors=True)
    if not np.isfinite(dist[b]):
        raise ValueError(f"{name}: control texels {a} and {b} are not connected on the texel graph")
    path = walk(pred, a, b)
    return four_connect(mesh, path) if texels else np.array(path)


def flood(mesh, loop, seed):
    """Nodes 4/seam-connected to seed without crossing the loop."""
    keep = np.ones(mesh.N, bool)
    keep[loop] = False
    if not keep[seed]:
        raise ValueError("seed lies on the loop")
    idx = np.flatnonzero(keep)
    _, lab = csgraph.connected_components(mesh.A[idx][:, idx], directed=False)
    return idx[lab == lab[np.searchsorted(idx, seed)]]


def fill_patch(mesh, loop, seed, name):
    """Interior of the loop. Thickens the loop by one texel ring once if the fill leaks."""
    fill = flood(mesh, loop, seed)
    if fill.size <= MAX_LEAK * mesh.N:
        return fill, loop, False
    ring = np.union1d(loop, mesh.A[loop].indices)
    fill = flood(mesh, ring, seed)
    if fill.size > MAX_LEAK * mesh.N:
        raise ValueError(f"{name}: loop does not close; the flood from the seed reaches "
                         f"{100 * fill.size / mesh.N:.0f}% of the mesh even with a thickened loop")
    return fill, ring, True


def nearest_values(mesh, src, vals, targets):
    """Values of the nearest src node (texel-graph distance) for each target node."""
    _, _, sources = csgraph.dijkstra(mesh.G_len, indices=src, min_only=True, return_predecessors=True)
    lookup = np.full(mesh.N, -1, np.int64)
    lookup[src] = np.arange(src.size)
    s = sources[targets]
    if (s < 0).any():
        raise RuntimeError("target texels unreachable from the source set")
    return vals[lookup[s]]


# ---------------------------------------------------------------- template islands

class IslandShape:
    """One template island: mask, boundary graph for contour edges, clamp lookup."""

    def __init__(self, islands, iid):
        self.iid = iid
        self.mask = islands == iid
        if not self.mask.any():
            raise ValueError(f"island {iid} is not in the template")
        bp = np.argwhere(self.mask & ~ndimage.binary_erosion(self.mask))
        self.bxy = bp[:, ::-1].astype(float)
        lab = np.full(self.mask.shape, -1, np.int64)
        lab[bp[:, 0], bp[:, 1]] = np.arange(len(bp))
        H, W = self.mask.shape
        rows, cols, w = [], [], []
        for i, (y, x) in enumerate(bp):
            for dy in (-1, 0, 1):
                for dx in (-1, 0, 1):
                    yy, xx = y + dy, x + dx
                    if (dy or dx) and 0 <= yy < H and 0 <= xx < W and lab[yy, xx] >= 0:
                        rows.append(i); cols.append(lab[yy, xx]); w.append(np.hypot(dy, dx))
        n = len(bp)
        self.G = sparse.csr_matrix((w, (rows, cols)), shape=(n, n))
        self.tree = cKDTree(self.bxy)
        self._edt = None

    def arc(self, ta, tb, f):
        """Template points at arc fraction f along the shorter boundary arc from ta to tb."""
        ia = int(self.tree.query(ta)[1])
        ib = int(self.tree.query(tb)[1])
        dist, pred = csgraph.dijkstra(self.G, indices=ia, return_predecessors=True)
        if not np.isfinite(dist[ib]):
            raise ValueError(f"island {self.iid}: boundary points {ta} and {tb} are not connected")
        poly = np.vstack([ta, self.bxy[walk(pred, ia, ib)], tb])
        s = np.concatenate([[0.0], np.cumsum(np.linalg.norm(np.diff(poly, axis=0), axis=1))])
        s = s / s[-1] if s[-1] > 0 else np.linspace(0, 1, len(s))
        return np.stack([np.interp(f, s, poly[:, 0]), np.interp(f, s, poly[:, 1])], 1)

    def clamp(self, xy):
        """Move points outside the island to its nearest pixel. Returns (xy, percent clamped)."""
        H, W = self.mask.shape
        r = np.clip(np.rint(xy[:, 1]).astype(int), 0, H - 1)
        c = np.clip(np.rint(xy[:, 0]).astype(int), 0, W - 1)
        inside = self.mask[r, c]
        out = xy.copy()
        if not inside.all():
            if self._edt is None:
                _, self._edt = ndimage.distance_transform_edt(~self.mask, return_indices=True)
            ir, ic = self._edt[0][r, c], self._edt[1][r, c]
            out[~inside] = np.stack([ic, ir], 1)[~inside]
        return out, 100.0 * float((~inside).mean())


def mirror_island(islands, iid):
    """Island id at the mirrored position (u -> W-1-u) of the island's centroid."""
    cy, cx = ndimage.center_of_mass(islands == iid)
    r = int(np.clip(round(cy), 0, islands.shape[0] - 1))
    c = int(np.clip(round(islands.shape[1] - 1 - cx), 0, islands.shape[1] - 1))
    lab = int(islands[r, c])
    if lab == 0:
        _, idx = ndimage.distance_transform_edt(islands == 0, return_indices=True)
        lab = int(islands[idx[0][r, c], idx[1][r, c]])
    if lab == iid:
        raise ValueError(f"island {iid} is centred on the midline; cannot mirror it")
    return lab


# ---------------------------------------------------------------- spec

def validate_spec(spec):
    pats = spec.get("patches")
    if not pats:
        raise ValueError("spec needs a non-empty 'patches' list")
    names = [p.get("name") for p in pats]
    if len(set(names)) != len(names) or None in names:
        raise ValueError("patch names must be present and unique")
    for p in pats:
        if not isinstance(p.get("island"), int):
            raise ValueError(f"{p['name']}: 'island' must be an integer template island id")
        if len(p.get("loop", [])) < 3:
            raise ValueError(f"{p['name']}: loop needs at least 3 control points")
        if "seed" not in p:
            raise ValueError(f"{p['name']}: 'seed' is required")
        for pt in p["loop"] + p.get("interior", []) + [p["seed"]]:
            if "uv" not in pt and "xyz" not in pt:
                raise ValueError(f"{p['name']}: point needs 'uv' or 'xyz': {pt}")
        if "t" not in p["loop"][0]:
            raise ValueError(f"{p['name']}: the first loop point needs 't'")
        for pt in p["loop"]:
            if pt.get("edge", "line") not in ("line", "contour"):
                raise ValueError(f"{p['name']}: edge must be 'line' or 'contour'")
    if spec.get("fallback", "nearest") not in ("nearest", "none"):
        raise ValueError("fallback must be 'nearest' or 'none'")


def expand_patches(spec, islands):
    """Source patches, each followed by its mirror image when 'mirror' is set."""
    out = []
    for p in spec["patches"]:
        out.append(dict(p, mirror=False))
        if p.get("mirror"):
            out.append(dict(p, name=p["name"] + "_mirror", mirror=True,
                            island=p.get("mirror_island") or mirror_island(islands, p["island"])))
    return out


def resolve_point(mesh, pt, mirror, width):
    """Texel node and template coords (None if the point has no t) for one spec point."""
    if "uv" in pt:
        node = mesh.node_from_uv(pt["uv"])
        p3 = mesh.P[node]
    else:
        p3 = np.asarray(pt["xyz"], float)
        node = mesh.node_from_xyz(p3)
    t = np.asarray(pt["t"], float) if "t" in pt else None
    if mirror:
        node = mesh.node_from_xyz(p3 * [1.0, 1.0, -1.0])
        if t is not None:
            t = np.array([width - 1 - t[0], t[1]])
    return node, t


# ---------------------------------------------------------------- per-patch solve

def edge_samples(path, seg, ta, tb, kind, isl):
    s = np.concatenate([[0.0], np.cumsum(seg)])
    f = s / s[-1] if s[-1] > 0 else np.linspace(0, 1, len(path))
    if kind == "line":
        return ta + f[:, None] * (tb - ta)
    return isl.arc(ta, tb, f)


def harmonic(mesh, pn, dir_nodes, dir_vals):
    """Graph Laplace solve for template (u, v) on nodes pn with Dirichlet values."""
    loc = np.full(mesh.N, -1, np.int64)
    loc[pn] = np.arange(pn.size)
    W = mesh.G_harm[pn][:, pn]
    L = (sparse.diags(np.asarray(W.sum(axis=1)).ravel()) - W).tocsr()
    fixed = np.zeros(pn.size, bool)
    fixed[loc[dir_nodes]] = True
    vals = np.zeros((pn.size, 2))
    vals[loc[dir_nodes]] = dir_vals
    U, D = np.flatnonzero(~fixed), np.flatnonzero(fixed)
    out = vals.copy()
    if U.size:
        A = L[U][:, U].tocsr()
        rhs = -(L[U][:, D] @ vals[D])
        M = sparse.diags(1.0 / A.diagonal())
        for k in range(2):
            x, info = cg(A, rhs[:, k], rtol=CG_RTOL, maxiter=CG_MAXITER, M=M)
            if info != 0:
                raise RuntimeError(f"CG did not converge (info={info}, unknowns={U.size})")
            out[U, k] = x
    return out


def solve_patch(mesh, pat, isl, width):
    name, mirror = pat["name"], pat["mirror"]
    ctrl = []
    for pt in pat["loop"]:
        node, t = resolve_point(mesh, pt, mirror, width)
        ctrl.append((node, t, pt.get("edge", "line")))

    # Points without t are via points: t is interpolated by 3D arc length over the run
    # from the previous anchor to the next one, using the previous anchor's edge rule.
    n = len(ctrl)
    anchors = [k for k in range(n) if ctrl[k][1] is not None]
    nodes, tvals, loop_len = [], [], 0.0
    for j, k0 in enumerate(anchors):
        k1 = anchors[(j + 1) % len(anchors)]
        run = [k0]
        k = k0
        while True:
            k = (k + 1) % n
            run.append(k)
            if k == k1:
                break
        path = [np.array([ctrl[run[0]][0]])]
        for a_k, b_k in zip(run[:-1], run[1:]):
            a, b = ctrl[a_k][0], ctrl[b_k][0]
            if a != b:
                path.append(mesh_path(mesh, a, b, name)[1:])
        path = np.concatenate(path)
        if path.size < 2:
            nodes.append(path)
            tvals.append(ctrl[k0][1][None])
            continue
        seg = np.linalg.norm(np.diff(mesh.P[path], axis=0), axis=1)
        loop_len += float(seg.sum())
        t = edge_samples(path, seg, ctrl[k0][1], ctrl[k1][1], ctrl[k0][2], isl)
        nodes.append(path[:-1])          # each run owns its start texel
        tvals.append(t[:-1])
    loop, first = np.unique(np.concatenate(nodes), return_index=True)
    loop_t = np.concatenate(tvals)[first]

    seed, _ = resolve_point(mesh, pat["seed"], mirror, width)
    fill, loop_all, thick = fill_patch(mesh, loop, seed, name)
    if thick:
        extra = np.setdiff1d(loop_all, loop)
        loop_t = np.concatenate([loop_t, nearest_values(mesh, loop, loop_t, extra)])
        loop_all = np.concatenate([loop, extra])

    cons_nodes, cons_t, skipped = [], [], 0
    for pt in pat.get("interior", []):
        node, t = resolve_point(mesh, pt, mirror, width)
        if t is None:
            raise ValueError(f"{name}: interior points need 't'")
        if np.isin(node, loop_all):
            skipped += 1
            continue
        if not np.isin(node, fill):
            raise ValueError(f"{name}: interior point {pt} lies outside the patch")
        cons_nodes.append(node)
        cons_t.append(t)
    dir_nodes = np.concatenate([loop_all, np.array(cons_nodes, np.int64)])
    dir_vals = np.concatenate([loop_t, np.array(cons_t, float).reshape(-1, 2)])
    dir_nodes, first = np.unique(dir_nodes, return_index=True)
    dir_vals = dir_vals[first]

    t0 = time.time()
    pn = np.union1d(fill, dir_nodes)
    xy = harmonic(mesh, pn, dir_nodes, dir_vals)
    return dict(pn=pn, xy=xy, loop=loop_all, loop_len=loop_len, thick=thick,
                skipped=skipped, solve_s=time.time() - t0)


def anisotropy(mesh, pn, xy):
    """Median and p90 of s1/s2 of the template-per-surface Jacobian over patch texels."""
    grid = np.full(mesh.shape, -1, np.int64)
    grid[mesh.pix[pn, 0], mesh.pix[pn, 1]] = np.arange(pn.size)
    R, C = mesh.shape
    r, c = mesh.pix[pn, 0], mesh.pix[pn, 1]
    right = grid[r, np.minimum(c + 1, C - 1)]
    down = grid[np.minimum(r + 1, R - 1), c]
    i = np.flatnonzero((right >= 0) & (down >= 0) & (c + 1 < C) & (r + 1 < R))
    if i.size == 0:
        return float("nan"), float("nan")
    j, k = right[i], down[i]
    T = np.stack([xy[j] - xy[i], xy[k] - xy[i]], -1)
    S = np.stack([mesh.P[pn[j]] - mesh.P[pn[i]], mesh.P[pn[k]] - mesh.P[pn[i]]], -1)
    lim = 3.0 * mesh.med
    good = (np.linalg.norm(S[..., 0], axis=1) < lim) & (np.linalg.norm(S[..., 1], axis=1) < lim)
    good &= np.isfinite(T).all((1, 2))
    _, Rr = np.linalg.qr(S[good])
    ok = np.abs(np.linalg.det(Rr)) > 1e-12
    Jm = T[good][ok] @ np.linalg.inv(Rr[ok])
    if Jm.size == 0:
        return float("nan"), float("nan")
    sv = np.linalg.svd(Jm, compute_uv=False)
    aniso = sv[:, 0] / np.maximum(sv[:, 1], 1e-12)
    return float(np.median(aniso)), float(np.percentile(aniso, 90))


def build_main_body(mesh, islands, spec, log=print):
    """Solve every patch in order (later patches win overlaps). Returns coords, owner, loops, stats."""
    width = islands.shape[1]
    patches = expand_patches(spec, islands)
    owner = np.full(mesh.N, -1, np.int64)
    coords = np.full((mesh.N, 2), np.nan)
    loopflag = np.zeros(mesh.N, bool)
    shapes, stats = {}, []
    for idx, pat in enumerate(patches):
        if pat["island"] not in shapes:
            shapes[pat["island"]] = IslandShape(islands, pat["island"])
        isl = shapes[pat["island"]]
        res = solve_patch(mesh, pat, isl, width)
        pn = res["pn"]
        xy, clamped = isl.clamp(res["xy"])
        overlap = int((owner[pn] >= 0).sum())
        owner[pn] = idx
        coords[pn] = xy
        loopflag[res["loop"]] = True
        a50, a90 = anisotropy(mesh, pn, xy)
        st = dict(index=idx, name=pat["name"], island=pat["island"], mirror=pat["mirror"],
                  texels=int(pn.size), paint_texels=int(mesh.paint[pn].sum()),
                  loop_texels=int(res["loop"].size), loop_len=res["loop_len"],
                  thickened=res["thick"], interior_skipped=res["skipped"],
                  solve_s=res["solve_s"], clamped_pct=clamped, overlap_texels=overlap,
                  aniso_p50=a50, aniso_p90=a90)
        stats.append(st)
        log(f"[{idx}] {pat['name']:<22s} island={pat['island']:<3d} texels={st['texels']:7d} "
            f"(paint {st['paint_texels']}) loop={st['loop_texels']} texels, "
            f"len={st['loop_len']:.3f}  solve={st['solve_s']:.2f}s  clamped={clamped:.2f}%  "
            f"overlap={overlap}  aniso p50={a50:.3f} p90={a90:.3f}"
            + ("  [loop thickened]" if res["thick"] else ""))
    return coords, owner, loopflag, stats, patches


def fill_unassigned(mesh, coords, owner, fallback):
    """Paint texels without a patch take the coords of the nearest assigned paint texel."""
    if fallback == "none":
        return 0
    todo = np.flatnonzero((owner < 0) & mesh.paint)
    src = np.flatnonzero((owner >= 0) & mesh.paint)
    if todo.size == 0 or src.size == 0:
        return 0
    _, _, sources = csgraph.dijkstra(mesh.G_len, indices=src, min_only=True, return_predecessors=True)
    s = sources[todo]
    ok = s >= 0
    coords[todo[ok]] = coords[s[ok]]
    owner[todo[ok]] = owner[s[ok]]
    smooth_filled(mesh, coords, owner, todo[ok])
    return int(ok.sum())


def smooth_filled(mesh, coords, owner, filled):
    """Replace nearest-copied coords by a harmonic fill over UV-grid neighbours.

    Texels cut off the 3D texel graph (grazing texels of FP04RB's planar projections)
    otherwise show blocky copies. Neighbours are UV 4-neighbours with the same owner patch,
    so the fill never blends two template islands; solved texels stay fixed.
    """
    if filled.size == 0:
        return
    R, C = mesh.shape
    grid = np.full((R, C), -1, np.int64)
    grid[mesh.pix[:, 0], mesh.pix[:, 1]] = np.arange(mesh.N)
    a, b = _grid_pairs(grid >= 0, grid)
    same = (owner[a] == owner[b]) & (owner[a] >= 0)
    a, b = a[same], b[same]
    W = sparse.csr_matrix((np.ones(2 * a.size), (np.r_[a, b], np.r_[b, a])), shape=(mesh.N, mesh.N))
    free = np.zeros(mesh.N, bool)
    free[filled] = True
    deg = np.asarray(W.sum(axis=1)).ravel()
    U = np.flatnonzero(free & (deg > 0))
    if U.size == 0:
        return
    L = (sparse.diags(deg) - W).tocsr()
    D = np.flatnonzero(~free)
    A = L[U][:, U].tocsr()
    rhs = -(L[U][:, D] @ coords[D])
    # Components of free texels with no fixed neighbour keep their nearest copy.
    _, comp = csgraph.connected_components(A, directed=False)
    anchored = np.zeros(comp.max() + 1, bool)
    anchored[comp[np.flatnonzero(np.abs(rhs).sum(1) > 0)]] = True
    M = sparse.diags(1.0 / A.diagonal())
    for k in range(2):
        x, _ = cg(A, rhs[:, k], x0=coords[U, k], rtol=1e-6, maxiter=4000, M=M)
        keep = anchored[comp]
        coords[U[keep], k] = x[keep]


def upsample(xy, region, stride):
    """Bilinear upsample of a coarse coordinate grid to the full grid; NaN where the coarse sample is empty."""
    R, C = xy.shape[:2]
    ok = np.isfinite(xy[..., 0])
    _, idx = ndimage.distance_transform_edt(~ok, return_indices=True)
    filled = xy[idx[0], idx[1]]
    yy, xx = np.mgrid[0:R * stride, 0:C * stride].astype(float) / stride
    out = np.stack([ndimage.map_coordinates(filled[..., k], [yy, xx], order=1, mode="nearest")
                    for k in (0, 1)], -1)
    ny = np.clip(np.rint(yy).astype(int), 0, R - 1)
    nx = np.clip(np.rint(xx).astype(int), 0, C - 1)
    valid = ok[ny, nx]
    out[~valid] = np.nan
    return out.astype(np.float32), np.where(valid, region[ny, nx], -1).astype(np.int8)


def output_grids(mesh, coords, owner):
    """main_body xy and region on the solve grid; region -1 where the texel has no coords."""
    ok = (owner >= 0) & mesh.paint
    xy = np.full(mesh.shape + (2,), np.nan, np.float32)
    region = np.full(mesh.shape, -1, np.int8)
    xy[mesh.pix[ok, 0], mesh.pix[ok, 1]] = coords[ok]
    region[mesh.pix[ok, 0], mesh.pix[ok, 1]] = owner[ok]
    return xy, region


def write_map(base_path, out, xy, region, names=None):
    """Copy every key of base_path, replacing the main_body arrays."""
    base = np.load(base_path)
    arrays = {k: base[k] for k in base.files}
    if arrays["main_body_xy"].shape[:2] != xy.shape[:2]:
        raise ValueError("base map shape does not match the new main_body map")
    shape = xy.shape[:2]
    occupied = np.isfinite(arrays["main_body_xy"][...,0]) | (arrays["main_body_fill"] > 0)
    arrays["main_body_xy"] = xy.astype(np.float32)
    arrays["main_body_region"] = region.astype(np.int8)
    arrays["main_body_fill"] = np.where(np.isfinite(xy[..., 0]), 0,
        np.where(occupied, np.maximum(arrays["main_body_fill"], 2), 0)).astype(np.int8)
    if names is not None:
        arrays["main_body_names"] = np.array(names)
    arrays["main_body_snap"] = np.zeros(shape, np.float16)
    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    np.savez_compressed(out, **arrays)


# ---------------------------------------------------------------- debug images

def _palette(i):
    r, g, b = colorsys.hsv_to_rgb((i * 0.618034) % 1.0, 0.85, 1.0)
    return np.array([r, g, b]) * 255


def debug_images(mesh, coords, owner, loopflag, islands, stem):
    stride = mesh.stride
    half = lambda a: np.repeat(np.repeat(a, stride, 0), stride, 1)[::2, ::2]   # to 1024 UV
    R, C = mesh.shape
    lab = np.full(mesh.shape, -2, np.int64)
    lab[mesh.pix[:, 0], mesh.pix[:, 1]] = np.where(mesh.paint, owner, -2)
    loops = np.zeros(mesh.shape, bool)
    loops[mesh.pix[loopflag, 0], mesh.pix[loopflag, 1]] = True
    uv = np.zeros(mesh.shape + (3,), np.uint8)
    n_patch = int(owner.max()) + 1 if owner.size else 0
    palette = np.array([_palette(i) for i in range(max(n_patch, 1))], np.uint8)
    paint_grid = lab >= 0
    uv[paint_grid] = palette[lab[paint_grid]]
    uv[lab == -1] = (110, 110, 110)
    uv[loops] = (255, 255, 255)
    uv_img = half(uv)
    Image.fromarray(uv_img).save(f"{stem}_uv_patches.png")

    cov = np.zeros((1024, 1024, 3), np.uint8)
    cov[islands[::2, ::2] > 0] = (60, 60, 60)
    ok = (owner >= 0) & mesh.paint
    xy = coords[ok]
    rr = np.clip(np.rint(xy[:, 1] / 2).astype(int), 0, 1023)
    cc = np.clip(np.rint(xy[:, 0] / 2).astype(int), 0, 1023)
    cov[rr, cc] = palette[owner[ok]]
    Image.fromarray(cov).save(f"{stem}_template_coverage.png")


# ---------------------------------------------------------------- CLI

def main(argv=None):
    ap = argparse.ArgumentParser(description="Build main_body map from harmonic patches.")
    ap.add_argument("--spec", type=Path, required=True, help="patch spec JSON")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT, help="output map npz")
    ap.add_argument("--bake", type=Path, default=DEFAULT_BAKE, help="bake2k main_body.npz")
    ap.add_argument("--islands", type=Path, default=DEFAULT_ISLANDS, help="islands.npy")
    ap.add_argument("--base", type=Path, default=DEFAULT_BASE, help="map2026.npz: other parts copied from it")
    ap.add_argument("--res", type=int, default=2048, choices=(1024, 2048),
                    help="solve grid; 1024 subsamples texels 2x and bilinearly upsamples the result")
    args = ap.parse_args(argv)

    t_all = time.time()
    spec = json.loads(args.spec.read_text())
    validate_spec(spec)
    bake = np.load(args.bake)
    pos, mask, mat = bake["pos"], bake["mask"], bake["mat"]
    islands = np.load(args.islands)
    stride = pos.shape[0] // args.res
    if stride > 1:
        pos, mask, mat = pos[::stride, ::stride], mask[::stride, ::stride], mat[::stride, ::stride]
    t0 = time.time()
    mesh = Mesh(pos, mask, mask & (mat == 0), stride)
    print(f"texel graph: {mesh.N} nodes, median spacing {mesh.med:.5f}, cut edges {mesh.n_cut}, "
          f"seam links {mesh.n_seam}, stride {stride}, built in {time.time() - t0:.1f}s", flush=True)

    coords, owner, loopflag, stats, _ = build_main_body(mesh, islands, spec)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    stem = str(args.out.with_suffix(""))
    debug_images(mesh, coords, owner, loopflag, islands, stem)   # solved texels only, before fallback
    n_fb = fill_unassigned(mesh, coords, owner, spec.get("fallback", "nearest"))
    print(f"fallback ({spec.get('fallback', 'nearest')}): {n_fb} paint texels filled from nearest assigned texel")

    xy, region = output_grids(mesh, coords, owner)
    if stride > 1:
        xy, region = upsample(xy, region, stride)
    write_map(args.base, args.out, xy, region, [p["name"] for p in expand_patches(spec, islands)])
    sidecar = args.out.with_suffix(".patches.json")
    sidecar.write_text(json.dumps(stats, indent=1))
    covered = np.isfinite(xy[..., 0]).sum()
    print(f"wrote {args.out} (main_body coords on {covered} texels), debug {stem}_*.png, {sidecar}")
    print(f"total {time.time() - t_all:.1f}s")


if __name__ == "__main__":
    main()
