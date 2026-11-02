"""Installer rejects unknown destinations before writing artwork."""
from pathlib import Path
import tempfile
import unittest

import numpy as np
from PIL import Image

from install_liveries import install_direct, mappings


class InstallTest(unittest.TestCase):
    def test_roster_validation_and_repeated_pairs(self):
        roster = {'teams': [{'id': 'mclaren'}, {'id': 'ferrari'}]}
        with tempfile.TemporaryDirectory() as directory:
            sheet = Path(directory) / 'sheet.png'
            sheet.touch()
            self.assertEqual(mappings([f'mclaren={sheet}', f'ferrari={sheet}'], roster),
                             {'mclaren': sheet, 'ferrari': sheet})
            for pair in [f'../escape={sheet}', f'unknown={sheet}', 'mclaren', 'mclaren=missing.png']:
                with self.assertRaises(ValueError):
                    mappings([pair], roster)
            with self.assertRaises(ValueError):
                mappings([f'mclaren={sheet}', f'mclaren={sheet}'], roster)


class DirectSheetTest(unittest.TestCase):
    def test_direct_sheet_preserves_panel_pixels_and_pads_gutters(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pixels = np.zeros((2048, 2048, 3), np.uint8)
            pixels[100:200, 100:200] = [240, 30, 10]
            pixels[20:40, 20:40] = [10, 80, 240]
            pixels[55:75, 55:75] = [240, 150, 10]
            pixels[120, 150] = [10, 80, 240]
            source = root / 'source.png'
            Image.fromarray(pixels).save(source)
            labels = np.zeros((2048, 2048), np.int16)
            labels[100:200, 100:200] = 1
            labels[20:40, 20:40] = 2
            labels[55:75, 55:75] = 28
            destination = root / 'ferrari'
            install_direct(source, destination, labels)
            result = np.array(Image.open(destination / 'template.png'))
            np.testing.assert_array_equal(result[labels > 0], pixels[labels > 0])
            np.testing.assert_array_equal(result[99, 150], pixels[100, 150])
            np.testing.assert_array_equal(result[50, 150], pixels[100, 150])
            np.testing.assert_array_equal(result[20, 150], pixels[20, 150])
            np.testing.assert_array_equal(result[80, 80], pixels[100, 100])
            np.testing.assert_array_equal(result[:16, :16], pixels[:16, :16])
            self.assertEqual((destination / 'template.png').read_bytes(),
                             (destination / 'lod1/template.png').read_bytes())
            with Image.open(destination / 'lod2/template.png') as image:
                self.assertEqual(image.size, (1024, 1024))
