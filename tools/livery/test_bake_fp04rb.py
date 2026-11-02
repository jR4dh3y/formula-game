"""Rasterized positions must describe texel centres, including thin triangles."""
import unittest
import numpy as np
from bake_fp04rb import raster_samples

class RasterTest(unittest.TestCase):
    def test_affine_square_has_no_holes_or_position_jitter(self):
        tris = np.array([[[0,0],[8,0],[8,8]], [[0,0],[8,8],[0,8]]],float)
        out = np.full((8,8,2), np.nan)
        for face,x,y,w in raster_samples(tris,8):
            out[y,x] = w @ tris[face]
        y,x = np.mgrid[:8,:8]
        np.testing.assert_allclose(out, np.stack([x+.5,y+.5],-1))

    def test_no_splats_outside_triangle(self):
        tri = np.array([[[1,1],[6,1],[1,6]]],float)
        for _,x,y,w in raster_samples(tri,8):
            self.assertTrue(((x+.5+y+.5) <= 7).all())
            self.assertTrue((w >= -1e-9).all())
