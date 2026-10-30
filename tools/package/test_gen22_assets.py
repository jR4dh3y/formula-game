"""Package preflight rejects incomplete GEN22 asset trees without building."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('build_appimage', Path(__file__).with_name('build_appimage.py'))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class Gen22PackageTests(unittest.TestCase):
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
                path.write_text(json.dumps(roster) if name == 'teams.json' else 'fixture')
            package.validate_gen22_assets(root)
            for name in paths:
                path = root / name
                contents = path.read_bytes()
                path.unlink()
                with self.subTest(name=name), self.assertRaisesRegex(RuntimeError, name):
                    package.validate_gen22_assets(root)
                path.write_bytes(contents)


if __name__ == '__main__':
    unittest.main()
