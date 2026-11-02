"""Render a colour-coded test sheet of the 2026 UV islands.

Usage: python3 -I tools/livery/make_test_sheet.py ISLANDS.npy OUT.png

Each island label gets its own colour on a 2048px sheet, with a 64px grid
drawn over it and 'F' / 'col+row' markers stamped where an island is present.
"""
import argparse
import colorsys

import numpy as np
from PIL import Image, ImageDraw, ImageFont

FONT_PATH = "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf"
H = 2048


def font(size):
    try:
        return ImageFont.truetype(FONT_PATH, size)
    except OSError:
        return ImageFont.load_default(size=size)


def main():
    ap = argparse.ArgumentParser(description="Render a colour-coded test sheet of island labels.")
    ap.add_argument("islands", help="islands.npy written by template_islands.py")
    ap.add_argument("out", help="output PNG path")
    args = ap.parse_args()

    L = np.load(args.islands)
    img = np.zeros((H, H, 3), np.uint8)
    for i in range(1, L.max() + 1):
        r, g, b = colorsys.hsv_to_rgb((i * 0.618) % 1, 0.55, 0.95)
        img[L == i] = (int(r * 255), int(g * 255), int(b * 255))
    yy, xx = np.mgrid[0:H, 0:H]
    img[((xx % 64) < 3) | ((yy % 64) < 3)] = (30, 30, 30)

    im = Image.fromarray(img)
    d = ImageDraw.Draw(im)
    f40 = font(40)
    for y in range(16, H, 128):
        for x in range(16, H, 128):
            if L[min(y + 20, H - 1), min(x + 15, H - 1)] > 0:
                d.text((x, y), 'F', fill=(0, 0, 0), font=f40)
    f22 = font(22)
    for y in range(80, H, 128):
        for x in range(80, H, 128):
            if L[min(y + 15, H - 1), min(x + 30, H - 1)] > 0:
                d.text((x, y), f'{x // 128}{"ABCDEFGHIJKLMNOP"[y // 128]}',
                       fill=(255, 255, 255), font=f22)
    im.save(args.out)


if __name__ == "__main__":
    main()
