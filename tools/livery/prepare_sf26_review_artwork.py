"""Prepare Ferrari review-01 artwork corrections from recorded sources.

Run with python3 -I. SVG rendering uses librsvg; downloaded code is never run.
The photographic crops are from launch-02.jpg in the existing SF26 reference set.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile
import xml.etree.ElementTree as ET

import numpy as np
from PIL import Image, ImageFilter


def unicredit(source, output):
    """Retain the slash as negative space and outline the dark wordmark."""
    tree = ET.parse(source)
    root = tree.getroot()
    root.attrib.update(viewBox='0 0 46.3 9.2', width='2315', height='460')
    for node in root.iter():
        if node.tag.endswith('path'):
            node.set('fill', '#000000' if node.get('fill') == '#fff' else '#ffffff')
    with tempfile.TemporaryDirectory() as temporary:
        svg, png = Path(temporary) / 'mark.svg', Path(temporary) / 'mark.png'
        tree.write(svg)
        subprocess.run(['rsvg-convert', str(svg), '-o', str(png)], check=True)
        with Image.open(png) as image:
            pixels = np.array(image.convert('RGBA'))
    pixels[:, :, 3] = (pixels[:, :, 3].astype(float) * pixels[:, :, :3].max(2) / 255).astype('uint8')
    pixels[:, :, :3] = 255
    image = Image.fromarray(pixels)
    image = image.crop(image.getchannel('A').getbbox())
    mask = image.getchannel('A')
    result = Image.new('RGBA', image.size, 'white')
    result.putalpha(mask.filter(ImageFilter.MaxFilter(25)))
    # The icon remains white; only the wordmark has the dark interior.
    interior = np.array(mask)
    interior[:, :int(image.height * 1.12)] = 0
    dark = Image.new('RGBA', image.size, '#111111')
    dark.putalpha(Image.fromarray(interior))
    result.alpha_composite(dark)
    result.save(output)


def photo_badge(reference, box, output, dark_ink=False):
    """Isolate photographed ink, retaining its supplied low-resolution silhouette."""
    with Image.open(reference) as source:
        image = source.crop(box).convert('RGB')
    pixels = np.asarray(image).astype(float)
    if dark_ink:
        alpha = np.clip((175 - pixels.min(2)) * 4, 0, 255).astype('uint8')
        image = image.convert('RGBA')
    else:
        alpha = np.clip((pixels.min(2) - 70) * 2.7, 0, 255).astype('uint8')
        image = Image.new('RGBA', image.size, 'white')
    image.putalpha(Image.fromarray(alpha))
    image.resize((image.width * 10, image.height * 10), Image.Resampling.LANCZOS).save(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sources', required=True, type=Path)
    parser.add_argument('--references', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    unicredit(args.sources / 'unicredit-1-extra.svg', args.out / 'unicredit-outline-white.png')
    with Image.open(args.sources / 'hp-blue.png') as source:
        image = source.convert('RGBA')
        middle = image.width // 2
        image.crop((0, 0, middle, image.height)).save(args.out / 'hp-top-left.png')
        image.crop((middle, 0, image.width, image.height)).save(args.out / 'hp-top-right.png')
    reference = args.references / 'launch-02.jpg'
    photo_badge(reference, (1237, 736, 1270, 768), args.out / 'fia-photo-white.png')
    photo_badge(reference, (1053, 600, 1078, 624), args.out / 'circled-e-colour.png', dark_ink=True)
    (args.out / 'REVIEW-01-SOURCES.tsv').write_text(
        'unicredit-outline-white.png\tunicredit-1-extra.svg\t'
        'https://cdn.worldvectorlogo.com/logos/unicredit-1.svg\tnegative slash; dark outlined wordmark\n'
        'hp-top-left.png\thp-blue.png\tprepared Simple Icons HP\tleft half for UV seam\n'
        'hp-top-right.png\thp-blue.png\tprepared Simple Icons HP\tright half for UV seam\n'
        'fia-photo-white.png\tlaunch-02.jpg\tcrop 1237,736,1270,768\twhite ink isolated from supplied photo\n'
        'circled-e-colour.png\tlaunch-02.jpg\tcrop 1053,600,1078,624\tdark/red ink isolated from supplied photo\n'
    )


if __name__ == '__main__':
    main()
