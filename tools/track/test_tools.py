"""Checks at the archive, geometry and output-format boundaries."""

import json
import shutil
import stat
import struct
import tempfile
import unittest
import zipfile
from pathlib import Path

import numpy as np
from convert_glb import walls_from
from extract_zip import extract
from glb import GlbMaterial, GlbMesh, write_glb
from PIL import Image
from read_glb import Glb


class AssetToolsTest(unittest.TestCase):
    def test_rejects_traversal_links_and_duplicate_members(self):
        for kind in ["traversal", "link", "duplicate"]:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as root:
                path = Path(root) / "source.zip"
                with zipfile.ZipFile(path, "w") as archive:
                    if kind == "traversal":
                        archive.writestr("../escaped", "data")
                    elif kind == "link":
                        member = zipfile.ZipInfo("link")
                        member.create_system = 3
                        member.external_attr = (stat.S_IFLNK | 0o777) << 16
                        archive.writestr(member, "/tmp")
                    else:
                        archive.writestr("model.glb", "a")
                        archive.writestr("model.glb", "b")
                with self.assertRaises((ValueError, FileExistsError)):
                    extract(path)
                self.assertFalse((Path(root).parent / "escaped").exists())

    def test_extracts_data_without_executable_permissions(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "source.zip"
            with zipfile.ZipFile(path, "w") as archive:
                member = zipfile.ZipInfo("source/model.glb")
                member.external_attr = (stat.S_IFREG | 0o755) << 16
                archive.writestr(member, b"only data")
            destination = extract(path)
            try:
                output = destination / "source/model.glb"
                self.assertEqual(output.read_bytes(), b"only data")
                self.assertEqual(output.stat().st_mode & 0o111, 0)
            finally:
                shutil.rmtree(destination)

    def test_wall_height_gate_keeps_kerb_lips_drivable(self):
        kerb = np.array([[0, 0, 0], [2, 0, 0], [2, 0.2, 0]])
        wall = np.array([[0, 0, 1], [2, 0, 1], [2, 1, 1]])
        result = walls_from(
            np.array([kerb, wall, wall[::-1]]), np.array([[0, 0, 1], [2, 0, 1]])
        )
        np.testing.assert_allclose(result, [[0.4, 1, 2, 1]])

    def test_glb_roundtrip_caps_cutout_and_keeps_ground_contract(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "visual.glb"
            positions = np.array([[0, 0, 0], [0, 0, 3], [4, 0, 0]], dtype="f4")
            mesh = GlbMesh(
                "ground:road@0_0",
                0,
                positions,
                np.tile([0, 1, 0], (3, 1)),
                np.zeros((3, 2)),
                np.array([0, 1, 2]),
            )
            material = GlbMaterial(
                "ground:road", image=Image.new("RGBA", (128, 64)), alpha_mask=True
            )
            write_glb(str(path), [material], [mesh], 32)
            model = Glb(path)
            entry = model.json["materials"][0]
            self.assertEqual(entry["alphaMode"], "MASK")
            self.assertEqual(model.image(entry).size, (32, 16))
            _, actual, _, _, faces = next(model.primitives())
            np.testing.assert_allclose(actual[faces].reshape(-1, 3), positions)
            self.assertTrue(model.json["meshes"][0]["name"].startswith("ground:"))
            raw = path.read_bytes()
            length = struct.unpack_from("<I", raw, 12)[0]
            self.assertEqual(
                json.loads(raw[20 : 20 + length])["asset"]["version"], "2.0"
            )


if __name__ == "__main__":
    unittest.main()
