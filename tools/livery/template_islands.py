#!/usr/bin/env python3
"""Find UV islands in the Gen22 2026 template.

Pipeline:
  1. inside mask = luminance >= THRESH and alpha >= ALPHA_MIN
  2. morphological closing (dilate N times, erode N times, 3x3 structuring
     element) so thin dark wireframe lines inside islands are filled
  3. 4-connected labelling, drop components smaller than MINAREA
  4. relabel survivors 1..N ordered by centroid (row, then column)
  5. write islands.npy, islands.json, islands_overlay.png,
     islands_overlay_1024.png into --out

Usage: python3 -I tools/livery/template_islands.py [--template PATH] [--out DIR]
                                                  [--thresh 80] [--close 2] [--minarea 150]
"""

import argparse
import colorsys
import json
import os
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy import ndimage as ndi

REPO = Path(__file__).resolve().parents[2]
DEFAULT_TEMPLATE = REPO / "raw/gen22/mapping-reference/2026-template.png"
DEFAULT_OUT = REPO / "raw/gen22/livery-mapping"
FONT_PATH = "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf"


def parse_args():
    ap = argparse.ArgumentParser()
    ap.add_argument("--template", type=Path, default=DEFAULT_TEMPLATE,
                    help="2026 template PNG (2048x2048)")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT,
                    help="output directory for islands.npy/json and overlays")
    ap.add_argument("--thresh", type=float, default=80.0,
                    help="luminance threshold for 'inside' (0-255)")
    ap.add_argument("--alpha", type=int, default=1,
                    help="minimum alpha for inside (islands are alpha 64, background alpha 0)")
    ap.add_argument("--close", type=int, default=2,
                    help="closing iterations (dilate N then erode N, 3x3)")
    ap.add_argument("--minarea", type=int, default=150,
                    help="drop components with fewer pixels than this")
    ap.add_argument('--dark-wireframe', action='store_true',
                    help='opaque white-background templates: fill enclosed dark outlines')
    return ap.parse_args()


def build_mask(rgba, thresh, alpha_min, close_iters):
    rgb = rgba[..., :3].astype(np.float32)
    alpha = rgba[..., 3]
    lum = 0.299 * rgb[..., 0] + 0.587 * rgb[..., 1] + 0.114 * rgb[..., 2]
    inside = (lum >= thresh) & (alpha >= alpha_min)

    if close_iters > 0:
        st = np.ones((3, 3), dtype=bool)
        dil = ndi.binary_dilation(inside, structure=st, iterations=close_iters)
        # border_value=1 so islands touching the image edge are not eroded away
        closed = ndi.binary_erosion(dil, structure=st, iterations=close_iters,
                                    border_value=1)
        # never fill true background (alpha 0): the slits between neighbouring
        # islands are alpha-0 gaps and must not be bridged by the closing
        closed &= alpha >= alpha_min
    else:
        closed = inside
    return inside, closed


def label_and_filter(mask, minarea):
    # default structure = 4-connectivity, so diagonal touches do not merge islands
    lab, n_raw = ndi.label(mask)
    if n_raw == 0:
        return np.zeros(mask.shape, dtype=np.int16), [], n_raw

    idx = np.arange(1, n_raw + 1)
    areas = ndi.sum(np.ones_like(lab), lab, index=idx).astype(np.int64)
    coms = ndi.center_of_mass(mask, lab, index=idx)  # (row, col) per label
    slices = ndi.find_objects(lab)

    keep = [i for i in range(n_raw) if areas[i] >= minarea]
    # stable order: centroid row, then centroid column
    keep.sort(key=lambda i: (coms[i][0], coms[i][1]))

    out = np.zeros(mask.shape, dtype=np.int16)
    comps = []
    for new_id, i in enumerate(keep, start=1):
        old_id = i + 1
        sl = slices[i]
        region = lab[sl] == old_id
        out[sl][region] = new_id
        y0, y1 = sl[0].start, sl[0].stop - 1
        x0, x1 = sl[1].start, sl[1].stop - 1
        cy, cx = coms[i]
        comps.append({
            "id": new_id,
            "area": int(areas[i]),
            "bbox": [int(x0), int(y0), int(x1), int(y1)],
            "centroid": [round(float(cx), 2), round(float(cy), 2)],
        })
    return out, comps, n_raw


