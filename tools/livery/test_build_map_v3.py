"""Synthetic tests for build_map_v3.py.

Run from the repo root:
  python3 -I -m unittest discover -s tools/livery -p 'test_*.py'
"""
import tempfile
import unittest
from pathlib import Path

import numpy as np

import build_map_v3 as bm

R = C = 40          # flat texel grid: row = X, col -> Z = col - 20
W_T = 128           # template width; islands 1 (u < 64) and 2 (u >= 64) mirror each other
TOL = 1e-3
QUIET = lambda *a, **k: None


def plane():
    r, c = np.mgrid[0:R, 0:C]
    pos = np.zeros((R, C, 3), np.float32)
    pos[..., 0] = r
    pos[..., 2] = c - C / 2
    mask = np.ones((R, C), bool)
    return pos, mask, mask.copy()


def halves_islands():
    isl = np.zeros((W_T, W_T), np.int64)
    isl[:, :64] = 1
    isl[:, 64:] = 2
    return isl


def square_loop(r0, r1, c0, c1, t_of):
    """8 control points (corners + edge midpoints) so every edge is a straight, unique geodesic."""
    rm, cm = (r0 + r1) // 2, (c0 + c1) // 2
    cells = [(r0, c0), (r0, cm), (r0, c1), (rm, c1), (r1, c1), (r1, cm), (r1, c0), (rm, c0)]
    return [{"uv": [c, r], "t": list(t_of(r, c))} for r, c in cells]


def identity_t(r, c):
    return (4 + c, 4 + r)


def spec_with(patches):
    return {"patches": patches, "fallback": "nearest"}


