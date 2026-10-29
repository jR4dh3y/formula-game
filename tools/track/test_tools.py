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
from centerline import extract as extract_centerline
from convert_glb import (bridge_road_seams, convert_material, remove_placeholders,
                         walls_from, widths_inside_walls)
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

    def test_centerline_extracts_closed_road_without_painted_edges(self):
        angle = np.arange(128) * (2 * np.pi / 128)
        inner = np.column_stack([45 * np.cos(angle), np.zeros(128), 45 * np.sin(angle)])
        outer = np.column_stack([55 * np.cos(angle), np.zeros(128), 55 * np.sin(angle)])
        triangles = []
        for i in range(128):
            j = (i + 1) % 128
            triangles.extend(
                [[inner[i], outer[i], inner[j]], [outer[i], outer[j], inner[j]]]
            )
        with tempfile.TemporaryDirectory() as root:
            points, widths = extract_centerline(
                np.array(triangles),
                {"length": 2 * np.pi * 50, "start": [50, 0], "direction": [0, 1]},
                Path(root) / "debug.png",
            )
            self.assertTrue(np.isfinite(points).all())
            self.assertTrue((widths >= 3).all())
            self.assertLess(
                abs(
                    np.linalg.norm(np.roll(points, -1, axis=0) - points, axis=1).sum()
                    / (2 * np.pi * 50)
                    - 1
                ),
                0.02,
            )

    def test_grid_cube_removal_preserves_surrounding_ground(self):
        dimensions = np.array([1.416, 1.029, 1.407])
        cube = (
            np.array([[x, y, z] for x in [0, 1] for y in [0, 1] for z in [0, 1]])
            * dimensions
        )
        positions = np.vstack([cube, [[10, 0, 0], [12, 0, 0], [10, 0, 2]]])
        faces = np.array(
            [
                [0, 1, 3],
                [0, 3, 2],
                [4, 6, 7],
                [4, 7, 5],
                [0, 4, 5],
                [0, 5, 1],
                [2, 3, 7],
                [2, 7, 6],
                [0, 2, 6],
                [0, 6, 4],
                [1, 5, 7],
                [1, 7, 3],
                [8, 9, 10],
            ]
        )
        rules = [{"material": "grassc", "dimensions": dimensions.tolist()}]
        actual = remove_placeholders(positions, faces, "grassc", rules)
        np.testing.assert_array_equal(actual, [[8, 9, 10]])
        np.testing.assert_array_equal(
            remove_placeholders(positions, faces, "road", rules), faces
        )

    def test_wall_height_gate_keeps_kerb_lips_drivable(self):
        kerb = np.array([[0, 0, 0], [2, 0, 0], [2, 0.2, 0]])
        wall = np.array([[0, 0, 1], [2, 0, 1], [2, 1, 1]])
        result = walls_from(
            np.array([kerb, wall, wall[::-1]]), np.array([[0, 0, 1], [2, 0, 1]])
        )
        np.testing.assert_allclose(result, [[0.4, 1, 2, 1]])

    def test_subdivided_guardrail_and_widths_respect_physical_barrier(self):
        panel = np.array([[[0, 0.3, 3], [10, 0.3, 3], [10, 0.59, 3]]])
        points = np.array([[0, 0, 0], [10, 0, 0]])
        self.assertEqual(len(walls_from(panel, points)), 0)
        self.assertEqual(len(walls_from(panel, points, 0.2, 0.45)), 1)
        loop = np.array([[0, 0, 0], [0, 0, 10], [10, 0, 10], [10, 0, 0]])
        widths = np.full(4, 6.)
        wall = np.array([[3, -10, 3, 20]])
        limited = widths_inside_walls(loop, widths, wall)
        self.assertTrue((limited < widths).any())
        self.assertTrue((limited >= 1.5).all())
        np.testing.assert_allclose(widths_inside_walls(loop, widths, np.empty((0, 4))), widths)

    def test_seam_underpinning_only_joins_audited_matching_edges(self):
        road = np.array([
            [[0, 0, 0], [0, 0, 4], [2, 0, 0]],
            [[0.4, 0, 0], [0.4, 0, 4], [-2, 0, 0]],
        ])
        rule = {"bounds": [[-0.1, -0.1], [0.5, 4.1]], "max_gap": 0.5}
        patches = bridge_road_seams(road, [rule])
        self.assertEqual(len(patches), 2)
        self.assertTrue((patches[:, :, 1] == 0).all())
        self.assertEqual(len(bridge_road_seams(road, [{**rule, "max_gap": 0.2}])), 0)
        road[1, :, 1] = 0.1
        self.assertEqual(len(bridge_road_seams(road, [rule])), 0)

    def test_explicit_alpha_repairs_do_not_punch_holes_in_road(self):
        class Model:
            def image(self, material, cap):
                return Image.new("RGBA", (2, 2), (50, 60, 70, 20))

        config = {key: [] for key in ("asphalt", "kerb", "grass", "gravel", "concrete")}
        config["asphalt"] = ["road"]
        config["kerb"] = ["kerb"]
        config["material_alpha"] = {
            "MASK": ["fence"], "BLEND": ["rubber"], "OPAQUE": ["kerb"]
        }
        config["decals"] = ["rubber"]
        materials = [convert_material(Model(), m, config) for m in [
            {"name": "road"}, {"name": "fence"}, {"name": "rubber"},
            {"name": "kerb", "alphaMode": "BLEND"},
        ]]
        road, fence, rubber, kerb = materials
        self.assertEqual(road.image.mode, "RGB")
        self.assertEqual(kerb.image.mode, "RGB")
        self.assertFalse(kerb.alpha_mask or kerb.alpha_blend)
        self.assertEqual(fence.image.mode, "RGBA")
        self.assertTrue(fence.alpha_mask)
        self.assertEqual(rubber.name, "decal:rubber")
        self.assertTrue(rubber.alpha_blend)
        self.assertFalse(rubber.alpha_mask)
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "materials.glb"
            write_glb(str(path), materials, [])
            actual = Glb(path).json["materials"]
            self.assertEqual([m.get("alphaMode", "OPAQUE") for m in actual],
                             ["OPAQUE", "MASK", "BLEND", "OPAQUE"])
            self.assertNotIn("alphaCutoff", actual[2])
        config["material_alpha"]["BLEND"].append("fence")
        with self.assertRaisesRegex(ValueError, "conflicting alpha"):
            convert_material(Model(), {"name": "fence"}, config)

    def test_bahrain_runoff_is_visible_and_scenery_uses_cutout_depth(self):
        config = json.loads((Path(__file__).parent / "tracks/bahrain.json").read_text())

        class Model:
            def image(self, material, cap):
                return Image.new("RGBA", (2, 2), (130, 125, 110, 255))

        # Reproduce the upstream gravel error: an opaque texture multiplied by
        # zero alpha was discarded in MASK mode, hiding a million square metres.
        gravel = convert_material(Model(), {
            "name": "Bahrainv3001791Mtl_7", "alphaMode": "MASK",
            "pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 0]},
        }, config)
        fence = convert_material(Model(), {
            "name": "Bahrainv3001791Mtl_1", "alphaMode": "BLEND",
        }, config)
        glass = convert_material(Model(), {
            "name": "Bahrainv3001791Mtl_97", "alphaMode": "BLEND",
        }, config)
        asphalt = convert_material(Model(), {
            "name": "Bahrainv3001791Mtl_73",
            "pbrMetallicRoughness": {"metallicFactor": 0.5, "roughnessFactor": 0.3},
        }, config)
        self.assertEqual(asphalt.metallic, 0)
        self.assertGreaterEqual(asphalt.roughness, 0.85)
        self.assertFalse(gravel.alpha_mask or gravel.alpha_blend)
        self.assertEqual(gravel.image.mode, "RGB")
        self.assertTrue(fence.alpha_mask)
        self.assertFalse(fence.alpha_blend)
        self.assertTrue(glass.alpha_blend)
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / "materials.glb"
            write_glb(str(output), [gravel, fence, glass], [])
            materials = Glb(output).json["materials"]
            self.assertEqual([m.get("alphaMode", "OPAQUE") for m in materials],
                             ["OPAQUE", "MASK", "BLEND"])

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
