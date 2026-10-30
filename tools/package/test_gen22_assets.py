"""Package preflight rejects incomplete GEN22 asset trees without building."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from PIL import Image

spec = importlib.util.spec_from_file_location('build_appimage', Path(__file__).with_name('build_appimage.py'))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class Gen22PackageTests(unittest.TestCase):
    def test_empty_roster_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ('car.glb', 'car_lod1.glb', 'car.json'):
                (root / name).write_text('fixture')
            (root / 'teams.json').write_text('{"teams": []}')
            with self.assertRaisesRegex(RuntimeError, 'roster must not be empty'):
                package.validate_gen22_assets(root)

    def test_every_required_file_is_checked_including_nested_helmets(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            roster = {'teams': [{'id': 'ferrari', 'drivers': [{'id': 'leclerc'}, {'id': 'sainz'}]}]}
            paths = ['car.glb', 'car_lod1.glb', 'car.json', 'teams.json',
                     'liveries/ferrari/paint.png', 'liveries/ferrari/suit.png',
                     'liveries/ferrari/leclerc/helmet.png', 'liveries/ferrari/sainz/helmet.png']
            for name in paths:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                if name.endswith('.png'):
                    Image.new('RGB', (2048, 2048)).save(path)
                else:
                    path.write_text(json.dumps(roster) if name == 'teams.json' else 'fixture')
            package.validate_gen22_assets(root)
            for name in paths:
                path = root / name
                contents = path.read_bytes()
                path.unlink()
                with self.subTest(name=name), self.assertRaisesRegex(RuntimeError, name):
                    package.validate_gen22_assets(root)
                path.write_bytes(contents)
            path = root / 'liveries/ferrari/paint.png'
            path.write_bytes(b'truncated PNG')
            with self.assertRaisesRegex(RuntimeError, 'Invalid GEN22 livery texture'):
                package.validate_gen22_assets(root)
            Image.new('RGB', (512, 512)).save(path)
            with self.assertRaisesRegex(RuntimeError, '2048x2048'):
                package.validate_gen22_assets(root)


if __name__ == '__main__':
    unittest.main()
