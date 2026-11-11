"""Exercise the drift payload copied by the release builders."""
from pathlib import Path
import tempfile
import unittest
from drift_assets import FILES, copy_drift_assets


class DriftAssetTests(unittest.TestCase):
    def test_optional_pair_and_incomplete_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source"
            destination = Path(temporary) / "package"
            self.assertFalse(copy_drift_assets(source, destination))
            for directory, names in FILES.items():
                (source / directory).mkdir(parents=True)
                for name in names:
                    (source / directory / name).write_bytes(name.encode())
            missing = source / "cars/thar/car_lod1.glb"
            missing.unlink()
            with self.assertRaisesRegex(RuntimeError, "car_lod1.glb"):
                copy_drift_assets(source, destination)
            self.assertFalse(destination.exists())
            missing.write_bytes(b"lod")
            self.assertTrue(copy_drift_assets(source, destination))
            for directory, names in FILES.items():
                for name in names:
                    self.assertEqual((source / directory / name).read_bytes(),
                                     (destination / directory / name).read_bytes())
