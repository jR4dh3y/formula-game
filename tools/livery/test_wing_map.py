"""Wing correspondence tests. Run: python3 -I -m unittest discover -s tools/livery -p 'test_*.py'"""
from pathlib import Path
import sys
import unittest

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import wing_map

WORK = Path(__file__).resolve().parents[2] / "raw/gen22/livery-mapping"


def curved_island(h, w, top, thick):
    """Island whose top edge dips in the middle, like the template strips."""
    yy, xx = np.mgrid[0:h, 0:w]
    lo = top + 10 * np.sin(np.linspace(0, np.pi, w))
    return (yy >= lo[None]) & (yy <= lo[None] + thick)


class Strips(unittest.TestCase):
    def test_stacked_islands_never_sample_the_gap(self):
        a = curved_island(120, 200, 10, 20)
        b = curved_island(120, 200, 50, 30)
        rng = np.random.default_rng(0)
        xn, t = rng.uniform(-1, 1, 20000), rng.uniform(0, 1, 20000)
        k, x, y = wing_map.strips([a, b], xn, t)
        r, c = np.round(y).astype(int), np.round(x).astype(int)
        inside = np.where(k == 0, a[r, c], b[r, c])
        self.assertTrue(inside.all(), "samples outside their island (gap or border)")
        # Each island takes its height share; t is monotone across the stack.
        order = np.argsort(t)
        self.assertTrue((np.diff(k[order]) >= 0).sum() > 0.99 * len(t))

    def test_span_interpolates_continuously(self):
        a = curved_island(120, 200, 10, 20)
        xn = np.linspace(-1, 1, 5001)
        _, x, y = wing_map.strips([a], xn, np.full_like(xn, 0.5))
        self.assertLess(np.abs(np.diff(x)).max(), 0.1)
        self.assertLess(np.abs(np.diff(y)).max(), 0.1)  # no per-column steps


class Chords(unittest.TestCase):
    def test_steep_face_keeps_true_arc_length(self):
        # A near-vertical rear face: planar X barely changes along the chord.
        sel = np.zeros((4, 64), bool); sel[:, 2:62] = True
        pos = np.zeros((4, 64, 3))
        s = np.linspace(0, 1, 64)
        pos[..., 0] = -6.2 + 0.02 * s          # X: LE (high) at the right
        pos[..., 1] = 2.0 + 0.4 * s ** 2       # steep, curved
        a, tot = wing_map.row_arc(sel, pos, start_high_x=False)
        u = a / tot
        self.assertAlmostEqual(np.nanmin(u), 0.0)
        self.assertAlmostEqual(np.nanmax(u), 1.0)
        # Arc fraction, not X fraction: the steep end is not compressed.
        x_frac = (pos[0, 2:62, 0] - pos[0, 2, 0]) / np.ptp(pos[0, 2:62, 0])
        self.assertGreater(np.abs(u[0, 2:62] - x_frac).max(), 0.1)

    def test_front_slots_separate_artwork_even_with_reversed_uv_columns(self):
        sel = np.ones((2, 120), bool)
        pos = np.zeros((2, 120, 3))
        pos[0, :, 0] = np.linspace(7, 6, 120)
        pos[0, :, 1] = np.linspace(0, .1, 120) + np.repeat([0, .05, .13], 40)
        pos[1] = pos[0, ::-1]
        elements = wing_map.front_elements(sel, pos)
        for i, element in enumerate(elements):
            expected = np.zeros(120, bool)
            expected[i * 40:(i + 1) * 40] = True
            np.testing.assert_array_equal(element[0], expected)
            np.testing.assert_array_equal(element[1], expected[::-1])

    def test_converged_front_flaps_stay_unpainted(self):
        sel = np.ones((1, 120), bool)
        pos = np.zeros((1, 120, 3))
        pos[0, :, 0] = np.linspace(7, 6, 120)
        self.assertFalse(np.any(wing_map.front_elements(sel, pos)))

    def test_flap_split_at_slot_step(self):
        sel = np.ones((1, 100), bool)
        pos = np.zeros((1, 100, 3))
        pos[0, :, 0] = np.linspace(-6.25, -5.45, 100)   # columns run with X
        pos[0, :, 1] = np.where(pos[0, :, 0] < -6.06, 2.3, 2.1)
        flap = wing_map.split_flap(sel, pos)
        np.testing.assert_array_equal(flap[0], pos[0, :, 0] < -6.06)


