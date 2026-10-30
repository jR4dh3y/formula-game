"""Import PNG paint, suit and helmet atlases, or export the matching Gen22 UV templates.

Native Gen22 atlases import directly. The monoposto-modern paint profile uses the
official Monoposto 2026 layout; other layouts need other profiles.
"""
import argparse
import io
import json
from pathlib import Path
import re
import struct
import stat

import sys
import tempfile

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
SIZE = 2048


def png_bytes(source, background):
    """Flatten optional PNG overlays over the current atlas, in the same UV layout."""
    with Image.open(source) as image:
        if image.format != 'PNG':
            raise ValueError('Expected a PNG image')
        width, height = image.size
        if width != height or not 256 <= width <= 8192:
            raise ValueError('Atlas must be square, between 256 and 8192 pixels')
        foreground = image.convert('RGBA').resize((SIZE, SIZE), Image.Resampling.LANCZOS)
    with Image.open(background) as image:
        base = image.convert('RGBA').resize((SIZE, SIZE), Image.Resampling.LANCZOS)
    result = Image.alpha_composite(base, foreground).convert('RGB')
    output = io.BytesIO()
    result.save(output, format='PNG')
    return output.getvalue()


def install(directory, overrides, roster, team_id, driver_id, images):
    team = next((team for team in roster['teams'] if team['id'] == team_id), None)
    if team is None or not re.fullmatch(r'[a-z0-9_-]+', team_id):
        raise ValueError(f'Unknown team: {team_id}')
    if images.get('helmet') and not any(driver['id'] == driver_id for driver in team['drivers']):
        raise ValueError('Helmet import requires a driver from the selected team')
    if driver_id and not re.fullmatch(r'[a-z0-9_-]+', driver_id):
        raise ValueError('Invalid driver ID')
    pending = []
    for surface, source in images.items():
        if source is None:
            continue
        relative = Path(team_id) / (Path(driver_id) / 'helmet.png' if surface == 'helmet' else Path(surface + '.png'))
        pending.append((relative, png_bytes(source, directory / 'liveries' / relative)))
    if not pending:
        raise ValueError('Supply at least one of --paint, --suit or --helmet')
    # Stage every new image and every rollback copy before publishing any of
    # them. A later rename failure must not leave a new persistent override.
    staged = []
    backups = {}
    published = []
    keep = set()
    try:
        for relative, data in pending:
            for root in (overrides, directory / 'liveries'):
                destination = root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                with tempfile.NamedTemporaryFile(prefix='.gen22-import-', dir=destination.parent, delete=False) as file:
                    temporary = Path(file.name)
                    staged.append((temporary, destination))
                    file.write(data)
                temporary.chmod(stat.S_IMODE(destination.stat().st_mode) if destination.exists() else 0o644)
                backups[destination] = None
                if destination.exists():
                    with tempfile.NamedTemporaryFile(prefix='.gen22-backup-', dir=destination.parent, delete=False) as file:
                        backup = Path(file.name)
                        backups[destination] = backup
                        file.write(destination.read_bytes())
                    backup.chmod(stat.S_IMODE(destination.stat().st_mode))
        for temporary, destination in staged:
            temporary.replace(destination)
            published.append(destination)
    except BaseException as original:
        failed = []
        for destination in reversed(published):
            backup = backups[destination]
            try:
                if backup is None:
                    destination.unlink(missing_ok=True)
                else:
                    backup.replace(destination)
            except OSError:
                if backup is not None:
                    keep.add(backup)
                failed.append(str(destination))
        if failed:
            raise RuntimeError('Livery rollback failed; backup files retained for: ' + ', '.join(failed)) from original
        raise
    finally:
        for path in [temp for temp, _ in staged] + [backup for backup in backups.values() if backup is not None]:
            if path not in keep:
                path.unlink(missing_ok=True)

    if images.get('paint') is not None:
        (overrides / team_id / 'paint.source.json').unlink(missing_ok=True)
    return [directory / 'liveries' / relative for relative, _ in pending]


def apply_overrides(directory, overrides, roster, bake=None):
    """Only accept roster paths; unrelated files in the override tree are ignored."""
    converter = None
    for team in roster['teams']:
        provenance = overrides / team['id'] / 'paint.source.json'
        if bake is not None and provenance.is_file():
            report = json.loads(provenance.read_text())
            if report.get('profile') != 'monoposto-modern':
                raise ValueError('Unknown saved paint source profile: ' + str(report.get('profile')))
            if report.get('profile') == 'monoposto-modern':
                # Atlas packing can change on rebuild. Reproject the original sheet.
                from convert_livery import Converter
                if converter is None:
                    converter = Converter(bake)
                import hashlib
                if report.get('source_sha256') and hashlib.sha256(Path(report['source']).read_bytes()).hexdigest() != report['source_sha256']:
                    raise ValueError('Saved livery source changed; import the source explicitly again')
                output = provenance.with_name('paint.png')
                converter.convert(Path(report['source']), output)
                provenance.write_bytes(output.with_name('paint.png.json').read_bytes())
        paths = [Path(team['id']) / name for name in ('paint.png', 'suit.png')]
        paths += [Path(team['id']) / driver['id'] / 'helmet.png' for driver in team['drivers']]
        for relative in paths:
            source = overrides / relative
            if source.is_file():
                destination = directory / 'liveries' / relative
                data = png_bytes(source, destination)
                destination.write_bytes(data)
                print('CUSTOM LIVERY', relative, flush=True)


