"""Install native UV0 parts, 2025 UV1 sheets, or converted 2026 sheets. Run with python3 -I."""
import argparse
import json
from pathlib import Path
import sys
import shutil

import numpy as np
from PIL import Image
from scipy import ndimage as ndi

# Isolated Python omits the script directory. Import only this repository's converter.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import convert_2026

REPO = Path(__file__).resolve().parents[2]
DIRECT_PANELS = (1, 2, 4, 5, 6, 7, 10, 11, 13, 15, 16, 18, 19, 20, 21,
                 23, 24, 25, 27, 30, 31, 34, 35)


def padded_sheet(sheet, labels, padding=64):
    """Keep all island pixels at their sheet coordinates; extend edges into gutters."""
    with Image.open(sheet) as source:
        if source.size != (2048, 2048):
            raise ValueError(f'expected a 2048 square template sheet: {sheet}')
        pixels = np.array(source.convert('RGB'))
    if labels.shape != (2048, 2048):
        raise ValueError('template labels must be 2048 square')
    # Unassigned printed inserts keep their pixels, but must not supply gutter
    # colour to a neighbouring physical panel, notably the halo's inner curve.
    distance, nearest = ndi.distance_transform_edt(~np.isin(labels, DIRECT_PANELS), return_indices=True)
    gutter = (labels == 0) & (distance <= padding)
    # UV (0, 1) is the plain carbon fallback for unassigned source panels.
    gutter[:16, :16] = False
    pixels[gutter] = pixels[nearest[0][gutter], nearest[1][gutter]]
    return Image.fromarray(pixels)


def install_direct(sheet, destination, labels, padding=64):
    """Install one direct sheet and the opponent resolutions without a UV warp."""
    image = padded_sheet(sheet, labels, padding)
    destination.mkdir(parents=True, exist_ok=True)
    image.save(destination / 'template.png')
    for lod, size in [('lod1', 2048), ('lod2', 1024)]:
        (destination / lod).mkdir(exist_ok=True)
        if size == 2048:
            shutil.copyfile(destination / 'template.png', destination / lod / 'template.png')
        else:
            image.resize((size, size), Image.Resampling.LANCZOS).save(destination / lod / 'template.png')


def mappings(values, roster, native=False):
    """Validate every destination before converting or writing any sheet."""
    teams = {team['id'] for team in roster['teams']}
    jobs = {}
    for value in values:
        team, separator, sheet = value.partition('=')
        if not separator or team not in teams:
            raise ValueError(f'expected a known TEAM_ID=SHEET.png, got {value!r}')
        if team in jobs:
            raise ValueError(f'duplicate team: {team}')
        path = Path(sheet)
        if native and not path.is_dir():
            raise ValueError(f'not a native parts directory: {path}')
        if not native and not path.is_file():
            raise ValueError(f'not a sheet file: {path}')
        jobs[team] = path
    return jobs


NATIVE_PARTS = ('main_body', 'front_wing', 'rear_wing', 'mirrors', 'floor',
                'cockpit_foam', 'winglets', 'front_covers', 'rear_covers')


def install_native(source, destination, team, size=4096):
    """Validate a full native bake before replacing parts or removing UV1 sheets."""
    manifest = json.loads((source / 'native-manifest.json').read_text())
    if (manifest.get('schema') != 1 or manifest.get('team') != team
            or manifest.get('uv_channel') != 0 or manifest.get('size') != size
            or set(manifest.get('parts', [])) != set(NATIVE_PARTS)):
        raise ValueError('native bake manifest does not describe this team and all UV0 parts')
    for part in NATIVE_PARTS:
        with Image.open(source / f'{part}.png') as image:
            if image.size != (size, size):
                raise ValueError(f'{part}: expected {size} square native UV0 bake')
            image.verify()
    # All input validation completes before the existing installation is touched.
    destination.mkdir(parents=True, exist_ok=True)
    for part in NATIVE_PARTS:
        with Image.open(source / f'{part}.png') as image:
            image.convert('RGB').save(destination / f'{part}.png')
            for lod, pixels in [('lod1', 2048), ('lod2', 1024)]:
                (destination / lod).mkdir(exist_ok=True)
                image.convert('RGB').resize((pixels, pixels), Image.Resampling.LANCZOS).save(destination / lod / f'{part}.png')
    shutil.copyfile(source / 'native-manifest.json', destination / 'native-manifest.json')
    # Direct sheets take precedence at runtime, so remove just this team's three.
    for folder in [destination, destination / 'lod1', destination / 'lod2']:
        (folder / 'template.png').unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--map', action='append', required=True, metavar='TEAM_ID=SHEET.png')
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--native', action='store_true', help='map TEAM_ID to a complete native UV0 bake directory')
    mode.add_argument('--template-year', choices=['2025', '2026'], default='2026',
                        help='2025 uses direct UV1; 2026 retains the original-UV converter')
    args = parser.parse_args()
    root = REPO / 'assets/cars/gen22'
    try:
        jobs = mappings(args.map, json.loads((root / 'teams.json').read_text()), args.native)
    except ValueError as error:
        parser.error(str(error))
    for team, sheet in jobs.items():
        destination = root / 'liveries' / team
        if args.native:
            install_native(sheet, destination, team)
            print(f'{team}: native UV0 parts, 4096/2048/1024 -> {destination}')
            continue
        if args.template_year == '2025':
            labels = np.load(REPO / 'raw/gen22/livery-mapping/2025/islands.npy')
            install_direct(sheet, destination, labels)
            print(f'{team}: direct 2025 sheet, 2048/2048/1024 -> {destination}')
            continue
        parts = convert_2026.convert(convert_2026.DEFAULT_MAP, sheet, destination, 4096)
        for lod, size in [('lod1', 2048), ('lod2', 1024)]:
            (destination / lod).mkdir(exist_ok=True)
            for part in parts:
                with Image.open(destination / f'{part}.png') as image:
                    image.resize((size, size), Image.Resampling.LANCZOS).save(destination / lod / f'{part}.png')
        for folder in [destination, destination / 'lod1', destination / 'lod2']:
            (folder / 'template.png').unlink(missing_ok=True)
        print(f'{team}: {len(parts)} parts at 4096/2048/1024 -> {destination}')


if __name__ == '__main__':
    main()
