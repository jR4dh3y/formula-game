"""Regressions for source-chart cross sections, independent of Blender/GPU."""
import unittest
from types import SimpleNamespace

import numpy as np

from author_uv import boundary_transfer


class BoundaryTransferTests(unittest.TestCase):
    def test_notched_panel_keeps_interior_letter_rows_straight(self):
        # A missing corner changes the outer section, but must not shear text
        # on the uninterrupted interior field.
        points = [(u, v) for u, lo, hi in
                  [(0, 0, 1), (.25, .15, .85), (.75, .2, .8), (1, 0, 1)]
                  for v in (lo, .37, .63, hi)]
        faces = [SimpleNamespace(loop_indices=[4*x+y, 4*(x+1)+y,
                                               4*(x+1)+y+1, 4*x+y+1])
                 for x in range(3) for y in range(3)]
        mesh = SimpleNamespace(polygons=faces)
        original = SimpleNamespace(data=[SimpleNamespace(uv=p) for p in points])
        rows = [[0, 0, 1, 0, .60, 0, .72], [1, 0, 1, 1, .60, 1, .72]]
        mapped = boundary_transfer(mesh, original, range(9), rows,
                                   interior={'source_v': [.37, .63],
                                             'guide_v': [.625, .662]})
        np.testing.assert_allclose([mapped[i][1] for i in (5, 9)], [.625, .625])
        np.testing.assert_allclose([mapped[i][1] for i in (6, 10)], [.662, .662])

    def test_chevron_span_samples_full_chord_at_each_section(self):
        # Bbox U would crop this sliding chord as the source V span changes.
        points = [(0, 0), (1, 0), (3, 1), (2, 1)]
        mesh = SimpleNamespace(polygons=[SimpleNamespace(loop_indices=range(4))])
        original = SimpleNamespace(data=[SimpleNamespace(uv=p) for p in points])
        rows = [[0, 0, 1, .64, .76, .70, .76],
                [1, 0, 1, .65, .99, .71, .99]]
        mapped = boundary_transfer(mesh, original, [0], rows, swap=True)
        np.testing.assert_allclose([mapped[i] for i in range(4)],
                                   [[.64, .76], [.70, .76], [.71, .99], [.65, .99]])

    def test_reverse_and_flip_are_independent(self):
        mesh = SimpleNamespace(polygons=[SimpleNamespace(loop_indices=range(4))])
        original = SimpleNamespace(data=[SimpleNamespace(uv=p) for p in
                                        [(0, 0), (1, 0), (1, 1), (0, 1)]])
        rows = [[0, 0, 1, 0, 0, 0, 1], [1, 0, 1, 1, 0, 1, 1]]
        mapped = boundary_transfer(mesh, original, [0], rows, reverse=True, flip=True)
        np.testing.assert_allclose([mapped[i] for i in range(4)],
                                   [[1, 1], [0, 1], [0, 0], [1, 0]])


if __name__ == '__main__':
    unittest.main()