def export_templates(directory, output):
    # Read the actual exported UV0, so a stale/source-model template cannot be
    # mistaken for the current car's atlas. glTF stores V bottom-up.
    data = (directory / 'car.glb').read_bytes()
    length = struct.unpack_from('<I', data, 12)[0]
    document = json.loads(data[20:20 + length])
    binary = data[28 + length:]

    def values(index):
        accessor = document['accessors'][index]
        view = document['bufferViews'][accessor['bufferView']]
        components = {'SCALAR': 1, 'VEC2': 2}[accessor['type']]
        kind = {5123: 'H', 5125: 'I', 5126: 'f'}[accessor['componentType']]
        fmt = '<' + kind * components
        stride = view.get('byteStride', struct.calcsize(fmt))
        start = view.get('byteOffset', 0) + accessor.get('byteOffset', 0)
        return [struct.unpack_from(fmt, binary, start + i * stride) for i in range(accessor['count'])]

    output.mkdir(parents=True, exist_ok=True)
    for surface in ('paint', 'suit', 'helmet_paint'):
        image = Image.new('RGBA', (SIZE, SIZE), (0, 0, 0, 0))
        draw = ImageDraw.Draw(image)
        for mesh in document['meshes']:
            for primitive in mesh['primitives']:
                if document['materials'][primitive['material']]['name'] != surface:
                    continue
                uv = values(primitive['attributes']['TEXCOORD_0'])
                indices = [v[0] for v in values(primitive['indices'])]
                for i in range(0, len(indices), 3):
                    points = [(round(uv[j][0] * (SIZE - 1)), round((1 - uv[j][1]) * (SIZE - 1))) for j in indices[i:i + 3]]
                    draw.line(points + points[:1], fill=(80, 210, 255, 190), width=1)
        image.save(output / (surface + '-uv.png'))
    (output / 'README.txt').write_text('Gen22 UV0 templates from the exported car.glb.\nUse as a guide layer; hide it before exporting paint/suit/helmet PNG artwork.\nMonoposto square modern sheets can use --layout monoposto-modern; other layouts need a matching profile.\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', type=Path, default=ROOT / 'assets/cars/gen22')
    parser.add_argument('--overrides', type=Path, default=ROOT / 'raw/gen22/liveries')
    parser.add_argument('--team')
    parser.add_argument('--driver', default='')
    parser.add_argument('--paint', type=Path)
    parser.add_argument('--suit', type=Path)
    parser.add_argument('--helmet', type=Path)
    parser.add_argument('--layout', choices=['gen22', 'monoposto-modern'], help='Input car-paint layout; Monoposto conversion is a authored mesh UV correspondence')
    parser.add_argument('--bake', type=Path, default=ROOT / 'raw/gen22/bake/paint.npz')
    parser.add_argument('--templates', type=Path, help='Export matching UV guide PNGs to this directory')
    args = parser.parse_args()
    try:
        if args.templates:
            export_templates(args.assets, args.templates)
        else:
            if not args.team or not args.layout:
                parser.error('Import requires --team and an explicit --layout')
            if args.layout == 'monoposto-modern' and (not args.paint or args.suit or args.helmet):
                parser.error('monoposto-modern converts car paint only; import suit/helmet separately with --layout gen22')
            roster = json.loads((args.assets / 'teams.json').read_text())
            with tempfile.TemporaryDirectory(prefix='gen22-livery-import-') as scratch:
                paint = args.paint
                if args.layout == 'monoposto-modern':
                    sys.path.insert(0, str(Path(__file__).resolve().parent))
                    from convert_livery import Converter
                    paint = Path(scratch) / 'paint.png'
                    Converter(args.bake).convert(args.paint, paint)
                for path in install(args.assets, args.overrides, roster, args.team, args.driver,
                                    {'paint': paint, 'suit': args.suit, 'helmet': args.helmet}):
                    print('IMPORTED', path)
                if args.layout == 'monoposto-modern':
                    provenance = args.overrides / args.team / 'paint.source.json'
                    provenance.write_bytes(paint.with_name(paint.name + '.json').read_bytes())
                    print('PROFILE monoposto-modern: authored mesh UV correspondence; check the result on the car.')
            print('Restart the game, then select the team/driver in Race setup.')
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        parser.exit(1, f'Livery import failed: {error}\n')


if __name__ == '__main__':
    main()
