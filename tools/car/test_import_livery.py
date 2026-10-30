"""PNG import transactions and persistent rebuild overrides."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from PIL import Image

spec = importlib.util.spec_from_file_location('import_livery', Path(__file__).with_name('import_livery.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ImportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.assets = self.root / 'assets'
        self.overrides = self.root / 'raw'
        self.roster = {'teams': [{'id': 'ferrari', 'drivers': [{'id': 'leclerc'}]}]}
        self.directory = self.assets / 'liveries/ferrari'
        (self.directory / 'leclerc').mkdir(parents=True)
        for path in ['paint.png', 'suit.png', 'leclerc/helmet.png']:
            Image.new('RGB', (2048, 2048), (0, 0, 255)).save(self.directory / path)
        (self.assets / 'teams.json').write_text(json.dumps(self.roster))
        self.input = self.root / 'artist.png'
        Image.new('RGBA', (512, 512), (255, 0, 0, 128)).save(self.input)

    def test_car_suit_and_helmet_imports_preserve_alpha_and_survive_rebuild(self):
        paths = module.install(self.assets, self.overrides, self.roster, 'ferrari', 'leclerc',
                               {'paint': self.input, 'suit': self.input, 'helmet': self.input})
        self.assertEqual(len(paths), 3)
        for path in paths:
            with Image.open(path) as image:
                self.assertEqual(image.size, (2048, 2048))
                self.assertEqual(image.getpixel((10, 10)), (128, 0, 127))
            Image.new('RGB', (2048, 2048), (0, 255, 0)).save(path)
        module.apply_overrides(self.assets, self.overrides, self.roster)
        for path in paths:
            with Image.open(path) as image:
                self.assertEqual(image.getpixel((10, 10)), (128, 0, 127))

    def test_invalid_second_input_does_not_replace_first(self):
        invalid = self.root / 'invalid.png'
        Image.new('RGB', (512, 128)).save(invalid)
        previous = (self.directory / 'paint.png').read_bytes()
        with self.assertRaises(ValueError):
            module.install(self.assets, self.overrides, self.roster, 'ferrari', '',
                           {'paint': self.input, 'suit': invalid})
        self.assertEqual((self.directory / 'paint.png').read_bytes(), previous)
        self.assertFalse(self.overrides.exists())

    def test_late_publish_failure_restores_live_and_persistent_bytes(self):
        persistent = self.overrides / 'ferrari/paint.png'
        persistent.parent.mkdir(parents=True)
        persistent.write_bytes(b'previous persistent bytes')
        paths = [self.directory / 'paint.png', self.directory / 'suit.png', persistent]
        before = {path: path.read_bytes() for path in paths}
        replace = Path.replace
        failed = False

        def fail_late(path, target):
            nonlocal failed
            if Path(target) == self.directory / 'suit.png' and not failed:
                failed = True
                raise PermissionError('injected late publish failure')
            return replace(path, target)

        with patch.object(Path, 'replace', fail_late), self.assertRaises(PermissionError):
            module.install(self.assets, self.overrides, self.roster, 'ferrari', '',
                           {'paint': self.input, 'suit': self.input})
        self.assertTrue(failed)
        for path, data in before.items():
            self.assertEqual(path.read_bytes(), data)
        self.assertFalse((self.overrides / 'ferrari/suit.png').exists())
        self.assertFalse(list(self.root.rglob('.gen22-*')))

    def test_unknown_identity_is_rejected(self):
        for team, driver in [('missing', 'leclerc'), ('ferrari', 'missing'), ('../escape', '')]:
            with self.assertRaises(ValueError):
                module.install(self.assets, self.overrides, self.roster, team, driver, {'helmet': self.input})

    def test_foreign_uv_layout_is_not_silently_installed(self):
        result = subprocess.run([sys.executable, '-I', str(Path(module.__file__)),
                                 '--assets', str(self.assets), '--team', 'ferrari',
                                 '--paint', str(self.input), '--layout', 'monoposto22'], capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.overrides.exists())


if __name__ == '__main__':
    unittest.main()
