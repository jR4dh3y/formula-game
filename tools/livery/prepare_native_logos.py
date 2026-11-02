"""Rasterise audited brand SVGs into tightly cropped, transparent decal artwork.

Run with python3 -I. Sources remain separate from derived colour variants.
The manifest records source URLs and licences; no downloaded code is executed.
"""
import argparse
from io import BytesIO
import json
from pathlib import Path
import subprocess
import urllib.request
import xml.etree.ElementTree as ET

import numpy as np
from PIL import Image, ImageDraw, ImageFilter
from scipy import ndimage


def transparent_background(image):
    """Remove only border-connected white, preserving white inside a mark."""
    a = np.array(image.convert('RGBA'))
    white = (a[:, :, :3].min(2) > 245) & (a[:, :, 3] > 0)
    seed = np.zeros_like(white)
    seed[0] = white[0]
    seed[-1] = white[-1]
    seed[:, 0] = white[:, 0]
    seed[:, -1] = white[:, -1]
    outside = ndimage.binary_propagation(seed, mask=white)
    a[outside, 3] = 0
    return Image.fromarray(a)


def prepare(entry, source):
    if source.suffix.lower() == '.svg':
        if 'svg_paths' in entry:
            # Keep measured components of genuine artwork, such as a star without its ring.
            root = ET.fromstring(source.read_bytes())
            paths = list(root.iter('{http://www.w3.org/2000/svg}path'))
            keep = {paths[index] for index in entry['svg_paths']}
            for parent in root.iter():
                for child in list(parent):
                    if child.tag == '{http://www.w3.org/2000/svg}path' and child not in keep:
                        parent.remove(child)
            data = subprocess.run(['rsvg-convert', '-w', '4096'], input=ET.tostring(root), check=True, capture_output=True).stdout
        else:
            data = subprocess.run(['rsvg-convert', '-w', '4096', str(source)], check=True, capture_output=True).stdout
        image = Image.open(BytesIO(data)).convert('RGBA')
    else:
        image = Image.open(source).convert('RGBA')
    if entry.get('source_quad'):
        image = image.transform(tuple(entry['rectified_size']), Image.Transform.QUAD,
                                entry['source_quad'], Image.Resampling.BICUBIC)
    if entry.get('source_crop'):
        image = image.crop(entry['source_crop'])
    if entry.get('source_polygon'):
        mask = Image.new('L', image.size)
        ImageDraw.Draw(mask).polygon([tuple(point) for point in entry['source_polygon']], fill=255)
        image.putalpha(Image.fromarray(np.minimum(np.array(image.getchannel('A')), np.array(mask))))
    if entry.get('source_rotate'):
        image = image.rotate(entry['source_rotate'], expand=True)
    if entry.get('remove_all_white'):
        pixels = np.array(image)
        pixels[pixels[:, :, :3].min(2) > 235, 3] = 0
        image = Image.fromarray(pixels)
    if 'dark_ink' in entry:
        # Isolate genuine dark photo lettering without substituting a font.
        pixels = np.array(image)
        alpha = np.where(pixels[:, :, :3].max(2) < entry['dark_ink'], pixels[:, :, 3], 0).astype(np.uint8)
        image = Image.new('RGBA', image.size, 'white')
        image.putalpha(Image.fromarray(alpha))
    if 'rgb_ink' in entry:
        # Select coloured photographic glyphs, retaining their negative space.
        pixels = np.array(image)
        lower, upper = np.asarray(entry['rgb_ink'])
        selected = ((pixels[:, :, :3] >= lower) & (pixels[:, :, :3] <= upper)).all(2)
        alpha = np.where(selected, pixels[:, :, 3], 0).astype(np.uint8)
        image = Image.new('RGBA', image.size, 'white')
        image.putalpha(Image.fromarray(alpha))
    if entry.get('photo_crop'):
        image = image.crop(entry['photo_crop']).rotate(entry.get('rotate', 0), expand=True)
        pixels = np.array(image)
        mask = Image.fromarray(pixels[:, :, :3].min(2)).filter(ImageFilter.GaussianBlur(.55))
        alpha = np.asarray(mask) > 170
        labels, _ = ndimage.label(alpha)
        sizes = np.bincount(labels.ravel())
        sizes[0] = 0
        keep = np.argsort(sizes)[-2:]
        alpha = np.isin(labels, keep).astype(np.uint8) * 255
        image = Image.new('RGBA', image.size, 'white')
        image.putalpha(Image.fromarray(alpha))
    if entry.get('remove_white'):
        image = transparent_background(image)
    image = image.crop(image.getbbox())
    if entry.get('crop'):
        x0, y0, x1, y1 = entry['crop']
        image = image.crop((round(x0*image.width), round(y0*image.height), round(x1*image.width), round(y1*image.height)))
        image = image.crop(image.getbbox())
    if entry.get('white_ink'):
        pixels = np.array(image)
        alpha = np.where(pixels[:, :, :3].min(2) > 210, pixels[:, :, 3], 0).astype(np.uint8)
        image = Image.new('RGBA', image.size, 'white')
        image.putalpha(Image.fromarray(alpha))
        image = image.crop(image.getbbox())
    if entry.get('red_ink'):
        pixels = np.array(image)
        alpha = np.where((pixels[:, :, 0] > 130) & (pixels[:, :, 1] < 110) & (pixels[:, :, 2] < 110), pixels[:, :, 3], 0).astype(np.uint8)
        image = Image.new('RGBA', image.size, 'white')
        image.putalpha(Image.fromarray(alpha))
        image = image.crop(image.getbbox())
    if 'largest_components' in entry:
        alpha = np.array(image.getchannel('A'))
        labels, _ = ndimage.label(alpha > 0)
        sizes = np.bincount(labels.ravel())
        sizes[0] = 0
        keep = np.argsort(sizes)[-entry['largest_components']:]
        alpha[~np.isin(labels, keep)] = 0
        image.putalpha(Image.fromarray(alpha))
        image = image.crop(image.getbbox())
    if entry.get('flip_x'):
        image = image.transpose(Image.Transpose.FLIP_LEFT_RIGHT)
    if entry.get('color'):
        alpha = image.getchannel('A')
        image = Image.new('RGBA', image.size, entry['color'])
        image.putalpha(alpha)
    if entry.get('hp_roundel'):
        alpha = image.getchannel('A')
        image = Image.new('RGBA', image.size, '#0076BD')
        image.putalpha(alpha)
        background = Image.new('RGBA', image.size)
        ImageDraw.Draw(background).ellipse((0, 0, image.width-1, image.height-1), fill='white')
        image = Image.alpha_composite(background, image)
    image.thumbnail((2048, 2048), Image.Resampling.LANCZOS)
    if entry.get('outline'):
        pad = 12
        expanded = Image.new('RGBA', (image.width+pad*2, image.height+pad*2))
        expanded.paste(image, (pad, pad))
        back = Image.new('RGBA', expanded.size, 'white')
        back.putalpha(expanded.getchannel('A').filter(ImageFilter.MaxFilter(15)))
        image = Image.alpha_composite(back, expanded)
    if image.getbbox() is None:
        raise ValueError(f'artwork preparation removed every pixel: {source}')
    return image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--directory', required=True, type=Path)
    parser.add_argument('--download', action='store_true')
    args = parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    entries = json.loads(args.manifest.read_text())
    for entry in entries:
        source = args.directory / entry['source']
        if args.download and not source.exists():
            request = urllib.request.Request(entry['url'], headers={'User-Agent': 'Mozilla/5.0'})
            with urllib.request.urlopen(request, timeout=30) as response:
                source.write_bytes(response.read())
        destination = args.directory / entry['output']
        temporary = destination.with_suffix('.tmp.png')
        prepare(entry, source).save(temporary)
        temporary.replace(destination)
    (args.directory / 'SOURCES.tsv').write_text('output\tsource\turl\tnotes\n' + ''.join(
        '\t'.join((e['output'], e['source'], e['url'], e['notes'])) + '\n' for e in entries))


if __name__ == '__main__':
    main()
