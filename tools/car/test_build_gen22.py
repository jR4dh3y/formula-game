"""Full build forwards custom override paths as one quoted argument."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class BuildArgumentsTests(unittest.TestCase):
    def test_selected_override_directory_reaches_generator(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            calls = root / 'calls.jsonl'
            executable = root / 'stub'
            executable.write_text('#!/usr/bin/env python3\nimport json,os,sys\nwith open(os.environ["GEN22_TEST_CALLS"],"a") as log: log.write(json.dumps(sys.argv[1:])+"\\n")\n')
            executable.chmod(0o755)
            overrides = root / 'artist overrides'
            env = dict(os.environ, BLENDER=str(executable), PYTHON=str(executable),
                       GEN22_TEST_CALLS=str(calls), GEN22_OVERRIDES=str(overrides))
            script = Path(__file__).with_name('build_gen22.sh')
            subprocess.run(['bash', str(script), str(root / 'output')], env=env, check=True)
            arguments = [json.loads(line) for line in calls.read_text().splitlines()]
            generator = next(args for args in arguments if any(arg.endswith('/livery.py') for arg in args))
            self.assertEqual(generator[generator.index('--overrides') + 1], str(overrides))


if __name__ == '__main__':
    unittest.main()
