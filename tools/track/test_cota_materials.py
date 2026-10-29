"""Explicit diffuse-image repairs for the COTA legacy export."""
import copy
import json
from pathlib import Path
import unittest
from PIL import Image
from convert_glb import convert_material


class CotaMaterialTests(unittest.TestCase):
    def test_explicit_source_texture_mapping_preserves_other_material_properties(self):
        class Model:
            json = {
                "images": [{"name": "grey-asphalt"}, {"name": "red-paint"}],
                "textures": [{"source": 0}, {"source": 1}],
            }

            def image(self, material, cap):
                index = material["pbrMetallicRoughness"]["baseColorTexture"]["index"]
                return Image.new("RGBA", (2, 2), [(90, 90, 90, 255), (210, 30, 20, 255)][index])

        config = {key: [] for key in ("asphalt", "kerb", "grass", "gravel", "concrete")}
        config["material_base_color_texture"] = {"paint*": "red-paint"}
        config["material_alpha"] = {"MASK": ["paint"]}
        config["material_pbr"] = {"paint": {"roughnessFactor": 0.92}}
        source = {"name": "paint", "alphaCutoff": 0.3, "pbrMetallicRoughness": {
            "baseColorTexture": {"index": 0}, "baseColorFactor": [0.8, 0.9, 1, 1],
            "metallicFactor": 0.1, "roughnessFactor": 0.5}}
        original = copy.deepcopy(source)
        paint = convert_material(Model(), source, config)
        self.assertEqual(paint.image.getpixel((0, 0)), (210, 30, 20, 255))
        self.assertEqual(paint.base_color, (0.8, 0.9, 1, 1))
        self.assertEqual((paint.metallic, paint.roughness), (0.1, 0.92))
        self.assertTrue(paint.alpha_mask)
        self.assertEqual(paint.alpha_cutoff, 0.3)
        self.assertEqual(source, original)
        road = convert_material(Model(), {**source, "name": "road"}, config)
        self.assertEqual(road.image.getpixel((0, 0)), (90, 90, 90))
        self.assertIsNone(convert_material(Model(), source, config, True).image)
        config["material_base_color_texture"]["paint*"] = "missing-image"
        with self.assertRaisesRegex(ValueError, "expected one source image"):
            convert_material(Model(), source, config)
        config["material_base_color_texture"]["paint*"] = "red-paint"
        duplicate = Model()
        duplicate.json = {**Model.json, "images": [{"name": "red-paint"}, {"name": "red-paint"}]}
        with self.assertRaisesRegex(ValueError, "expected one source image"):
            convert_material(duplicate, source, config)
        unreferenced = Model()
        unreferenced.json = {**Model.json, "textures": [{"source": 0}]}
        with self.assertRaisesRegex(ValueError, "has no texture"):
            convert_material(unreferenced, source, config)
        config["material_base_color_texture"] = {"paint*": "red-paint", "paint": "red-paint"}
        with self.assertRaisesRegex(ValueError, "conflicting base color"):
            convert_material(Model(), source, config)

    def test_cota_names_recover_painted_runoff_without_masking_legacy_ground_alpha(self):
        config = json.loads((Path(__file__).parent / "tracks/cota.json").read_text())
        expected = {"fillred": "roadfillredt-new", "offtracka_blue": "blue_2",
                    "roadlinewhite": "tex_whiteline_a_01_d", "sanda_red": "re_write1"}
        for material, image in expected.items():
            self.assertEqual(config["material_base_color_texture"][material], image)
        masks = config["material_alpha"]["MASK"]
        self.assertNotIn("gravela", masks)
        self.assertNotIn("rmbla", masks)
        self.assertNotIn("roada", masks)
        self.assertIn("roadlinewhite", masks)
        self.assertIn("tower_glass", config["material_alpha"]["BLEND"])


if __name__ == "__main__":
    unittest.main()
