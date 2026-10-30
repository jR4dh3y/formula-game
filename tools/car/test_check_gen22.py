"""Standalone contract checks fail on missing roster and textures."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
from PIL import Image

spec = importlib.util.spec_from_file_location('check_gen22', Path(__file__).with_name('check_gen22.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class CheckerTests(unittest.TestCase):
    def test_roster_and_all_liveries_are_required(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'car.json').write_text('{}')
            with patch.object(sys, 'argv', ['check_gen22.py', str(root)]), patch.object(checker, 'inspect'):
                with self.assertRaisesRegex(AssertionError, 'teams.json'):
                    checker.main()
                roster = {'teams': [{'id': 'test', 'drivers': [{'id': 'driver'}]}]}
                (root / 'teams.json').write_text(json.dumps(roster))
                for name in ['paint.png', 'suit.png', 'driver/helmet.png']:
                    with self.assertRaises(AssertionError):
                        checker.main()
                    path = root / 'liveries/test' / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    Image.new('RGB', (2048, 2048)).save(path)
                checker.main()


if __name__ == '__main__':
    unittest.main()
