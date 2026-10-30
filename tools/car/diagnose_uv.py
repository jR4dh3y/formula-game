"""Draw original and authored charts from the actual mesh, with fold diagnostics."""

import argparse
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw, ImageFont


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bake", type=Path)
    parser.add_argument("--template", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    report = {}
    profile = json.loads(Path(__file__).with_name("monoposto-modern.json").read_text())
    font = ImageFont.truetype("/usr/share/fonts/TTF/DejaVuSans.ttf", 22)
    for path in sorted(args.bake.glob("*-charts.npz")):
        data = np.load(path)
        original = data["original"]
        mapped = data["mapped"]
        valid = np.all(mapped >= 0, axis=(1, 2))

        def determinant(t):
            a = t[:, 1] - t[:, 0]
            b = t[:, 2] - t[:, 0]
            return a[:, 0] * b[:, 1] - a[:, 1] * b[:, 0]

        a, b = determinant(original[valid]), determinant(mapped[valid])
        nonzero = (np.abs(a) > 1e-10) & (np.abs(b) > 1e-10)
        ratios = np.sign(a[nonzero] * b[nonzero])
        report[path.stem] = {
            "triangles": len(original),
            "mapped": int(valid.sum()),
            "collapsed": int((np.abs(b) < 1e-10).sum()),
            "positive_jacobian": int((ratios > 0).sum()),
            "negative_jacobian": int((ratios < 0).sum()),
        }
        for label, tri in [("original", original), ("mapped", mapped[valid])]:
            im = (
                Image.new("RGB", (2048, 2048), (20, 20, 20))
                if label == "original"
                else Image.open(args.template).convert("RGB")
            )
            d = ImageDraw.Draw(im)
            for t in tri:
                pts = [(float(u) * 2048, (1 - float(v)) * 2048) for u, v in t]
                d.line(pts + [pts[0]], fill=(40, 160, 220), width=1)
            for number, spec in enumerate(
                profile["objects"].get(path.stem.replace("-charts", ""), []), 1
            ):
                if label == "original":
                    centre = np.array(spec["bounds"]).mean(axis=0)
                    xy = (centre[0] * 2048, (1 - centre[1]) * 2048)
                else:
                    part = spec.get("parts", [spec])[0]
                    r = np.array(part["rows"])
                    xy = (r[:, [3, 5]].mean() * 2048, r[:, [4, 6]].mean() * 2048)
                d.text(
                    xy,
                    str(number) + ": " + spec["panel"],
                    font=font,
                    fill="red",
                    stroke_width=2,
                    stroke_fill="black",
                )
            im.save(args.output / (path.stem + "-" + label + ".png"))
    (args.output / "uv-quality.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
