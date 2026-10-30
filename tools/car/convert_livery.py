"""Bake official Monoposto 2026 sheets using authored mesh UV correspondence."""

import argparse
import hashlib
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_PROFILE = Path(__file__).with_name("monoposto-modern.json")


def sample_rgba(image, uv):
    """Bilinear sampling in top-down PNG coordinates, preserving alpha."""
    pixels = np.asarray(image.convert("RGBA"))
    h, w = pixels.shape[:2]
    xy = uv * np.array([w - 1, h - 1], dtype=np.float32)
    low = np.floor(xy).astype(np.int32)
    high = np.minimum(low + 1, [w - 1, h - 1])
    t = (xy - low).astype(np.float32)
    top = (
        pixels[low[:, 1], low[:, 0]] * (1 - t[:, :1])
        + pixels[low[:, 1], high[:, 0]] * t[:, :1]
    )
    bottom = (
        pixels[high[:, 1], low[:, 0]] * (1 - t[:, :1])
        + pixels[high[:, 1], high[:, 0]] * t[:, :1]
    )
    return top * (1 - t[:, 1:]) + bottom * t[:, 1:]


def dilate(rgb, mask, iterations=12):
    # Keep this small helper independent of the procedural livery generator.
    result, filled = rgb.copy(), mask.astype(bool).copy()
    for _ in range(iterations):
        previous = filled.copy()
        for dy, dx in ((0, 1), (0, -1), (1, 0), (-1, 0)):
            sy, sx = (
                slice(max(0, -dy), min(mask.shape[0], mask.shape[0] - dy)),
                slice(max(0, -dx), min(mask.shape[1], mask.shape[1] - dx)),
            )
            ty, tx = (
                slice(max(0, dy), min(mask.shape[0], mask.shape[0] + dy)),
                slice(max(0, dx), min(mask.shape[1], mask.shape[1] + dx)),
            )
            use = ~filled[ty, tx] & previous[sy, sx]
            result[ty, tx][use] = result[sy, sx][use]
            filled[ty, tx][use] = True
    return result


class Converter:
    def __init__(self, bake, profile=DEFAULT_PROFILE):
        self.profile = json.loads(Path(profile).read_text())
        if self.profile.get("version") != 2:
            raise ValueError("Unsupported mesh correspondence profile")
        with np.load(bake, allow_pickle=False) as archive:
            if "source_uv" not in archive or "correspondence_sha256" not in archive:
                raise ValueError(
                    "Rebuild Gen22: paint bake lacks mesh source UV correspondence"
                )
            expected = hashlib.sha256(Path(profile).read_bytes()).hexdigest()
            if str(archive["correspondence_sha256"]) != expected:
                raise ValueError(
                    "Source atlas correspondence differs from bake; rebuild Gen22"
                )
            self.mask = archive["mask"]
            source_uv = archive["source_uv"]
        if source_uv.shape != (*self.mask.shape, 2):
            raise ValueError("Invalid mesh source UV bake shape")
        self.indices = np.flatnonzero(self.mask)
        self.uv = source_uv.reshape(-1, 2)[self.indices]
        self.painted = np.all(self.uv >= 0, axis=1)
        if not np.all(np.isfinite(self.uv)) or np.any(self.uv > 1):
            raise ValueError("Nonfinite or out of bounds mesh source UV")
        self.uv = np.clip(self.uv, 0, 1)
        self.profile_fingerprint = expected
        self.fingerprint = hashlib.sha256(Path(bake).read_bytes()).hexdigest()

    def convert(self, source, output):
        if Path(source).resolve() == Path(output).resolve():
            raise ValueError("Output must not overwrite the source sheet")
        expected = self.profile["source_size"]
        with Image.open(source) as image:
            if image.format != "PNG" or list(image.size) != expected:
                raise ValueError(
                    f"Expected PNG with {expected[0]}:{expected[1]} layout, exactly 2048 pixels per side"
                )
            sampled = sample_rgba(image, self.uv)
        carbon = np.asarray(self.profile["carbon"], dtype=np.float32)
        alpha = sampled[:, 3:4] / 255
        rgb = sampled[:, :3] * alpha + carbon * (1 - alpha)
        rgb[~self.painted] = carbon
        texture = np.broadcast_to(carbon, (*self.mask.shape, 3)).copy()
        texture.reshape(-1, 3)[self.indices] = rgb
        texture = np.clip(dilate(texture, self.mask), 0, 255).astype(np.uint8)
        output = Path(output)
        output.parent.mkdir(parents=True, exist_ok=True)
        Image.fromarray(texture).save(output)
        report = {
            "source": str(Path(source).resolve()),
            "profile": self.profile["name"],
            "method": "authored FP04RB chart correspondence, barycentrically rasterized",
            "paint_bake_sha256": self.fingerprint,
            "mapped_texels": int(self.painted.sum()),
            "source_sha256": hashlib.sha256(Path(source).read_bytes()).hexdigest(),
            "correspondence_sha256": self.profile_fingerprint,
            "limitations": [
                "Only painted car panels are converted; suit, helmet, tyres and bare-carbon floor and suspension keep their own materials.",
                "Different sheet layouts need a separate profile; panel proportions differ between the two models.",
            ],
        }
        output.with_name(output.name + ".json").write_text(
            json.dumps(report, indent=2) + "\n"
        )
        return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path, help="One PNG or a directory of PNG sheets")
    parser.add_argument(
        "--output",
        type=Path,
        required=True,
        help="Output PNG for one input, or directory for a batch",
    )
    parser.add_argument("--bake", type=Path, default=ROOT / "raw/gen22/bake/paint.npz")
    parser.add_argument("--profile", type=Path, default=DEFAULT_PROFILE)
    parser.add_argument("--recursive", action="store_true")
    args = parser.parse_args()
    try:
        converter = Converter(args.bake, args.profile)
        if args.input.is_dir():
            if args.output.resolve().is_relative_to(args.input.resolve()):
                parser.error("Batch output must be outside the source directory")
            files = sorted(
                p
                for p in (
                    args.input.rglob("*") if args.recursive else args.input.iterdir()
                )
                if p.is_file() and p.suffix.lower() == ".png"
            )
            if not files:
                parser.error("No PNG files found")
            jobs = [(p, args.output / p.relative_to(args.input)) for p in files]
        else:
            jobs = [(args.input, args.output)]
        if any(source.resolve() == output.resolve() for source, output in jobs):
            parser.error("Output must not overwrite source sheets")
        failures = []
        completed = []
        for source, output in jobs:
            try:
                converter.convert(source, output)
                completed.append({"source": str(source), "output": str(output)})
                print("CONVERTED", source, "->", output, flush=True)
            except (OSError, ValueError, Image.DecompressionBombError) as error:
                failures.append({"source": str(source), "error": str(error)})
                print("FAILED", source, error, file=sys.stderr, flush=True)
        if args.input.is_dir():
            args.output.mkdir(parents=True, exist_ok=True)
            (args.output / "conversion-report.json").write_text(
                json.dumps(
                    {
                        "profile": converter.profile["name"],
                        "converted": completed,
                        "failed": failures,
                    },
                    indent=2,
                )
                + "\n"
            )
        if failures:
            parser.exit(1, json.dumps(failures, indent=2) + "\n")
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"Livery conversion failed: {error}\n")


if __name__ == "__main__":
    main()
