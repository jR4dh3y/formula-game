"""Coordinate grid images for reading landmarks off the livery mapping.

Writes two PNGs into --out, both labelled in 2048 texel/template pixel coordinates
(x to the right, y down):
  fp04rb_main_body_uv_grid.png  main_body UV shown at 1024: shaded normals, coloured
                                height bands, outline of each UV island, grid every 128 px.
  template_islands_grid.png     islands.npy colours, id labels, blended 50% with --sheet
                                when given, grid every 64 px.

Usage (from the repo root):
  python3 -I tools/livery/grid_images.py --out DIR [--sheet SHEET.png]
         [--bake main_body.npz] [--islands islands.npy]
"""
import argparse
import colorsys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy import ndimage

REPO = Path(__file__).resolve().parents[2]
DEFAULT_BAKE = REPO / "raw/gen22/livery-mapping/bake2k/main_body.npz"
DEFAULT_ISLANDS = REPO / "raw/gen22/livery-mapping/islands.npy"
BAND_HEIGHT = 0.1          # OBJ units per height band
MIN_ISLAND_TEXELS = 200    # smaller UV islands are not outlined
LIGHT = np.array([0.3, 0.8, 0.5]) / np.linalg.norm([0.3, 0.8, 0.5])


def hue_rgb(i, sat=0.7, val=1.0):
    """Stable bright colour for index i."""
    r, g, b = colorsys.hsv_to_rgb((i * 0.618034) % 1.0, sat, val)
    return np.array([r, g, b]) * 255


def font(size):
    try:
        return ImageFont.load_default(size=size)
    except TypeError:
        return ImageFont.load_default()


