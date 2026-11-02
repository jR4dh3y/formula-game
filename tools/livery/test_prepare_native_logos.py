"""Artwork preparation must keep negative space and the source mark's proportions."""
from pathlib import Path
import tempfile
import unittest

import numpy as np
from PIL import Image
from prepare_native_logos import prepare, transparent_background


class NativeLogoTest(unittest.TestCase):
    def test_svg_component_keeps_genuine_star_path_without_ring(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M0 0H100V100H0Z"/><path d="M50 20L80 80H20Z"/></svg>'
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'emblem.svg'
            source.write_text(svg)
            result = prepare({'svg_paths': [1], 'color': 'white'}, source)
            self.assertEqual(result.getpixel((0, 0))[3], 0)
            self.assertEqual(result.getpixel((result.width//2, result.height//2))[3], 255)

    def test_polygon_excludes_dark_photo_panel_edge_from_real_glyphs(self):
        pixels = np.full((30, 50, 4), (240, 130, 0, 255), np.uint8)
        pixels[:, :3, :3] = 0
        pixels[10:20, 20:40, :3] = 0
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'wing.png'
            Image.fromarray(pixels).save(source)
            result = prepare({'source_polygon': [[15, 5], [45, 5], [45, 25], [15, 25]], 'dark_ink': 50}, source)
            self.assertEqual(result.size, (20, 10))

    def test_photo_rectification_samples_only_the_measured_quad(self):
        pixels = np.full((30, 50, 4), (20, 90, 80, 255), np.uint8)
        pixels[5:25, 10:40, :3] = (210, 230, 50)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'number.png'
            Image.fromarray(pixels).save(source)
            result = prepare({'source_quad': [12, 7, 12, 23, 38, 23, 38, 7],
                              'rectified_size': [52, 32],
                              'rgb_ink': [[150, 160, 0], [255, 255, 120]]}, source)
            self.assertEqual(result.size, (52, 32))
            self.assertTrue(np.all(np.asarray(result)[:, :, 3] == 255))

    def test_source_rotation_precedes_photo_ink_extraction(self):
        pixels = np.full((20, 30, 4), (235, 130, 10, 255), np.uint8)
        pixels[4:8, 3:23, :3] = 0
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'word.png'
            Image.fromarray(pixels).save(source)
            result = prepare({'source_rotate': 90, 'dark_ink': 50}, source)
            self.assertEqual(result.size, (4, 20))

    def test_component_cleanup_removes_photo_debris_and_mirrors_only_the_artwork(self):
        pixels = np.zeros((20, 40, 4), np.uint8)
        pixels[5:15, 10:25] = (240, 20, 20, 255)
        pixels[5:10, 20:30] = (240, 20, 20, 255)
        pixels[1, 1] = (240, 20, 20, 255)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'bull.png'
            Image.fromarray(pixels).save(source)
            result = prepare({'largest_components': 1, 'flip_x': True}, source)
            self.assertEqual(result.size, (20, 10))
            self.assertEqual(result.getpixel((0, 9))[3], 0)
            self.assertEqual(result.getpixel((19, 9))[3], 255)

    def test_colour_photo_glyph_excludes_body_and_keeps_counter(self):
        pixels = np.full((30, 50, 4), (10, 90, 80, 255), np.uint8)
        pixels[5:25, 10:40, :3] = (210, 230, 50)
        pixels[10:20, 20:30, :3] = (10, 90, 80)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'number.png'
            Image.fromarray(pixels).save(source)
            result = prepare({'rgb_ink': [[150, 160, 0], [255, 255, 120]], 'color': '#D2E632'}, source)
            self.assertEqual(result.size, (30, 20))
            self.assertEqual(result.getpixel((1, 1)), (210, 230, 50, 255))
            self.assertEqual(result.getpixel((15, 10))[3], 0)

    def test_dark_photo_lettering_excludes_coloured_panel_and_keeps_counters(self):
        pixels = np.full((30, 50, 4), (245, 135, 15, 255), np.uint8)
        pixels[5:25, 10:40, :3] = (18, 20, 23)
        pixels[10:20, 20:30, :3] = (190, 190, 190)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'photo.png'
            Image.fromarray(pixels).save(source)
            result = prepare({'dark_ink': 60, 'color': 'black'}, source)
            self.assertEqual(result.size, (30, 20))
            self.assertEqual(result.getpixel((1, 1)), (0, 0, 0, 255))
            self.assertEqual(result.getpixel((15, 10))[3], 0)

    def test_source_crop_excludes_campaign_marks_before_extracting_ink(self):
        pixels = np.full((50, 100, 4), 255, np.uint8)
        pixels[5:15, 5:15, :3] = (220, 20, 20)
        pixels[25:35, 30:90, :3] = (220, 20, 20)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'brand.png'
            Image.fromarray(pixels).save(source)
            result = prepare({'source_crop': [20, 20, 95, 40], 'red_ink': True}, source)
            self.assertEqual(result.size, (60, 10))
            self.assertEqual(result.getpixel((1, 1)), (255, 255, 255, 255))

    def test_only_exterior_white_is_removed(self):
        pixels = np.full((20, 30, 4), 255, np.uint8)
        pixels[3:17, 5:25, :3] = 0
        pixels[7:13, 10:20, :3] = 255
        result = np.array(transparent_background(Image.fromarray(pixels)))
        self.assertEqual(result[0,0,3], 0)
        self.assertEqual(result[9,15,3], 255)
        self.assertEqual(result[4,7,3], 255)

    def test_opaque_download_canvas_cannot_become_a_white_decal_rectangle(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'brand.png'
            pixels = np.full((60,60,4),255,np.uint8)
            pixels[20:40,10:50,:3] = (30,30,30)
            Image.fromarray(pixels).save(source)
            result = prepare({'remove_all_white': True, 'color': 'white'}, source)
            self.assertEqual(result.size, (40,20))
            self.assertEqual(result.getpixel((1,1)), (255,255,255,255))

    def test_pirelli_brand_tile_becomes_only_transparent_wordmark(self):
        pixels = np.full((20, 60, 4), (255,220,0,255), np.uint8)
        pixels[5:15, 10:50] = (235,20,25,255)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'tile.png'
            Image.fromarray(pixels).save(source)
            result = prepare({'red_ink': True}, source)
            self.assertEqual(result.size, (40,10))
            self.assertEqual(result.getpixel((20,5)), (255,255,255,255))


if __name__ == '__main__':
    unittest.main()
