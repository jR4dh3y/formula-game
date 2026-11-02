"""Four front-wing elements retain separate template ribbons."""

from pathlib import Path
import unittest

import numpy as np

import author_template_uv as author
import template_2025

ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(
    (ROOT / "raw/gen22/livery-mapping/2025/islands.npy").is_file(),
    "segment the 2025 reference template first",
)
class Template2025Test(unittest.TestCase):
    def test_four_front_flaps_sample_distinct_ribbons_without_gutters(self):
        template = author.Template("2025")
        span = np.repeat(np.linspace(0.05, 0.38, 200), 5)
        chord = np.tile(np.linspace(0.05, 0.95, 5), 200)
        mapped = [
            template_2025.wing_ribbon(template, span, chord, rank) for rank in range(4)
        ]
        for xy in mapped:
            ix = np.rint(xy).astype(int)
            self.assertTrue((template.labels[ix[:, 1], ix[:, 0]] == 19).all())
        for a, b in zip(mapped, mapped[1:]):
            self.assertTrue((a[:, 1] < b[:, 1]).all())
