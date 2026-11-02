"""Behaviour tests for flat UV0 art, reflection, padding and editable ORA import."""
import json
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET
import zipfile

import numpy as np
from PIL import Image, ImageDraw

from compose_livery import (base_layer, compose, png_bytes, read_ora,
                            reimport_oras, seam_dilate, sticker_layer)
from uv_layout import PARTS, panel_lookup, panel_points, require_native_parts


class ComposeTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.templates = self.root/'templates'; self.templates.mkdir()
        self.logos = self.root/'logos'; self.logos.mkdir()
        self.size = 64
        self.index = {'schema': 1, 'uv_channel': 0, 'size': self.size,
                      'source_sha256': 'fixture-source', 'objects': [], 'panels': {}, 'regions': {}}
        for part in PARTS:
            mask = Image.new('L', (64, 64)); ImageDraw.Draw(mask).rectangle((8, 8, 55, 55), fill=255)
            mask.save(self.templates/f'{part}-coverage.png')
        for name, frame, mirrored in [('left', [[.125, .75, 0], [.125, 0, .75]], False),
                                      ('right', [[.875, -.75, 0], [.125, 0, .75]], True)]:
            self.index['panels'][name] = {'part': 'main_body', 'mask': 'main_body-coverage.png',
                                           'frame_uv': frame, 'mirrored': mirrored, 'shared_with': []}
        self.save_index()
        # Deliberately asymmetric flag: a red left half and a blue right half.
        art = Image.new('RGBA', (16, 8), 'red'); ImageDraw.Draw(art).rectangle((8, 0, 15, 7), fill='blue')
        art.save(self.logos/'flag.png')

    def save_index(self):
        (self.templates/'panels.json').write_text(json.dumps(self.index))

    def layout(self):
        return {'schema': 1, 'team': 'fixture', 'source_sha256': 'fixture-source', 'padding': 3,
                'parts': {part: {'color': '#202020'} for part in PARTS}}

    def test_panel_lookup_rejects_unknown_and_wrong_part(self):
        self.assertIs(panel_lookup(self.index, 'left'), self.index['panels']['left'])
        with self.assertRaisesRegex(ValueError, 'unknown UV panel'): panel_lookup(self.index, 'missing')
        with self.assertRaisesRegex(ValueError, 'not mirrors'): panel_lookup(self.index, 'left', 'mirrors')

    def test_preview_rejects_missing_or_mismatched_parts(self):
        with self.assertRaises(FileNotFoundError): require_native_parts(self.root)
        for part in PARTS: Image.new('RGB', (16, 16)).save(self.root/f'{part}.png')
        require_native_parts(self.root)
        Image.new('RGB', (32, 32)).save(self.root/'mirrors.png')
        with self.assertRaisesRegex(ValueError, 'matching dimensions'): require_native_parts(self.root)

    def test_auto_reflection_is_applied_once_and_physical_art_reads_the_same(self):
        stamps = []
        for name in ['left', 'right']:
            layer = {'image': 'flag.png', 'panel': name, 'position': [.5, .5], 'size': [.5, .25]}
            stamp = sticker_layer(layer, 'main_body', self.index, self.templates, self.logos, 64)
            stamps.append(np.array(stamp))
            for point, expected in [([.4, .5], (255, 0, 0, 255)), ([.6, .5], (0, 0, 255, 255))]:
                pixel = (panel_points(self.index['panels'][name], point)*64).astype(int)
                self.assertEqual(stamp.getpixel(tuple(pixel)), expected)
        self.assertTrue(np.array_equal(stamps[0][:, ::-1], stamps[1]))
        self.assertTrue(np.allclose(panel_points(self.index['panels']['right'], [.25, .5], 'none'), [.3125, .5]))

    def test_panel_polygon_clips_gradient_and_material_region(self):
        region = Image.new('L', (64, 64)); ImageDraw.Draw(region).rectangle((32, 0, 63, 63), fill=255)
        region.save(self.templates/'region.png')
        self.index['regions']['red-zone'] = {'part': 'main_body', 'mask': 'region.png'}
        layer = {'panel': 'left', 'region': 'red-zone', 'polygon': [[0, 0], [1, 0], [0, 1]],
                 'gradient': {'from': '#000000', 'to': '#ffffff', 'axis': [[0, 0], [1, 0]]}}
        image = base_layer(layer, 'main_body', self.index, self.templates, 64, {})
        self.assertEqual(image.getpixel((20, 20))[3], 0)
        self.assertEqual(image.getpixel((50, 50))[3], 0)
        self.assertEqual(image.getpixel((35, 16))[3], 255)
        self.assertGreater(image.getpixel((35, 16))[0], 100)

    def test_side_facing_selection_keeps_sticker_off_shoulder_texels(self):
        mask = Image.new('L', (64, 64)); ImageDraw.Draw(mask).rectangle((0, 32, 63, 63), fill=255)
        mask.save(self.templates/'side-facing.png')
        self.index['face_masks'] = {'main_body:left-side': {'part': 'main_body', 'mask': 'side-facing.png'}}
        row = {'image': 'flag.png', 'panel': 'left', 'position': [.5, .5],
               'width': .6, 'face_mask': 'main_body:left-side'}
        image = sticker_layer(row, 'main_body', self.index, self.templates, self.logos, 64)
        self.assertEqual(image.getpixel((25, 30))[3], 0)
        self.assertEqual(image.getpixel((25, 34)), (255, 0, 0, 255))

    def test_dilation_preserves_covered_texels_and_stops_at_padding(self):
        image = Image.new('RGB', (17, 17), '#000000'); image.putpixel((8, 8), (239, 7, 31))
        coverage = Image.new('L', image.size); coverage.putpixel((8, 8), 255)
        padded = seam_dilate(image, coverage, 3)
        for pixel in [(8, 8), (8, 5), (10, 10)]: self.assertEqual(padded.getpixel(pixel), (239, 7, 31))
        self.assertEqual(padded.getpixel((8, 4)), (0, 0, 0))
        self.assertTrue(np.array_equal(np.asarray(padded)[np.asarray(coverage)>0], np.asarray(image)[np.asarray(coverage)>0]))

    def test_shared_texels_need_explicit_symmetric_art_permission(self):
        self.index['panels']['left']['shared_with'] = [{'panel': 'right'}]
        layer = {'image': 'flag.png', 'panel': 'left', 'position': [.5, .5]}
        with self.assertRaisesRegex(ValueError, 'shares texels'):
            sticker_layer(layer, 'main_body', self.index, self.templates, self.logos, 64)
        layer['allow_shared'] = True
        self.assertIsInstance(sticker_layer(layer, 'main_body', self.index, self.templates, self.logos, 64), Image.Image)

    def test_rotation_preserves_art_aspect_on_a_long_thin_panel(self):
        panel = self.index['panels']['left']
        panel['frame_uv'] = [[.125, .75, 0], [.4, 0, .2]]
        layer = {'image': 'flag.png', 'panel': 'left', 'position': [.5, .5],
                 'width': .4, 'rotation': 90, 'clip': False}
        stamp = sticker_layer(layer, 'main_body', self.index, self.templates, self.logos, 64)
        x0, y0, x1, y1 = stamp.getbbox()
        self.assertAlmostEqual((y1-y0)/(x1-x0), 2, delta=.25)

    def test_flat_control_mesh_maps_asymmetric_art_and_clips_to_panel(self):
        layer = {'image': 'flag.png', 'panel': 'left',
                 'warp_uv_px': [[[12, 12], [32, 16], [52, 12]],
                                [[16, 44], [32, 48], [48, 44]]]}
        image = sticker_layer(layer, 'main_body', self.index, self.templates, self.logos, 64)
        self.assertEqual(image.getpixel((23, 30)), (255, 0, 0, 255))
        self.assertEqual(image.getpixel((41, 30)), (0, 0, 255, 255))
        self.assertEqual(image.getpixel((3, 30))[3], 0)
        # An explicit reflected destination reads in the opposite UV direction.
        layer['warp_uv_px'] = [list(reversed(row)) for row in layer['warp_uv_px']]
        image = sticker_layer(layer, 'main_body', self.index, self.templates, self.logos, 64)
        self.assertEqual(image.getpixel((41, 30)), (255, 0, 0, 255))
        self.assertEqual(image.getpixel((23, 30)), (0, 0, 255, 255))

    def test_all_nine_part_compose_and_ora_reimport_are_pixel_identical(self):
        layout = self.layout()
        layout['parts']['main_body']['stickers'] = [{'image': 'flag.png', 'panel': 'left', 'position': [.5, .5]}]
        path = self.root/'team.layout.json'; path.write_text(json.dumps(layout))
        output = self.root/'composed'
        manifest = compose(path, self.templates, self.logos, output)
        imported = self.root/'imported'; restored = reimport_oras(output, self.templates, imported, padding=3)
        self.assertEqual(manifest['uv_channel'], 0)
        self.assertEqual(restored['team'], 'fixture')
        for part in PARTS:
            self.assertEqual((output/f'{part}.png').read_bytes(), (imported/f'{part}.png').read_bytes())
        with zipfile.ZipFile(output/'main_body.ora') as archive:
            self.assertEqual(archive.namelist()[0], 'mimetype')
            self.assertEqual(len(ET.fromstring(archive.read('stack.xml')).find('stack')), 2)
            saved = {name: archive.read(name) for name in archive.namelist() if name != 'livery.json'}
        # Editors can drop unknown ZIP entries. The generated sidecar keeps
        # the UV/team contract available after a normal Krita/GIMP save.
        with zipfile.ZipFile(output/'main_body.ora', 'w') as archive:
            for name, data in saved.items(): archive.writestr(name, data)
        reimport_oras(output, self.templates, imported, padding=3)
        # A bad panel cannot replace an existing output.
        before = (output/'main_body.png').read_bytes()
        layout['parts']['main_body']['stickers'][0]['panel'] = 'bad-panel'; path.write_text(json.dumps(layout))
        with self.assertRaises(ValueError): compose(path, self.templates, self.logos, output)
        self.assertEqual((output/'main_body.png').read_bytes(), before)
        # A later corrupt artwork decode also preserves every published file.
        layout['parts']['main_body']['stickers'][0]['panel'] = 'left'
        layout['parts']['rear_covers']['stickers'] = [{'image': 'corrupt.png', 'panel': 'left', 'position': [.5, .5]}]
        # Give the late part its own valid panel, so the failure is image IO.
        self.index['panels']['late'] = dict(self.index['panels']['left'], part='rear_covers')
        self.save_index(); layout['parts']['rear_covers']['stickers'][0]['panel'] = 'late'
        (self.logos/'corrupt.png').write_bytes(b'not a PNG')
        path.write_text(json.dumps(layout))
        with self.assertRaises(OSError): compose(path, self.templates, self.logos, output)
        self.assertEqual((output/'main_body.png').read_bytes(), before)

    def test_ora_reads_edited_layers_offsets_opacity_and_hidden_state(self):
        path = self.root/'edited.ora'
        root = ET.Element('image', w='64', h='64'); stack = ET.SubElement(root, 'stack')
        ET.SubElement(stack, 'layer', src='hidden.png', visibility='hidden')
        group = ET.SubElement(stack, 'stack', opacity='.5', x='3', y='-2')
        ET.SubElement(group, 'layer', src='paint.png', x='10', y='12')
        with zipfile.ZipFile(path, 'w') as archive:
            archive.writestr('stack.xml', ET.tostring(root))
            archive.writestr('hidden.png', png_bytes(Image.new('RGBA', (64, 64), 'blue')))
            archive.writestr('paint.png', png_bytes(Image.new('RGBA', (4, 4), 'red')))
            archive.writestr('mergedimage.png', png_bytes(Image.new('RGBA', (64, 64), 'green')))
        image, metadata = read_ora(path)
        self.assertIsNone(metadata)
        self.assertEqual(image.getpixel((13, 10)), (255, 0, 0, 128))
        self.assertEqual(image.getpixel((0, 0)), (0, 0, 0, 0))


if __name__ == '__main__':
    unittest.main()