class BuildMapV3Test(unittest.TestCase):
    def setUp(self):
        self.pos, self.mask, self.paint = plane()
        self.isl = halves_islands()
        self.mesh = bm.Mesh(self.pos, self.mask, self.paint, 1)

    def node(self, r, c):
        return int(self.mesh.node_of[r, c])

    def identity_patch(self, **extra):
        return dict(name="sq", island=1, mirror=False, loop=square_loop(10, 30, 5, 25, identity_t),
                    seed={"uv": [15, 20]}, **extra)

    def solve(self, pat, isl_id=1):
        isl = bm.IslandShape(self.isl, isl_id)
        return bm.solve_patch(self.mesh, dict(pat, mirror=pat.get("mirror", False)), isl, W_T)

    # --- graph ---

    def test_flat_plane_has_no_seam_links_or_cuts(self):
        self.assertEqual(self.mesh.n_seam, 0)
        self.assertEqual(self.mesh.n_cut, 0)

    # --- harmonic map ---

    def test_affine_boundary_gives_affine_interior(self):
        res = self.solve(self.identity_patch())
        r, c = self.mesh.pix[res["pn"], 0], self.mesh.pix[res["pn"], 1]
        expected = np.stack([4 + c, 4 + r], 1)
        self.assertLess(np.abs(res["xy"] - expected).max(), TOL)
        self.assertEqual(res["pn"].size, 21 * 21)   # 19x19 interior + 80 loop texels
        self.assertFalse(res["thick"])

    def test_anisotropy_of_identity_is_one(self):
        res = self.solve(self.identity_patch())
        a50, _ = bm.anisotropy(self.mesh, res["pn"], res["xy"])
        self.assertAlmostEqual(a50, 1.0, places=3)

    def test_interior_constraint_is_honoured(self):
        pat = self.identity_patch(interior=[{"uv": [15, 20], "t": [50, 50]}])
        res = self.solve(pat)
        i = np.flatnonzero(res["pn"] == self.node(20, 15))[0]
        np.testing.assert_allclose(res["xy"][i], [50, 50], atol=TOL)

    def test_open_loop_is_rejected(self):
        loop = [{"uv": [5, 10], "t": [9, 14]}, {"uv": [25, 10], "t": [29, 14]},
                {"uv": [15, 10], "t": [19, 14]}]
        pat = dict(name="open", island=1, mirror=False, loop=loop, seed={"uv": [15, 20]})
        with self.assertRaisesRegex(ValueError, "does not close"):
            self.solve(pat)

    # --- contour edges ---

    def test_contour_edge_follows_island_boundary_by_arc_length(self):
        isl3 = np.zeros((W_T, W_T), np.int64)
        isl3[20:101, 20:61] = 3
        A, B, Cc, D = (20, 30), (60, 30), (60, 90), (20, 90)
        corners = [(10, 5, A), (10, 25, B), (30, 25, Cc), (30, 5, D)]
        loop = [{"uv": [c, r], "t": list(t), "edge": "contour"} for r, c, t in corners]
        pat = dict(name="block", island=3, mirror=False, loop=loop, seed={"uv": [15, 20]})
        isl = bm.IslandShape(isl3, 3)
        res = bm.solve_patch(self.mesh, pat, isl, W_T)
        # Midpoint of the top edge (3D arc half-way) lands at arc 30 of the 60-px upper arc: (40, 20).
        i = np.flatnonzero(res["pn"] == self.node(10, 15))[0]
        np.testing.assert_allclose(res["xy"][i], [40, 20], atol=1.5)
        # Every loop texel of a contour edge lies on the block boundary.
        lp = res["xy"][np.isin(res["pn"], res["loop"])]
        on = ((np.abs(lp[:, 0] - 20) < 1) | (np.abs(lp[:, 0] - 60) < 1)
              | (np.abs(lp[:, 1] - 20) < 1) | (np.abs(lp[:, 1] - 100) < 1))
        self.assertTrue(on.all())

    def test_cotangent_solve_reproduces_affine_coordinates_on_uneven_mesh(self):
        from types import SimpleNamespace
        y, x = np.mgrid[:12, :12].astype(float)
        rng = np.random.default_rng(12)
        x[1:-1,1:-1] += rng.uniform(-.2,.2,(10,10))
        y[1:-1,1:-1] += rng.uniform(-.2,.2,(10,10))
        p = np.stack([x.ravel(),y.ravel(),np.zeros(x.size)],1)
        nodes = np.arange(x.size).reshape(x.shape)
        a,b,c,d = nodes[:-1,:-1].ravel(),nodes[:-1,1:].ravel(),nodes[1:,:-1].ravel(),nodes[1:,1:].ravel()
        triangles = np.concatenate([np.stack([a,b,d],1),np.stack([a,d,c],1)])
        mesh = SimpleNamespace(N=len(p),G_harm=bm.cotangent_weights(p,triangles))
        boundary = np.unique(np.r_[nodes[0],nodes[-1],nodes[:,0],nodes[:,-1]])
        xy = bm.harmonic(mesh,np.arange(len(p)),boundary,p[boundary,:2])
        np.testing.assert_allclose(xy,p[:,:2],atol=1e-6)

    # --- mirror ---

    def test_mirror_patch_is_mirrored_in_z_and_u(self):
        spec = spec_with([dict(name="R", island=1, mirror=True,
                               loop=square_loop(10, 30, 25, 35, identity_t),
                               seed={"uv": [30, 20]})])
        coords, owner, _, stats, patches = bm.build_main_body(self.mesh, self.isl, spec, QUIET)
        self.assertEqual([p["name"] for p in patches], ["R", "R_mirror"])
        self.assertEqual(stats[1]["island"], 2)
        self.assertEqual(stats[1]["overlap_texels"], 0)
        mir = np.flatnonzero(owner == 1)
        r, c = self.mesh.pix[mir, 0], self.mesh.pix[mir, 1]
        self.assertTrue(((c >= 5) & (c <= 15)).all())
        # Original texel (r, 40 - c) has u = 4 + (40 - c'); its mirror has u = 127 - that.
        expected = np.stack([(W_T - 1) - (4 + (C - c)), 4 + r], 1)
        self.assertLess(np.abs(coords[mir] - expected).max(), TOL)

    # --- fallback and output ---

    def test_unassigned_paint_is_filled_smoothly_from_assigned_coords(self):
        spec = spec_with([self.identity_patch()])
        coords, owner, _, _, _ = bm.build_main_body(self.mesh, self.isl, spec, QUIET)
        solved = coords[owner >= 0].copy()
        n = bm.fill_unassigned(self.mesh, coords, owner, "nearest")
        self.assertGreater(n, 0)
        self.assertEqual(owner[self.node(0, 20)], 0)
        # Harmonic fill: inside the range of the solved coords, and no texel equals its
        # neighbour (a blocky nearest copy) along the filled row.
        filled = coords[self.node(0, 20)]
        self.assertTrue(((filled >= solved.min(0) - TOL) & (filled <= solved.max(0) + TOL)).all())
        row = np.array([coords[self.node(0, c)] for c in range(10, 31)])
        self.assertGreater(np.abs(np.diff(row[:, 0])).min(), 1e-6)

    def test_write_map_keeps_base_keys_and_copies_other_parts(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        base = root / "base.npz"
        front = np.random.default_rng(0).normal(size=(R, C, 2)).astype(np.float32)
        np.savez(base, names=np.array(["a", "b"]),
                 main_body_xy=np.zeros((R, C, 2), np.float32),
                 main_body_region=np.zeros((R, C), np.int8),
                 main_body_fill=np.zeros((R, C), np.int8),
                 main_body_snap=np.zeros((R, C), np.float16),
                 front_wing_xy=front)
        xy = np.full((R, C, 2), 7.0, np.float32)
        region = np.zeros((R, C), np.int8)
        bm.write_map(base, root / "out.npz", xy, region)
        old, new = np.load(base), np.load(root / "out.npz")
        self.assertEqual(sorted(old.files), sorted(new.files))
        for k in old.files:
            self.assertEqual(old[k].dtype, new[k].dtype, k)
            self.assertEqual(old[k].shape, new[k].shape, k)
        np.testing.assert_array_equal(new["front_wing_xy"], front)
        np.testing.assert_array_equal(new["main_body_xy"], xy)

    def test_upsample_recovers_affine_field(self):
        rr, cc = np.mgrid[0:10, 0:10]
        coarse = np.stack([2 * cc + 3, 2 * rr + 5], -1).astype(np.float32)
        xy, region = bm.upsample(coarse, np.zeros((10, 10), np.int8), 2)
        self.assertEqual(xy.shape, (20, 20, 2))
        fr, fc = np.mgrid[2:18, 2:18]
        np.testing.assert_allclose(xy[2:18, 2:18, 0], fc + 3, atol=1e-3)
        np.testing.assert_allclose(xy[2:18, 2:18, 1], fr + 5, atol=1e-3)


if __name__ == "__main__":
    unittest.main()