@unittest.skipUnless((WORK / "map_v3.npz").is_file() and (WORK / "bake2k/rear_wing.npz").is_file(),
                     "generated livery map not present")
class InstalledMap(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.M = np.load(WORK / "map_v3.npz")
        cls.names = [str(n) for n in cls.M["names"]]

    def region(self, part, name):
        return self.M[f"{part}_region"] == self.names.index(name)

    def test_wing_samples_land_inside_their_islands(self):
        L = np.load(WORK / "islands.npy")
        for part in ("front_wing", "rear_wing"):
            reg, xy = self.M[f"{part}_region"], self.M[f"{part}_xy"]
            snap = self.M[f"{part}_snap"].astype(float)
            # Only rounding at island borders, never a collapse onto an edge.
            self.assertLess(snap.max(), 2.0, part)
            self.assertLess((snap > 0).sum(), 1e-3 * (reg >= 0).sum(), part)
            for i in np.unique(reg[reg >= 0]):
                m = reg == i
                ids = wing_map.ISLANDS[self.names[i]]
                c = np.clip(np.round(xy[m]).astype(int), 0, 2047)
                # float32 storage can move a border sample across a rounding edge.
                inside = np.isin(L[c[:, 1], c[:, 0]], ids).mean()
                self.assertGreater(inside, 0.9999, self.names[i])

    def test_drs_rear_face_is_fully_mapped_and_reads_from_behind(self):
        d = np.load(WORK / "bake2k/rear_wing.npz")
        steep = [c for c in wing_map.charts(d["mask"], d["pos"], d["nrm"])
                 if c["n"][0] < -0.5 and c["n"][1] < 0]
        self.assertEqual(len(steep), 1)
        back = self.region("rear_wing", "rw_flap_back")
        self.assertGreater((back & steep[0]["sel"]).sum() / steep[0]["sel"].sum(), 0.99)
        X, Z = d["pos"][..., 0], d["pos"][..., 2]
        xy = self.M["rear_wing_xy"]
        # Span is vertical: car right at the island top; trailing edge on the left.
        right, left = back & (Z > 0.5), back & (Z < -0.5)
        self.assertLess(xy[right][:, 1].mean(), xy[left][:, 1].mean())
        te, le = back & (X < -6.2), back & (X > -6.08)
        self.assertLess(xy[te][:, 0].mean(), xy[le][:, 0].mean())

    def test_front_wing_reads_from_in_front(self):
        d = np.load(WORK / "bake2k/front_wing.npz")
        X, Z = d["pos"][..., 0], d["pos"][..., 2]
        xy = self.M["front_wing_xy"]
        main = self.region("front_wing", "fw_main")
        flaps = self.region("front_wing", "fw_flap_top")
        self.assertGreater(main.sum(), 1e5)
        self.assertGreater(flaps.sum(), 1e5)
        self.assertGreater(X[main].mean(), X[flaps].mean())     # main plane is in front
        wing = main | flaps | self.region("front_wing", "fw_flap_mid")
        # Car right is image left (read from in front), symmetric about the centre.
        self.assertLess(xy[wing & (Z > 1)][:, 0].mean(), xy[wing & (Z < -1)][:, 0].mean())

    def test_endplate_text_is_not_upside_down(self):
        d = np.load(WORK / "bake2k/front_wing.npz")
        Y = d["pos"][..., 1]
        xy = self.M["front_wing_xy"]
        for name, upright in (("fw_ep_out_right", True), ("fw_ep_out_left", False),
                              ("fw_ep_in_left", True), ("fw_ep_in_right", False)):
            m = self.region("front_wing", name)
            hi, lo = m & (Y > np.percentile(Y[m], 80)), m & (Y < np.percentile(Y[m], 20))
            top_first = xy[hi][:, 1].mean() < xy[lo][:, 1].mean()
            self.assertEqual(top_first, upright, name)


if __name__ == "__main__":
    unittest.main()
