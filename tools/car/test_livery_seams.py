"""Run inside Blender to verify UV seam and smooth-normal preservation."""

import importlib.util
from pathlib import Path
import sys
import unittest


@unittest.skipUnless(importlib.util.find_spec("bpy"), "requires Blender")
class LiverySeamsTest(unittest.TestCase):
    @unittest.skipUnless(
        (
            Path(__file__).resolve().parents[2]
            / "raw/gen22/livery-mapping/2025/islands.npy"
        ).is_file(),
        "requires segmented template",
    )
    def test_panel_ownership_restores_paint_without_changing_detail_uvs(self):
        import bpy
        import numpy as np

        sys.path.insert(0, str(Path(__file__).resolve().parent))
        import livery_seams

        mesh = bpy.data.meshes.new("panel fixture")
        mesh.from_pydata(
            [(0, 0, 0), (1, 0, 0), (2, 0, 0), (0, 1, 0), (1, 1, 0), (2, 1, 0)],
            [],
            [(0, 1, 4, 3), (1, 2, 5, 4)],
        )
        obj = bpy.data.objects.new("panel fixture", mesh)
        mesh.materials.append(bpy.data.materials.new("source_paint_panel"))
        mesh.materials.append(bpy.data.materials.new("source_carbon_panel"))
        mesh.polygons[1].material_index = 1
        original = mesh.uv_layers.new(name="OriginalUV")
        layer = mesh.uv_layers.new(name="livery")
        owner = mesh.attributes.new("livery_2025_panel", "INT", "FACE")
        owner.data[0].value = 1
        for loop in layer.data:
            loop.uv = (20, 20)
        before = [tuple(loop.uv) for loop in original.data]
        livery_seams.constrain(obj)
        labels = np.load(
            Path(__file__).resolve().parents[2]
            / "raw/gen22/livery-mapping/2025/islands.npy"
        )
        for i in mesh.polygons[0].loop_indices:
            u, v = layer.data[i].uv
            self.assertEqual(labels[round((1 - v) * 2048), round(u * 2048)], 1)
        for i in mesh.polygons[1].loop_indices:
            self.assertEqual(tuple(layer.data[i].uv), (20, 20))
        self.assertEqual(before, [tuple(loop.uv) for loop in original.data])
        bpy.data.objects.remove(obj)
        bpy.data.meshes.remove(mesh)

    def test_brake_cover_fins_remain_carbon(self):
        import bpy

        sys.path.insert(0, str(Path(__file__).resolve().parent))
        import source_materials

        source = bpy.data.materials.new("Carbon fixture")
        library = source_materials.Library(Path("/tmp"))
        material = library.for_part("front_inner_cover_fins", source.name, "upright_fr")
        self.assertFalse(material.name.startswith("source_paint_"))
        self.assertIn("Carbon", material.name)

    def test_adjacent_panels_keep_uvs_and_corner_normals_when_separated(self):
        import bpy

        sys.path.insert(0, str(Path(__file__).resolve().parent))
        import livery_seams

        mesh = bpy.data.meshes.new("seam fixture")
        mesh.from_pydata(
            [(0, 0, 0), (1, 0, 0), (2, 0, 0), (0, 1, 0), (1, 1, 0), (2, 1, 0)],
            [],
            [(0, 1, 4, 3), (1, 2, 5, 4)],
        )
        obj = bpy.data.objects.new("seam fixture", mesh)
        mesh.materials.append(bpy.data.materials.new("source_paint_fixture"))
        original = mesh.uv_layers.new(name="OriginalUV")
        livery = mesh.uv_layers.new(name="livery")
        for face in mesh.polygons:
            face.use_smooth = True
            for index in face.loop_indices:
                x, y, _ = mesh.vertices[mesh.loops[index].vertex_index].co
                original.data[index].uv = (x / 2, y)
                livery.data[index].uv = (x / 4 + face.index / 2, y)
        mesh.normals_split_custom_set([(0, 0.1, 1)] * len(mesh.loops))
        normals = [tuple(normal.vector) for normal in mesh.corner_normals]
        coordinates = [
            [tuple(loop.uv) for loop in layer.data] for layer in mesh.uv_layers
        ]
        livery_seams.split(obj)
        self.assertEqual(len(mesh.vertices), 8)
        self.assertTrue(
            set(mesh.polygons[0].vertices).isdisjoint(mesh.polygons[1].vertices)
        )
        self.assertEqual(
            coordinates,
            [[tuple(loop.uv) for loop in layer.data] for layer in mesh.uv_layers],
        )
        for before, after in zip(normals, mesh.corner_normals):
            for a, b in zip(before, after.vector):
                self.assertAlmostEqual(a, b, places=4)
        bpy.data.objects.remove(obj)
        bpy.data.meshes.remove(mesh)


if __name__ == "__main__":
    unittest.main(argv=[__file__])
