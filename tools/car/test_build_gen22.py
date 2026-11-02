"""Build disables autoexec and forwards the original texture directory."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class BuildArgumentsTests(unittest.TestCase):
    def test_source_textures_and_safety_flags_reach_blender(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            calls = root / 'calls.jsonl'
            executable = root / 'stub'
            executable.write_text('#!/usr/bin/env python3\nimport json,os,sys\nwith open(os.environ["GEN22_TEST_CALLS"],"a") as log: log.write(json.dumps(sys.argv[1:])+"\\n")\n')
            executable.chmod(0o755)
            textures = root / 'source textures'
            env = dict(os.environ, BLENDER=str(executable), PYTHON=str(executable),
                       GEN22_TEST_CALLS=str(calls), GEN22_TEXTURES=str(textures))
            script = Path(__file__).with_name('build_gen22.sh')
            subprocess.run(['bash', str(script), str(root / 'output')], env=env, check=True)
            arguments = [json.loads(line) for line in calls.read_text().splitlines()]
            self.assertEqual(len(arguments), 4)
            self.assertIn('--dark-wireframe', arguments[0])
            for args in arguments[2:]:
                self.assertIn('--disable-autoexec', args)
                self.assertIn('--python-exit-code', args)
            conversion = arguments[2]
            self.assertIn(str(textures), arguments[1])
            self.assertTrue(conversion[conversion.index('--textures')+1].endswith('/textures'))
            self.assertEqual(conversion[conversion.index('--template-year')+1], '2025')



if __name__ == '__main__':
    unittest.main()
