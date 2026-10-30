"""Projection coverage, sampling, batch isolation, and real importer integration."""

import importlib.util
import json
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import numpy as np
from PIL import Image

spec = importlib.util.spec_from_file_location(
    "convert_livery", Path(__file__).with_name("convert_livery.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ConvertTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "artist.png"
        Image.new("RGB", (2048, 2048), (190, 80, 30)).save(self.source)
        self.bake = self.root / "paint.npz"
        points = np.zeros((16, 16, 3), np.float32)
        points[:] = [0, 0.5, 2.0]  # Nose, safely inside its mapped panel.
        points[0, 0] = [0, 0, 0]  # An uncovered texel, not a sample from source UV0.
        mask = np.ones((16, 16), np.uint8)
        mask[0, 0] = 0
        self.write_bake(mask)

    def write_bake(self, mask, uv=None):
        if uv is None:
            uv = np.broadcast_to([0.5, 0.5], (*mask.shape, 2)).copy()
        np.savez(
            self.bake,
            source_uv=uv,
            mask=mask,
            correspondence_sha256=np.array(
                hashlib.sha256(module.DEFAULT_PROFILE.read_bytes()).hexdigest()
            ),
        )

    def test_bilinear_sampling_and_alpha(self):
        pixels = np.array(
            [
                [[255, 0, 0, 0], [0, 255, 0, 64]],
                [[0, 0, 255, 128], [255, 255, 255, 255]],
            ],
            np.uint8,
        )
        got = module.sample_rgba(
            Image.fromarray(pixels), np.array([[0, 0], [1, 1], [0.5, 0.5]], np.float32)
        )
        np.testing.assert_array_equal(got[0], pixels[0, 0])
        np.testing.assert_array_equal(got[1], pixels[1, 1])
        np.testing.assert_allclose(got[2], pixels.mean(axis=(0, 1)))

    def test_conversion_keeps_sources_and_writes_traceable_report(self):
        original = self.source.read_bytes()
        converter = module.Converter(self.bake)
        output = self.root / "converted.png"
        converter.convert(self.source, output)
        self.assertEqual(self.source.read_bytes(), original)
        with self.assertRaises(ValueError):
            converter.convert(self.source, self.source)
        with Image.open(output) as image:
            self.assertEqual(image.size, (16, 16))
            self.assertEqual(image.getpixel((8, 8)), (190, 80, 30))
        report = json.loads(output.with_name(output.name + ".json").read_text())
        self.assertEqual(report["mapped_texels"], 255)
        self.assertEqual(len(report["paint_bake_sha256"]), 64)

    def test_missing_and_mismatched_mesh_correspondence_rejected(self):
        np.savez(self.bake, mask=np.ones((2, 2)))
        with self.assertRaisesRegex(ValueError, "lacks mesh"):
            module.Converter(self.bake)
        np.savez(
            self.bake,
            mask=np.ones((2, 2)),
            source_uv=np.zeros((2, 2, 2)),
            correspondence_sha256=np.array("wrong"),
        )
        with self.assertRaisesRegex(ValueError, "differs"):
            module.Converter(self.bake)

    def test_invalid_source_uv_and_incompatible_sheet_size_are_rejected(self):
        self.write_bake(np.ones((2, 2), np.uint8), np.full((2, 2, 2), 1.01))
        with self.assertRaisesRegex(ValueError, "out of bounds"):
            module.Converter(self.bake)
        self.write_bake(np.ones((2, 2), np.uint8))
        Image.new("RGB", (1024, 1024)).save(self.source)
        with self.assertRaisesRegex(ValueError, "2048"):
            module.Converter(self.bake).convert(self.source, self.root / "wrong.png")

    def test_original_chart_cage_preserves_landmarks_and_orientation(self):
        sys.path.insert(0, str(Path(module.__file__).parent))
        from author_uv import strip

        rows = [[0, 0, 1, 0.2, 0.1, 0.4, 0.1], [1, 0, 1, 0.3, 0.8, 0.5, 0.8]]
        got = strip(np.array([[0, 0], [0, 1], [1, 0], [1, 1], [0.5, 0.5]]), rows)
        np.testing.assert_allclose(
            got, [[0.2, 0.1], [0.4, 0.1], [0.3, 0.8], [0.5, 0.8], [0.35, 0.45]]
        )

    def test_real_chart_boundaries_drive_interior_uvs(self):
        from types import SimpleNamespace as NS

        sys.path.insert(0, str(Path(module.__file__).parent))
        from author_uv import boundary_transfer

        coordinates = [
            (0, 0),
            (1, 0),
            (0.5, 0.5),
            (1, 0),
            (1, 1),
            (0.5, 0.5),
            (1, 1),
            (0, 1),
            (0.5, 0.5),
            (0, 1),
            (0, 0),
            (0.5, 0.5),
        ]
        original = NS(data=[NS(uv=p) for p in coordinates])
        mesh = NS(
            polygons=[NS(loop_indices=list(range(i, i + 3))) for i in range(0, 12, 3)]
        )
        rows = [[0, 0, 1, 0.2, 0.1, 0.4, 0.1], [1, 0, 1, 0.3, 0.8, 0.5, 0.8]]
        result = boundary_transfer(mesh, original, list(range(4)), rows)
        np.testing.assert_allclose(result[2], [0.35, 0.45])
        np.testing.assert_allclose(result[0], [0.2, 0.1])
        np.testing.assert_allclose(result[4], [0.5, 0.8])
        reversed_chart = boundary_transfer(
            mesh, original, list(range(4)), rows, reverse=True, flip=True
        )
        np.testing.assert_allclose(reversed_chart[0], [0.5, 0.8])
        np.testing.assert_allclose(reversed_chart[4], [0.2, 0.1])

    def test_batch_continues_past_invalid_png_and_preserves_subfolders(self):
        inputs = self.root / "inputs"
        (inputs / "nested").mkdir(parents=True)
        (inputs / "nested/good.PNG").write_bytes(self.source.read_bytes())
        (inputs / "bad.png").write_bytes(b"not a png")
        output = self.root / "output"
        command = [
            sys.executable,
            "-I",
            module.__file__,
            str(inputs),
            "--output",
            str(output),
            "--bake",
            str(self.bake),
            "--recursive",
        ]
        result = subprocess.run(command, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue((output / "nested/good.PNG").is_file())
        self.assertFalse((output / "bad.png").exists())
        self.assertIn(b"FAILED", result.stderr)
        command[command.index(str(output))] = str(inputs / "converted")
        result = subprocess.run(command, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"outside", result.stderr)

    def test_direct_import_converts_and_persists_paint(self):
        points = np.broadcast_to([0, 0.5, 2.0], (256, 256, 3)).astype(np.float32)
        self.write_bake(np.ones((256, 256), np.uint8))
        assets = self.root / "assets"
        (assets / "liveries/ferrari").mkdir(parents=True)
        Image.new("RGB", (2048, 2048), (0, 0, 255)).save(
            assets / "liveries/ferrari/paint.png"
        )
        (assets / "teams.json").write_text(
            json.dumps({"teams": [{"id": "ferrari", "drivers": []}]})
        )
        overrides = self.root / "overrides"
        result = subprocess.run(
            [
                sys.executable,
                "-I",
                str(Path(module.__file__).with_name("import_livery.py")),
                "--assets",
                str(assets),
                "--overrides",
                str(overrides),
                "--team",
                "ferrari",
                "--layout",
                "monoposto-modern",
                "--paint",
                str(self.source),
                "--bake",
                str(self.bake),
            ],
            capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertTrue((overrides / "ferrari/paint.png").exists())
        self.assertTrue((overrides / "ferrari/paint.source.json").exists())
        # Repacking the atlas must regenerate from the source, not reuse old UVs.
        points[:] = [0, -2, -4]
        self.write_bake(np.ones((256, 256), np.uint8), np.full((256, 256, 2), -1.0))
        importer_spec = importlib.util.spec_from_file_location(
            "importer", Path(module.__file__).with_name("import_livery.py")
        )
        importer = importlib.util.module_from_spec(importer_spec)
        importer_spec.loader.exec_module(importer)
        sys.path.insert(0, str(Path(module.__file__).parent))
        importer.apply_overrides(
            assets, overrides, {"teams": [{"id": "ferrari", "drivers": []}]}, self.bake
        )
        with Image.open(assets / "liveries/ferrari/paint.png") as image:
            self.assertEqual(image.getpixel((128, 128)), (18, 20, 23))
        importer.install(
            assets,
            overrides,
            {"teams": [{"id": "ferrari", "drivers": []}]},
            "ferrari",
            "",
            {"paint": self.source},
        )
        self.assertFalse((overrides / "ferrari/paint.source.json").exists())

    def test_non_square_source_is_rejected(self):
        Image.new("RGB", (512, 256)).save(self.source)
        with self.assertRaises(ValueError):
            module.Converter(self.bake).convert(self.source, self.root / "bad.png")


if __name__ == "__main__":
    unittest.main()
