"""Unit test for convert_2026.convert() on a synthetic map and a solid-colour sheet.

Run from the repo root:
  python3 -I -m unittest discover -s tools/livery -p 'test_*.py'
"""
import tempfile
import unittest
from pathlib import Path

import numpy as np
from PIL import Image

import convert_2026

SHEET_RGB = (200, 40, 30)  # distinct from the CARBON fallback colour
TOL = 2
N = 8        # coordinate-map resolution
SIZE = 16    # output texture size


class ConvertSyntheticTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def _write_map(self, path):
        xy = np.full((N, N, 2), np.nan, np.float32)
        ys, xs = np.mgrid[0:N, 0:N]
        xy[..., 0] = 200 + xs * 10
        xy[..., 1] = 300 + ys * 10
        xy[:, N // 2:] = np.nan              # right half unmapped -> fill texels
        fill = np.where(np.isnan(xy[..., 0]), 1, 0).astype(np.int8)
        np.savez(path,
                 names=np.array(["main_body"]),
                 main_body_xy=xy,
                 main_body_fill=fill,
                 main_body_snap=np.zeros((N, N), np.float16),
                 main_body_region=np.zeros((N, N), np.int8))

    def _write_sheet(self, path):
        Image.new("RGB", (2048, 2048), SHEET_RGB).save(path)

    def test_convert_solid_sheet(self):
        map_path = self.root / "map.npz"
        sheet_path = self.root / "sheet.png"
        out_dir = self.root / "out"
        self._write_map(map_path)
        self._write_sheet(sheet_path)

        parts = convert_2026.convert(map_path, sheet_path, out_dir, size=SIZE)

        self.assertEqual(parts, ["main_body"])
        out_png = out_dir / "main_body.png"
        self.assertTrue(out_png.is_file())
        img = np.asarray(Image.open(out_png).convert("RGB")).astype(int)
        self.assertEqual(img.shape, (SIZE, SIZE, 3))

        target = np.array(SHEET_RGB)
        mapped = img[:, :SIZE // 2]
        fill = img[:, SIZE // 2:]
        self.assertLessEqual(np.abs(mapped - target).max(), TOL,
                             "mapped texels should carry the sheet colour")
        # The fill colour is the dominant colour of the mapped pixels, i.e. the sheet colour.
        self.assertLessEqual(np.abs(fill - target).max(), TOL,
                             "fill texels should carry the dominant colour")


if __name__ == "__main__":
    unittest.main()

class SeamTest(unittest.TestCase):
    def test_disjoint_charts_do_not_sample_between_islands(self):
        xy = np.array([[[10,20],[1000,1200]],[[10,20],[1000,1200]]],np.float32)
        region = np.array([[0,1],[0,1]])
        out,_ = convert_2026.resample_map(xy,np.zeros((2,2),int),region,8,0,8)
        np.testing.assert_allclose(out[:,:4],np.broadcast_to([10,20],(8,4,2)))
        np.testing.assert_allclose(out[:,4:],np.broadcast_to([1000,1200],(8,4,2)))

    def test_carbon_fill_is_not_dilated_over(self):
        xy = np.array([[[10,20],[np.nan,np.nan]],[[10,20],[np.nan,np.nan]]],np.float32)
        fill = np.array([[0,2],[0,2]])
        out,kind = convert_2026.resample_map(xy,fill,np.where(fill==0,0,-1),8,0,8)
        self.assertTrue((kind[:,4:]==2).all())
        self.assertTrue(np.isnan(out[:,4:]).all())
