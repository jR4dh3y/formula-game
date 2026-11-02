"""Apply the 2026 livery mapping to Monoposto sheets.

Usage:
  python3 -I tools/livery/convert_2026.py SHEET_OR_DIR --out DIR [--map PATH] [--size 4096]

SHEET_OR_DIR is either one 2048px sheet PNG, which writes DIR/<part>.png, or a
directory, where every *.png inside is converted into DIR/<png stem>/<part>.png.
"""
import argparse
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

REPO = Path(__file__).resolve().parents[2]
DEFAULT_MAP = REPO / "raw/gen22/livery-mapping/map_v3.npz"

CARBON = np.array([18, 20, 23], np.float32)


def bilinear(img, x, y):
    h, w = img.shape[:2]
    x = np.clip(x, 0, w - 1.001); y = np.clip(y, 0, h - 1.001)
    x0 = np.floor(x).astype(int); y0 = np.floor(y).astype(int)
    fx = (x - x0)[:, None]; fy = (y - y0)[:, None]
    a = img[y0, x0]; b = img[y0, x0 + 1]; c = img[y0 + 1, x0]; d = img[y0 + 1, x0 + 1]
    return (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + d * fx) * fy


def dominant(pixels):
    q = (pixels // 16).astype(int)
    key = q[:, 0] * 256 + q[:, 1] * 16 + q[:, 2]
    k = np.bincount(key).argmax()
    return pixels[key == k].mean(0)


def resample_map(xy, fill, region, size, start, stop):
    """Interpolate within a chart, selecting the nearest chart at a seam.

    Pixel-centre alignment matches normalized GLB UV sampling. A chart boundary
    must not interpolate between unrelated locations on the source sheet.
    """
    h, w = fill.shape
    y = (np.arange(start, stop) + .5) * h / size - .5
    x = (np.arange(size) + .5) * w / size - .5
    yy, xx = np.meshgrid(y, x, indexing="ij")
    nr = np.clip(np.floor(yy + .5).astype(int), 0, h-1)
    nc = np.clip(np.floor(xx + .5).astype(int), 0, w-1)
    kinds = fill[nr, nc]
    chart = region[nr, nc]
    y0, x0 = np.floor(yy).astype(int), np.floor(xx).astype(int)
    fy, fx = yy-y0, xx-x0
    total = np.zeros(xx.shape)
    value = np.zeros(xx.shape + (2,))
    for dy, dx, weight in ((0,0,(1-fy)*(1-fx)), (0,1,(1-fy)*fx),
                           (1,0,fy*(1-fx)), (1,1,fy*fx)):
        r, c = np.clip(y0+dy, 0, h-1), np.clip(x0+dx, 0, w-1)
        sample = xy[r,c]
        same = (region[r,c] == chart) & (fill[r,c] == kinds) & np.isfinite(sample).all(-1)
        weight = weight * same
        value += np.nan_to_num(sample) * weight[...,None]
        total += weight
    out = np.full(value.shape, np.nan, np.float32)
    np.divide(value, total[...,None], out=out, where=total[...,None] > 0)
    return out, kinds


def convert(map_path, sheet_path, out, size=4096):
    """Write one <part>.png per mapped part into out. Returns the part names."""
    M = np.load(map_path)
    sheet = np.asarray(Image.open(sheet_path).convert("RGB"), np.float32)
    if sheet.shape[:2] != (2048, 2048):
        sheet = np.asarray(Image.fromarray(sheet.astype(np.uint8)).resize((2048, 2048), Image.LANCZOS), np.float32)
    out = Path(out); out.mkdir(parents=True, exist_ok=True)
    body_px = []
    parts = [k[:-3] for k in M.files if k.endswith("_xy")]
    for part in parts:
        xy = M[f"{part}_xy"]
        has = ~np.isnan(xy[..., 0])
        if has.any():
            body_px.append(bilinear(sheet, xy[has][:, 0], xy[has][:, 1]))
    base = dominant(np.concatenate(body_px)) if body_px else CARBON
    for part in parts:
        xy = M[f"{part}_xy"]; fill = M[f"{part}_fill"]
        region = M[f"{part}_region"]
        valid = np.isfinite(xy).all(-1)
        occupied = valid | (fill > 0)
        # Pad UV gutters, but never overwrite intentional body/carbon fill.
        if occupied.any():
            _, idx = ndimage.distance_transform_edt(~occupied, return_indices=True)
            xy = xy[idx[0], idx[1]]
            fill = fill[idx[0], idx[1]]
            region = region[idx[0], idx[1]]
        img = np.empty((size, size, 3), np.uint8)
        for start in range(0, size, 128):
            stop = min(start + 128, size)
            coords, kinds = resample_map(xy, fill, region, size, start, stop)
            tile = np.broadcast_to(CARBON, (stop-start, size, 3)).copy()
            tile[kinds == 1] = base
            mapped = np.isfinite(coords).all(-1) & (kinds == 0)
            tile[mapped] = bilinear(sheet, coords[mapped, 0], coords[mapped, 1])
            img[start:stop] = tile.clip(0, 255).astype(np.uint8)
        Image.fromarray(img.clip(0, 255).astype(np.uint8)).save(out / f"{part}.png")
    return parts


def main():
    ap = argparse.ArgumentParser(description="Apply the 2026 livery mapping to Monoposto sheets.")
    ap.add_argument("input", type=Path, help="one sheet PNG, or a directory of *.png sheets")
    ap.add_argument("--out", type=Path, required=True, help="output directory")
    ap.add_argument("--map", type=Path, default=DEFAULT_MAP, help="map_v3.npz from build_2026.sh")
    ap.add_argument("--size", type=int, default=4096, help="output texture size per part")
    args = ap.parse_args()

    if args.input.is_dir():
        sheets = sorted(args.input.glob("*.png"))
        if not sheets:
            ap.error(f"no *.png files in {args.input}")
        jobs = [(sheet, args.out / sheet.stem) for sheet in sheets]
    elif args.input.is_file():
        jobs = [(args.input, args.out)]
    else:
        ap.error(f"not a file or directory: {args.input}")

    for sheet, dest in jobs:
        parts = convert(args.map, sheet, dest, args.size)
        print(f"{sheet.name}: {len(parts)} parts -> {dest}")


if __name__ == "__main__":
    main()