def draw_grid(img, step, label_step, scale, size_px):
    """Overlay grid lines every `step` 2048-px, labelled every `label_step`.

    `scale` maps 2048 coordinates to image pixels (image is img.size pixels)."""
    ov = Image.new("RGBA", img.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(ov)
    f = font(size_px)
    for v in range(0, 2048 + 1, step):
        p = v * scale
        major = v % label_step == 0
        alpha = 110 if major else 55
        d.line([(p, 0), (p, img.size[1])], fill=(255, 255, 255, alpha), width=1)
        d.line([(0, p), (img.size[0], p)], fill=(255, 255, 255, alpha), width=1)
        if major:
            d.text((p + 2, 2), str(v), fill=(255, 230, 80, 255), font=f,
                   stroke_width=2, stroke_fill=(0, 0, 0, 255))
            d.text((2, p + 2), str(v), fill=(255, 230, 80, 255), font=f,
                   stroke_width=2, stroke_fill=(0, 0, 0, 255))
    base = img.convert("RGBA")
    return Image.alpha_composite(base, ov).convert("RGB")


def down2(a, mask):
    """2x2 mask-aware average of a (2048,2048,C) array to (1024,1024,C)."""
    m = mask.reshape(1024, 2, 1024, 2).sum((1, 3)).astype(np.float32)
    s = (a * mask[..., None]).reshape(1024, 2, 1024, 2, -1).sum((1, 3))
    return s / np.maximum(m, 1)[..., None], m > 0


def uv_grid(bake_path, out):
    d = np.load(bake_path)
    mask = d["mask"]
    pos, nrm = d["pos"].astype(np.float32), d["nrm"].astype(np.float32)
    P, has = down2(pos, mask)
    N, _ = down2(nrm, mask)
    N /= np.linalg.norm(N, axis=-1, keepdims=True) + 1e-9
    lam = np.clip(N @ LIGHT, 0, 1)
    shade = 0.25 + 0.75 * lam
    ymin = P[..., 1][has].min()
    band = np.floor((P[..., 1] - ymin) / BAND_HEIGHT).astype(int)
    palette = {b: hue_rgb(b, 0.55, 0.95) for b in np.unique(band[has])}
    band_rgb = np.zeros(P.shape, np.float32)
    for b, c in palette.items():
        band_rgb[band == b] = c
    img = np.where(has[..., None], band_rgb * (0.35 + 0.65 * shade[..., None]) * 0.9,
                   np.array([14, 14, 18], np.float32))

    # Outline each UV island (4-connected components of the 2048 mask).
    lab, n = ndimage.label(mask)
    sizes = ndimage.sum(mask, lab, np.arange(1, n + 1))
    keep = np.zeros(n + 1, bool)
    keep[1:] = sizes >= MIN_ISLAND_TEXELS
    lab = np.where(keep[lab], lab, 0)
    edge = np.zeros(mask.shape, bool)
    for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
        nb = np.roll(lab, (dy, dx), axis=(0, 1))
        edge |= (lab > 0) & (nb != lab)
    edge1 = edge.reshape(1024, 2, 1024, 2).any((1, 3))
    lab1 = lab[::2, ::2]
    colours = np.array([hue_rgb(i, 0.9, 1.0) for i in range(n + 1)])
    img[edge1] = colours[lab1[edge1]]

    pil = Image.fromarray(np.clip(img, 0, 255).astype(np.uint8))
    pil = draw_grid(pil, 128, 128, 0.5, 13)
    d_ = ImageDraw.Draw(pil)
    d_.text((6, 1024 - 40), "FP04RB main_body UV  x right, y down (2048 px)", fill=(255, 255, 255),
            font=font(14), stroke_width=2, stroke_fill=(0, 0, 0))
    d_.text((6, 1024 - 20), "colour = height band (Y), shading = normal; Z+ = car right",
            fill=(255, 255, 255), font=font(13), stroke_width=2, stroke_fill=(0, 0, 0))
    path = out / "fp04rb_main_body_uv_grid.png"
    pil.save(path)
    return path


def island_colour_image(islands):
    rgb = np.zeros(islands.shape + (3,), np.float32)
    for i in np.unique(islands):
        if i == 0:
            continue
        rgb[islands == i] = hue_rgb(int(i), 0.75, 0.95)
    return rgb


def island_anchor(mask):
    """Centroid if it lies on the island, else the island's interior point farthest from its edge."""
    cy, cx = ndimage.center_of_mass(mask)
    if mask[int(round(cy)), int(round(cx))]:
        return cx, cy
    dist = ndimage.distance_transform_edt(mask)
    y, x = np.unravel_index(np.argmax(dist), mask.shape)
    return x, y


def template_grid(islands_path, sheet_path, out):
    islands = np.load(islands_path)
    rgb = island_colour_image(islands)
    if sheet_path is not None:
        sheet = np.asarray(Image.open(sheet_path).convert("RGB"), np.float32)
        if sheet.shape[:2] != islands.shape:
            sheet = np.asarray(Image.fromarray(sheet.astype(np.uint8)).resize(
                islands.shape[::-1], Image.LANCZOS), np.float32)
        rgb = 0.5 * rgb + 0.5 * sheet
    pil = Image.fromarray(np.clip(rgb, 0, 255).astype(np.uint8))
    pil = draw_grid(pil, 64, 128, 1.0, 16)
    d = ImageDraw.Draw(pil)
    f = font(20)
    for i in np.unique(islands):
        if i == 0:
            continue
        x, y = island_anchor(islands == i)
        d.text((x, y), str(int(i)), fill=(255, 255, 255), font=f, anchor="mm",
               stroke_width=3, stroke_fill=(0, 0, 0))
    name = "template_islands_grid.png"
    path = out / name
    pil.save(path)
    return path


def main():
    ap = argparse.ArgumentParser(description="Grid images for reading livery mapping coordinates.")
    ap.add_argument("--out", type=Path, required=True, help="output directory")
    ap.add_argument("--sheet", type=Path, default=None, help="sheet PNG to blend under the islands")
    ap.add_argument("--bake", type=Path, default=DEFAULT_BAKE, help="bake2k main_body.npz")
    ap.add_argument("--islands", type=Path, default=DEFAULT_ISLANDS, help="islands.npy")
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    print(uv_grid(args.bake, args.out))
    print(template_grid(args.islands, args.sheet, args.out))


if __name__ == "__main__":
    main()
