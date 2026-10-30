"""Checks tools/car/teams.json against the gen22 team and livery contract.

Run with: python3 -I tools/car/test_teams.py
"""

import json
import re
import unittest
from pathlib import Path

TEAMS_PATH = Path(__file__).resolve().parent / "teams.json"

# 2022 constructors' order, with each team's race drivers as (abbr, number).
EXPECTED_GRID = {
    "redbull": [("VER", 1), ("PER", 11)],
    "ferrari": [("LEC", 16), ("SAI", 55)],
    "mercedes": [("HAM", 44), ("RUS", 63)],
    "alpine": [("OCO", 31), ("ALO", 14)],
    "mclaren": [("NOR", 4), ("RIC", 3)],
    "alfaromeo": [("BOT", 77), ("ZHO", 24)],
    "astonmartin": [("VET", 5), ("STR", 18)],
    "haas": [("MAG", 20), ("MSC", 47)],
    "alphatauri": [("GAS", 10), ("TSU", 22)],
    "williams": [("ALB", 23), ("LAT", 6)],
}

HEX_RE = re.compile(r"^#[0-9A-Fa-f]{6}$")
ID_RE = re.compile(r"^[a-z]+$")
ABBR_RE = re.compile(r"^[A-Z]{3}$")
ROLES = {"primary", "secondary", "accent", "carbon"}
LIVERY_STYLES = {"solid", "split", "fade", "stripe", "band"}
COMMON_LIVERY_KEYS = {"nose_tip", "wing_endplates", "halo", "text_color"}
HELMET_STYLES = {"stripe", "halves", "visor_band", "chevron"}
SHORT_NAME_MAX = 12


class TeamsContractTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads(TEAMS_PATH.read_text(encoding="utf-8"))
        cls.teams = cls.data["teams"]
        cls.styles_doc = cls.data["livery_styles"]["styles"]
        cls.common_doc = cls.data["livery_styles"]["common"]
        cls.roles_doc = cls.data["livery_styles"]["roles"]

    def test_season_and_grid_order(self):
        self.assertEqual(self.data["season"], 2022)
        self.assertEqual(len(self.teams), 10)
        self.assertEqual([t["id"] for t in self.teams], list(EXPECTED_GRID))

    def test_team_names(self):
        for team in self.teams:
            with self.subTest(team=team["id"]):
                self.assertIsInstance(team["name"], str)
                self.assertTrue(team["name"].strip())
                self.assertIsInstance(team["short"], str)
                self.assertTrue(team["short"].strip())
                self.assertLessEqual(len(team["short"]), SHORT_NAME_MAX)

    def test_drivers_match_2022_grid(self):
        self.assertEqual(sum(len(t["drivers"]) for t in self.teams), 20)
        for team in self.teams:
            with self.subTest(team=team["id"]):
                got = [(d["abbr"], d["number"]) for d in team["drivers"]]
                self.assertEqual(got, EXPECTED_GRID[team["id"]])

    def test_ids_and_numbers_unique(self):
        team_ids = [t["id"] for t in self.teams]
        driver_ids = [d["id"] for t in self.teams for d in t["drivers"]]
        numbers = [d["number"] for t in self.teams for d in t["drivers"]]
        abbrs = [d["abbr"] for t in self.teams for d in t["drivers"]]
        self.assertEqual(len(set(team_ids)), len(team_ids))
        self.assertEqual(len(set(driver_ids)), len(driver_ids))
        self.assertEqual(len(set(numbers)), len(numbers))
        self.assertEqual(len(set(abbrs)), len(abbrs))
        for team_id in team_ids:
            self.assertRegex(team_id, ID_RE)
        for driver_id in driver_ids:
            self.assertRegex(driver_id, ID_RE)
        for abbr in abbrs:
            self.assertRegex(abbr, ABBR_RE)

    def test_colours(self):
        for team in self.teams:
            with self.subTest(team=team["id"]):
                self.assertEqual(set(team["colors"]), ROLES)
                for role, value in team["colors"].items():
                    self.assertRegex(value, HEX_RE, msg=role)
                self.assertEqual(set(team["suit"]), {"primary", "secondary", "accent"})
                for value in team["suit"].values():
                    self.assertRegex(value, HEX_RE)
                for driver in team["drivers"]:
                    helmet = driver["helmet"]
                    self.assertEqual(set(helmet), {"primary", "secondary", "accent", "style"})
                    for key in ("primary", "secondary", "accent"):
                        self.assertRegex(helmet[key], HEX_RE)
                    self.assertIn(helmet["style"], HELMET_STYLES)
                    self.assertEqual(driver["helmets"], ["default"])

        # Avoid pure black and white in PBR surfaces.
        all_colours = [v for t in self.teams for v in t["colors"].values()]
        all_colours += [v for t in self.teams for v in t["suit"].values()]
        all_colours += [
            v for t in self.teams for d in t["drivers"] for k, v in d["helmet"].items() if k != "style"
        ]
        for value in all_colours:
            self.assertNotIn(value.upper(), ("#000000", "#FFFFFF"))

    def test_livery_styles_documented(self):
        self.assertEqual(set(self.roles_doc["values"]), ROLES)
        self.assertEqual(set(self.styles_doc), LIVERY_STYLES)
        for name, style in self.styles_doc.items():
            with self.subTest(style=name):
                self.assertTrue(style["description"].strip())
                for pname, spec in style["params"].items():
                    with self.subTest(param=pname):
                        self.assertTrue(spec["description"].strip())
                        self.assertIn(spec["type"], ("number", "role"))
                        if spec["type"] == "number":
                            self.assertLess(spec["min"], spec["max"])
        self.assertEqual(set(self.common_doc), COMMON_LIVERY_KEYS)
        for key, spec in self.common_doc.items():
            with self.subTest(common=key):
                self.assertTrue(spec["description"].strip())
                self.assertIn(spec["type"], ("role", "role_or_null"))

    def test_livery_matches_documented_schema(self):
        for team in self.teams:
            livery = team["livery"]
            style = livery["style"]
            with self.subTest(team=team["id"], style=style):
                self.assertIn(style, self.styles_doc)
                params = self.styles_doc[style]["params"]
                # Every documented key is required; no implicit defaults.
                self.assertEqual(set(livery), {"style"} | set(params) | COMMON_LIVERY_KEYS)
                for pname, spec in params.items():
                    self.assertTrue(self._value_ok(spec, livery[pname]), msg=f"{pname}={livery[pname]!r}")
                for key, spec in self.common_doc.items():
                    self.assertTrue(self._value_ok(spec, livery[key]), msg=f"{key}={livery[key]!r}")
                if style == "fade":
                    self.assertLess(livery["fade_z0"], livery["fade_z1"])

    def _value_ok(self, spec, value):
        kind = spec["type"]
        if kind == "number":
            return (
                isinstance(value, (int, float))
                and not isinstance(value, bool)
                and spec["min"] <= value <= spec["max"]
            )
        if kind == "role":
            return value in ROLES
        if kind == "role_or_null":
            return value is None or value in ROLES
        return False


if __name__ == "__main__":
    unittest.main()
