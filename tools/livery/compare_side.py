"""Side-by-side (or stacked) comparison of a preview render and a reference image.

Usage:
  python3 -I tools/livery/compare_side.py --render RENDER.png --ref REF.png --out OUT.png
         [--ref-crop x0,y0,x1,y1] [--render-crop x0,y0,x1,y1] [--flip] [--stack]
         [--height 800]

--ref-crop / --render-crop are boxes in that image's own pixels (default: whole image).
--flip mirrors the reference horizontally, e.g. to compare a car-left reference with a
car-right render. Both panels are scaled to the same height (or width when --stack).
"""
import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont, ImageOps

GAP = 12
LABEL_H = 28
BG = (24, 24, 28)


def parse_box(text):
    if text is None:
        return None
    vals = [int(v) for v in text.split(",")]
    if len(vals) != 4:
        raise argparse.ArgumentTypeError("box must be x0,y0,x1,y1")
    return tuple(vals)


def load(path, box=None):
    img = Image.open(path).convert("RGB")
    return img.crop(box) if box else img


def font(size):
    try:
        return ImageFont.load_default(size=size)
    except TypeError:
        return ImageFont.load_default()


def labelled(img, text, width, height):
    """Panel of the given size with a label strip above the image."""
    panel = Image.new("RGB", (width, height + LABEL_H), BG)
    panel.paste(img.resize((width, height), Image.LANCZOS), (0, LABEL_H))
    ImageDraw.Draw(panel).text((8, 6), text, fill=(235, 235, 235), font=font(16))
    return panel


def compose(render, ref, stack, target):
    """Scale both panels to one shared dimension (height side by side, width stacked)."""
    if stack:
        w = target
        h1 = round(render.height * w / render.width)
        h2 = round(ref.height * w / ref.width)
        return w, h1, h2
    h = target
    w1 = round(render.width * h / render.height)
    w2 = round(ref.width * h / ref.height)
    return h, w1, w2


def main():
    ap = argparse.ArgumentParser(description="Compare a Blender preview render with a reference.")
    ap.add_argument("--render", type=Path, required=True, help="preview PNG (e.g. side_right.png)")
    ap.add_argument("--ref", type=Path, required=True, help="reference image")
    ap.add_argument("--out", type=Path, required=True, help="output PNG")
    ap.add_argument("--ref-crop", type=parse_box, default=None, help="x0,y0,x1,y1 in reference pixels")
    ap.add_argument("--render-crop", type=parse_box, default=None, help="x0,y0,x1,y1 in render pixels")
    ap.add_argument("--flip", action="store_true", help="mirror the reference horizontally")
    ap.add_argument("--stack", action="store_true", help="stack vertically instead of side by side")
    ap.add_argument("--height", type=int, default=800, help="panel height (or width with --stack)")
    args = ap.parse_args()

    render = load(args.render, args.render_crop)
    ref = load(args.ref, args.ref_crop)
    if args.flip:
        ref = ImageOps.mirror(ref)
    ref_label = f"reference: {args.ref.name}" + (" (flipped)" if args.flip else "")
    if args.ref_crop:
        ref_label += f" crop {args.ref_crop}"
    render_label = f"render: {args.render.name}"

    target = args.height
    shared, a, b = compose(render, ref, args.stack, target)
    if args.stack:
        size_a, size_b = (shared, a), (shared, b)
    else:
        size_a, size_b = (a, shared), (b, shared)
    pa = labelled(render, render_label, *size_a)
    pb = labelled(ref, ref_label, *size_b)

    if args.stack:
        out = Image.new("RGB", (max(pa.width, pb.width), pa.height + pb.height + GAP), BG)
        out.paste(pa, (0, 0))
        out.paste(pb, (0, pa.height + GAP))
    else:
        out = Image.new("RGB", (pa.width + pb.width + GAP, max(pa.height, pb.height)), BG)
        out.paste(pa, (0, 0))
        out.paste(pb, (pa.width + GAP, 0))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    out.save(args.out)
    print(args.out)


if __name__ == "__main__":
    main()
