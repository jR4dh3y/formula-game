"""Refine selected patches on welded OBJ triangles, then bake onto original UVs.

The engine cover's planar UVs undersample its spine. A triangle solve avoids
inventing seam links there. Cockpit loops still require the texel chart because
FP04RB's nose/cockpit loop is not closed on the welded vertex graph.
"""
import argparse
import json
from pathlib import Path
import sys

import numpy as np
from scipy import ndimage, sparse
from scipy.sparse import csgraph
from scipy.spatial import cKDTree

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bake_fp04rb as bake
import build_map_v3 as bm


class TriangleMesh:
    """Same patch solver interface, with true mesh edges and cotangent weights."""
    def __init__(self, vertices, triangles, materials, baked):
        self.P, inverse = np.unique(np.round(vertices, 6), axis=0, return_inverse=True)
        self.triangles = inverse[triangles]
        t = self.triangles
        self.N = len(self.P)
        self.paint = np.zeros(self.N, bool)
        self.paint[t[materials == 0]] = True
        edges = np.unique(np.sort(np.concatenate([t[:, [0,1]], t[:, [1,2]], t[:, [2,0]]]), axis=1), axis=0)
        length = np.linalg.norm(self.P[edges[:,0]]-self.P[edges[:,1]], axis=1)
        keep = length > 1e-9
        edges, length = edges[keep], length[keep]
        row, col = np.r_[edges[:,0],edges[:,1]], np.r_[edges[:,1],edges[:,0]]
        self.G_len = sparse.csr_matrix((np.r_[length,length],(row,col)), shape=(self.N,self.N))
        self.A = self.G_len.copy()
        self.A.data[:] = 1
        self.G_harm = bm.cotangent_weights(self.P, t)
        _, comp = csgraph.connected_components(self.A, directed=False)
        self.main_nodes = np.flatnonzero(comp == np.bincount(comp).argmax())
        self.kd = cKDTree(self.P[self.main_nodes])
        self.baked = baked
        _, self.nearest = ndimage.distance_transform_edt(~baked['mask'], return_indices=True)

    def node_from_xyz(self, point):
        return int(self.main_nodes[self.kd.query(point)[1]])

    def node_from_uv(self, uv):
        c, r = np.rint(uv).astype(int)
        r, c = self.nearest[:, r, c]
        return self.node_from_xyz(self.baked['pos'][r, c])


def refine(obj, baked, spec, islands, mapping):
    vertices, uv, triangles, triangle_uv, materials, _ = bake.load(obj)
    mesh = TriangleMesh(vertices, triangles, materials, baked)
    patches = bm.expand_patches(spec, islands)
    owner = np.full(mesh.N, -1, int)
    coords = np.full((mesh.N, 2), np.nan)
    stats = []
    for i, patch in enumerate(patches):
        if patch.get('solver') != 'triangles':
            continue
        shape = bm.IslandShape(islands, patch['island'])
        solved = bm.solve_patch(mesh, patch, shape, islands.shape[1])
        xy, clamped = shape.clamp(solved['xy'])
        owner[solved['pn']] = i
        coords[solved['pn']] = xy
        stats.append({'name':patch['name'], 'vertices':len(solved['pn']), 'clamped_pct':clamped})
        print(stats[-1], flush=True)
    out = mapping['main_body_xy'].copy()
    region = mapping['main_body_region'].copy()
    size = out.shape[0]
    pixels = uv[triangle_uv] * [size, -size] + [0, size]
    # Faces across patch boundaries retain the texel solve. Interpolating their
    # different template islands would produce spurious stripes.
    for face, x, y, weights in bake.raster_samples(pixels, size):
        ids = mesh.triangles[face]
        labels = owner[ids]
        if materials[face] != 0 or labels[0] < 0 or not (labels == labels[0]).all():
            continue
        # A boundary face must not steal the neighbouring patch's interior.
        take = region[y,x] == labels[0]
        out[y[take],x[take]] = (weights @ coords[ids])[take]
    mapping['main_body_xy'] = out
    return mapping, stats


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--obj',type=Path,required=True)
    ap.add_argument('--bake',type=Path,required=True)
    ap.add_argument('--spec',type=Path,required=True)
    ap.add_argument('--islands',type=Path,required=True)
    ap.add_argument('--map',type=Path,required=True)
    a = ap.parse_args()
    result, stats = refine(a.obj, np.load(a.bake), json.loads(a.spec.read_text()),
                           np.load(a.islands), dict(np.load(a.map)))
    np.savez_compressed(a.map, **result)
    a.map.with_suffix('.triangles.json').write_text(json.dumps(stats,indent=2)+'\n')

if __name__ == '__main__':
    main()
