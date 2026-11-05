"""Add-on archives replace whole team folders and leave other assets alone."""
import hashlib
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from overlay_assets import overlay


class OverlayAssetsTests(unittest.TestCase):
    def test_replaces_team_folders_without_stale_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            assets = root / "assets"
            liveries = assets / "cars/gen22/liveries"
            (liveries / "ferrari").mkdir(parents=True)
            (liveries / "ferrari/old.png").write_text("stale")
            (liveries / "haas").mkdir()
            (liveries / "haas/main_body.png").write_text("keep")
            (assets / "cars/gen22/car.glb").write_text("car")
            archive = root / "liveries.zip"
            with zipfile.ZipFile(archive, "w") as z:
                z.writestr("assets/cars/gen22/liveries/ferrari/main_body.png", "new")
                z.writestr("assets/cars/gen22/liveries/cadillac/main_body.png", "new")
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            names = overlay(archive, assets, digest, "cars/gen22/liveries")
            self.assertEqual(names, ["cadillac", "ferrari"])
            self.assertFalse((liveries / "ferrari/old.png").exists())
            self.assertEqual((liveries / "ferrari/main_body.png").read_text(), "new")
            self.assertEqual((liveries / "haas/main_body.png").read_text(), "keep")
            self.assertEqual((assets / "cars/gen22/car.glb").read_text(), "car")

    def test_rejects_a_wrong_hash(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "liveries.zip"
            with zipfile.ZipFile(archive, "w") as z:
                z.writestr("assets/cars/gen22/liveries/ferrari/main_body.png", "new")
            with self.assertRaises(ValueError):
                overlay(archive, root / "assets", "0" * 64, "cars/gen22/liveries")


if __name__ == "__main__":
    unittest.main()
