"""UV diagnostics retain concavity, reflection and exact triangle coverage."""
import unittest
import tempfile
from pathlib import Path

import numpy as np

from PIL import Image
from uv_template import boundary_polygons, face_masks, panel_record, triangle_pixels
from uv_layout import panel_points


class UVTemplateTest(unittest.TestCase):
    def test_side_masks_exclude_upward_facing_shoulders(self):
        triangles = np.array([[[.1, .1], [.4, .1], [.1, .4]],
                              [[.6, .6], [.9, .6], [.6, .9]]])
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary); (directory/'masks').mkdir()
            masks = face_masks('body', triangles, np.array([[1, 0, 0], [.5, 0, .866]]), directory, 64)
            with Image.open(directory/masks['body:left-side']['mask']) as mask:
                self.assertEqual(mask.getpixel((10, 10)), 255)
                self.assertEqual(mask.getpixel((42, 42)), 0)

    def test_adjacent_triangles_do_not_report_shared_interior_texels(self):
        first = triangle_pixels([[0, 0], [1, 0], [0, 1]], 31)
        second = triangle_pixels([[1, 0], [1, 1], [0, 1]], 31)
        a, b = set(zip(*first)), set(zip(*second))
        self.assertFalse(a & b)
        self.assertGreater(len(a | b), 31*29)

    def test_degenerate_uv_edges_are_safe_and_concave_boundary_survives(self):
        triangles = np.array([[[0, 0], [1, 0], [.5, .5]],
                              [[0, 0], [.5, .5], [0, 1]],
                              [[0, 0], [0, 0], [0, 0]]])
        polygons = boundary_polygons(triangles)
        self.assertEqual(len(polygons), 1)
        self.assertTrue(any(np.allclose(point, [.5, .5]) for point in polygons[0]))

    def test_front_and_reflection_follow_geometry_without_changing_uv(self):
        xyz = np.array([[[1, 0, 0], [1, 1, 0], [1, 0, 1]],
                        [[1, 1, 0], [1, 1, 1], [1, 0, 1]]], float)
        # Left side: car front is screen left and upward Z is image up.
        uv = xyz[..., [1, 2]].copy(); uv[..., 1] = 1-uv[..., 1]
        original = uv.copy()
        record = panel_record('main_body', 'left', [0, 1], 0, uv, xyz,
                              np.array([[1, 0, 0]]*2), np.ones(2))
        self.assertTrue(np.array_equal(uv, original))
        self.assertTrue(np.allclose(record['front_uv'], [-1, 0], atol=1e-8))
        self.assertFalse(record['mirrored'])
        self.assertTrue(np.allclose(panel_points(record, [[0, 0], [1, 1]]), [[0, 0], [1, 1]]))
        reflected = uv.copy(); reflected[..., 0] = 1-reflected[..., 0]
        mirrored = panel_record('main_body', 'left', [0, 1], 0, reflected, xyz,
                                np.array([[1, 0, 0]]*2), np.ones(2))
        self.assertTrue(mirrored['mirrored'])
        self.assertAlmostEqual(mirrored['mean_projection_cosine'], 1)


if __name__ == '__main__':
    unittest.main()
