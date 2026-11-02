"""Native installation keeps UV1 teams intact and never leaves stale UV1 shadows."""
import json
from pathlib import Path
import tempfile
import unittest

from PIL import Image
from install_liveries import install_native, mappings, NATIVE_PARTS


class NativeInstallTest(unittest.TestCase):
    def test_native_install_and_rejected_bake_preserve_other_teams(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bake = root / 'bake'
            bake.mkdir()
            manifest = {'schema': 1, 'team': 'ferrari', 'size': 32,
                        'uv_channel': 0, 'parts': list(NATIVE_PARTS)}
            (bake / 'native-manifest.json').write_text(json.dumps(manifest))
            for part in NATIVE_PARTS:
                Image.new('RGB', (32, 32), (210, 5, 35)).save(bake / (part + '.png'))
            ferrari = root / 'ferrari'
            mclaren = root / 'mclaren'
            for team in (ferrari, mclaren):
                for level in ('', 'lod1', 'lod2'):
                    (team / level).mkdir(parents=True, exist_ok=True)
                    (team / level / 'template.png').write_bytes(b'preserve me')
            self.assertEqual(mappings([f'ferrari={bake}'], {'teams': [{'id': 'ferrari'}]}, True), {'ferrari': bake})
            # Missing input is rejected before any installed texture changes.
            (bake / 'mirrors.png').unlink()
            with self.assertRaises(FileNotFoundError):
                install_native(bake, ferrari, 'ferrari', 32)
            self.assertEqual((ferrari / 'template.png').read_bytes(), b'preserve me')
            Image.new('RGB', (32, 32), (210, 5, 35)).save(bake / 'mirrors.png')
            install_native(bake, ferrari, 'ferrari', 32)
            for level, size in [('', 32), ('lod1', 2048), ('lod2', 1024)]:
                self.assertFalse((ferrari / level / 'template.png').exists())
                self.assertEqual((mclaren / level / 'template.png').read_bytes(), b'preserve me')
                with Image.open(ferrari / level / 'main_body.png') as image:
                    self.assertEqual(image.size, (size, size))
                    self.assertEqual(image.getpixel((size//2, size//2)), (210, 5, 35))
            with self.assertRaises(ValueError):
                install_native(bake, ferrari, 'mclaren', 32)


if __name__ == '__main__':
    unittest.main()