def label_point(label_img, comp):
    """Point to draw the id at: the centroid if it lies on the island,
    otherwise the interior point farthest from the island edge."""
    cx, cy = comp["centroid"]
    px, py = int(round(cx)), int(round(cy))
    if label_img[py, px] == comp["id"]:
        return cx, cy
    x0, y0, x1, y1 = comp["bbox"]
    crop = label_img[y0:y1 + 1, x0:x1 + 1] == comp["id"]
    dist = ndi.distance_transform_edt(crop)
    r, c = np.unravel_index(int(np.argmax(dist)), dist.shape)
    return float(x0 + c), float(y0 + r)


def make_overlay(rgba, label_img, comps):
    base = rgba[..., :3].astype(np.float32)
    n = len(comps)
    # distinct, well-spread hues, golden-ratio spacing
    colours = np.zeros((n + 1, 3), dtype=np.float32)
    h = 0.0
    for i in range(1, n + 1):
        h = (h + 0.618033988749895) % 1.0
        r, g, b = colorsys.hsv_to_rgb(h, 0.85, 1.0)
        colours[i] = (r * 255, g * 255, b * 255)

    out = base.copy()
    m = label_img > 0
    out[m] = 0.4 * base[m] + 0.6 * colours[label_img[m]]
    img = Image.fromarray(np.clip(out, 0, 255).astype(np.uint8), "RGB")

    draw = ImageDraw.Draw(img)
    try:
        font = ImageFont.truetype(FONT_PATH, 28)
    except OSError:
        font = ImageFont.load_default(size=28)
    for comp in comps:
        x, y = label_point(label_img, comp)
        draw.text((x, y), str(comp["id"]), font=font, anchor="mm",
                  fill=(0, 0, 0), stroke_width=3, stroke_fill=(255, 255, 255))
    return img


def main():
    args = parse_args()
    os.makedirs(args.out, exist_ok=True)

    rgba = np.asarray(Image.open(args.template).convert("RGBA"))
    assert rgba.shape[:2] == (2048, 2048), rgba.shape

    if args.dark_wireframe:
        inside = rgba[..., :3].min(axis=2) < args.thresh
        closed = ndi.binary_fill_holes(inside)
    else:
        inside, closed = build_mask(rgba, args.thresh, args.alpha, args.close)
    label_img, comps, n_raw = label_and_filter(closed, args.minarea)

    np.save(os.path.join(args.out, "islands.npy"), label_img)
    with open(os.path.join(args.out, "islands.json"), "w") as f:
        json.dump(comps, f, indent=1)

    overlay = make_overlay(rgba, label_img, comps)
    overlay.save(os.path.join(args.out, "islands_overlay.png"))
    overlay.resize((1024, 1024), Image.LANCZOS).save(
        os.path.join(args.out, "islands_overlay_1024.png"))

    areas = [c["area"] for c in comps]
    print(f"params: thresh={args.thresh} alpha>={args.alpha} "
          f"close={args.close} minarea={args.minarea}")
    print(f"inside px={int(inside.sum())} closed px={int(closed.sum())}")
    print(f"raw components={n_raw}  kept={len(comps)}  "
          f"dropped={n_raw - len(comps)}")
    if areas:
        print(f"area min={min(areas)} median={int(np.median(areas))} max={max(areas)}")
    for c in comps:
        print(f"  id={c['id']:3d} area={c['area']:7d} bbox={c['bbox']} "
              f"centroid={c['centroid']}")


if __name__ == "__main__":
    main()
