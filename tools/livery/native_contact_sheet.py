"""Build bounded reference | Blender | Vulkan contacts from a review manifest.

Run with python3 -I. Every input is resized before it enters a contact sheet.
The manifest is a JSON list of name/reference/blender/game paths. Optional
reference_box, blender_box and game_box select comparable details. No raw image
is displayed by this tool.
"""
import argparse
import json
from pathlib import Path

from PIL import Image, ImageDraw


WIDTH, HEIGHT = 420, 250


def contact(row):
    sheet = Image.new('RGB', (1280, 290), '#252525')
    draw = ImageDraw.Draw(sheet)
    for column, key in enumerate(('reference', 'blender', 'game')):
        path = Path(row[key]).expanduser()
        with Image.open(path) as source:
            image = source.convert('RGB')
            if row.get(key + '_box'):
                image = image.crop(row[key + '_box'])
            image.thumbnail((WIDTH, HEIGHT), Image.Resampling.LANCZOS)
            x, y = column * 428, 30
            sheet.paste(image, (x + (WIDTH-image.width)//2, y + (HEIGHT-image.height)//2))
        draw.text((x+6, 7), row['name'] + ' | ' + key, fill='white')
    return sheet


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    rows = json.loads(args.manifest.read_text())
    sheets = []
    for row in rows:
        image = contact(row)
        image.save(args.out / (row['name']+'.jpg'), quality=80)
        sheets.append(image)
    for start in range(0, len(sheets), 4):
        batch = sheets[start:start+4]
        page = Image.new('RGB', (1280, 290*len(batch)), '#252525')
        for i, image in enumerate(batch):
            page.paste(image, (0, i*290))
        page.save(args.out / f'contact-{start//4+1:02}.jpg', quality=80)


if __name__ == '__main__':
    main()
