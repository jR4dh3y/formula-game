"""Audited material repairs for the Silverstone Assetto-style export."""
import json
from pathlib import Path
import unittest
from convert_glb import matches


class SilverstoneMaterialTests(unittest.TestCase):
    def setUp(self):
        self.config = json.loads((Path(__file__).parent / "tracks/silverstone.json").read_text())

    def test_detail_maps_are_replaced_by_specular_slot_atlases(self):
        # The exporter wired AC txDetail into base colour and txDiffuse into
        # KHR_materials_specular; the loose archive keeps only the latter's alpha.
        expected = {"barriers1.001": "barriers1", "barriers1.003": "barriers1",
                    "metals1.001": "metals1", "metals1.003": "metals1",
                    "walls2.001": "walls2b", "box1_multimap.001": "box1",
                    "tile1.001": "tile1", "tile1B.001": "tile1B"}
        rules = self.config["material_base_color_texture"]
        for material, image in expected.items():
            selected = [name for pattern, name in rules.items() if matches(material, [pattern])]
            self.assertEqual(selected, [image], material)
        # The gravel trap's detail map is already a plausible gravel albedo.
        self.assertFalse(any(matches("sand_new.001", [p]) for p in rules))
        self.assertFalse(any(matches("box1.001", [p]) for p in rules))

    def test_ground_tints_and_cutouts_never_hide_drivable_or_terrain_surfaces(self):
        alpha = self.config["material_alpha"]
        for name in ("grass.001", "top2.001", "top2_NM.001", "top2_refl.001",
                     "asphalt.001", "sand_new.001", "sand-bordini.001", "jersey.003",
                     "tyreswall.001", "barriers1.003", "box1_multimap.001"):
            for mode in ("MASK", "BLEND"):
                self.assertFalse(matches(name, alpha[mode]), f"{name} in {mode}")
        # Every sampled grass_NM texel is RGB 0 with alpha 0: a cutout, not black grass.
        self.assertTrue(matches("grass_NM.001", alpha["MASK"]))
        pbr = self.config["material_pbr"]
        for name in ("grass.001", "top2.001", "top2_NM.001", "top2_refl.001"):
            rules = [v for p, v in pbr.items() if matches(name, [p])]
            self.assertEqual(len(rules), 1, name)
            factor = rules[0]["baseColorFactor"]
            self.assertEqual(factor[3], 1.0)
            self.assertTrue(all(0.05 < c < 0.5 for c in factor[:3]), name)
            self.assertGreater(factor[1], factor[2], name)


if __name__ == "__main__":
    unittest.main()
