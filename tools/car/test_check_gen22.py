"""Standalone contract checks require the roster without external team textures."""
import importlib.util
import json
import struct
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('check_gen22', Path(__file__).with_name('check_gen22.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class CheckerTests(unittest.TestCase):
    def test_metre_scaled_carbon_uvs_cannot_be_used_as_paint(self):
        document = {'accessors': [{'type': 'VEC2', 'componentType': 5126,
                                   'count': 2, 'bufferView': 0}], 'bufferViews': [{}]}
        checker.check_template_uv(document, struct.pack('<4f', 0, 0, 1, 1), 0)
        with self.assertRaisesRegex(AssertionError, 'outside template'):
            checker.check_template_uv(document, struct.pack('<4f', 0, 0, 56, 56), 0)

    def test_roster_is_required_without_team_texture_directories(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'car.json').write_text('{}')
            with patch.object(sys, 'argv', ['check_gen22.py', str(root)]), patch.object(checker, 'inspect'), patch.object(checker, 'check_tyre_textures'):
                with self.assertRaisesRegex(AssertionError, 'teams.json'):
                    checker.main()
                (root / 'teams.json').write_text(json.dumps({'teams': [{'id':'test','drivers':[]}]}))
                checker.main()
                (root / 'teams.json').write_text('{"teams": []}')
                with self.assertRaisesRegex(AssertionError, 'teams must not be empty'):
                    checker.main()


if __name__ == '__main__':
    unittest.main()
