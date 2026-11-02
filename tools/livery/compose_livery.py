"""Paint native UV0 textures in 2D and export editable OpenRaster documents.

python3 -I tools/livery/compose_livery.py TEAM.layout.json --logos LOGOS --out OUT
python3 -I tools/livery/compose_livery.py --reimport EDITED_ORA_DIRECTORY --out OUT

No Blender, projection painter, lighting bake, or team-specific code is used.
"""
import argparse
from io import BytesIO
import json
import os
from pathlib import Path
import sys
import tempfile
import xml.etree.ElementTree as ET
import zipfile

import numpy as np
from PIL import Image, ImageChops, ImageColor, ImageDraw
from scipy import ndimage as ndi

sys.path.insert(0, str(Path(__file__).resolve().parent))
from uv_layout import DEFAULT_TEMPLATES, PARTS, load_index, panel_lookup, panel_points, sha256


def color(value, palette):
    return ImageColor.getcolor(palette.get(value, value), 'RGBA')


def read_mask(directory, relative, size):
    with Image.open(directory / relative) as source:
        return source.convert('L').resize((size, size), Image.Resampling.NEAREST)


def layer_mask(layer, part, index, directory, size, clip_panel=True):
    mask = read_mask(directory, f'{part}-coverage.png', size)
    panel = None
    if layer.get('panel'):
        panel = panel_lookup(index, layer['panel'], part)
        if clip_panel:
            mask = ImageChops.multiply(mask, read_mask(directory, panel['mask'], size))
    if layer.get('panels') and clip_panel:
        union = Image.new('L', (size, size))
        for name in layer['panels']:
            selected = panel_lookup(index, name, part)
            union = ImageChops.lighter(union, read_mask(directory, selected['mask'], size))
        mask = ImageChops.multiply(mask, union)
    if layer.get('region'):
        try:
            region = index['regions'][layer['region']]
        except KeyError as error:
            raise ValueError(f'unknown material-map region: {layer["region"]}') from error
        if region['part'] != part:
            raise ValueError(f'{layer["region"]} is not in {part}')
        mask = ImageChops.multiply(mask, read_mask(directory, region['mask'], size))
    if layer.get('face_mask'):
        try:
            selection = index['face_masks'][layer['face_mask']]
        except KeyError as error:
            raise ValueError(f'unknown facing mask: {layer["face_mask"]}') from error
        if selection['part'] != part:
            raise ValueError(f'{layer["face_mask"]} is not in {part}')
        mask = ImageChops.multiply(mask, read_mask(directory, selection['mask'], size))
    if layer.get('polygon'):
        polygon = np.asarray(layer['polygon'], dtype=float)
        space = layer.get('space', 'panel')
        if space == 'panel':
            if panel is None: raise ValueError('panel-local polygon requires a panel')
            polygon = panel_points(panel, polygon)
        elif space == 'uv_px':
            polygon = polygon / index['size']
        else:
            raise ValueError(f'unknown polygon space: {space}')
        clip = Image.new('L', (size, size))
        ImageDraw.Draw(clip).polygon([tuple(point) for point in polygon*size], fill=255)
        mask = ImageChops.multiply(mask, clip)
    return mask


def base_layer(layer, part, index, directory, size, palette):
    mask = layer_mask(layer, part, index, directory, size)
    paint = Image.new('RGBA', (size, size), color(layer.get('color', '#000000'), palette))
    if 'gradient' in layer:
        gradient = layer['gradient']
        a, b = np.asarray(gradient.get('axis', [[0, 0], [1, 0]]), float)
        if layer.get('panel'):
            a, b = panel_points(panel_lookup(index, layer['panel'], part), [a, b])
        direction = b-a
        if np.dot(direction, direction) < 1e-12:
            raise ValueError('gradient axis must have length')
        box = mask.getbbox()
        if box:
            x0, y0, x1, y1 = box
            y, x = np.mgrid[y0:y1, x0:x1].astype(np.float32)/size
            t = np.clip(((x-a[0])*direction[0]+(y-a[1])*direction[1])/np.dot(direction, direction), 0, 1)
            start = np.array(color(gradient['from'], palette)); end = np.array(color(gradient['to'], palette))
            pixels = np.rint(start+(end-start)*t[..., None]).astype(np.uint8)
            paint.paste(Image.fromarray(pixels), box)
    paint.putalpha(ImageChops.multiply(paint.getchannel('A'), mask))
    return paint


def artwork_path(layer, logos):
    path = Path(layer['image'])
    if path.is_absolute() or '..' in path.parts:
        raise ValueError('artwork paths must be relative to the logo directory')
    variant = layer.get('variant')
    if variant and not variant.startswith('#'):
        path = path.with_name(path.stem+'-'+variant+path.suffix)
    return logos / path


def warp_sticker(art, grid, size, source_size):
    """Piecewise-affine 2D control mesh, as in an image editor's warp tool.

    Grid entries are destination UV pixels; source points form a regular grid.
    There are no physical positions, normals or projectors in this operation.
    """
    grid = np.asarray(grid, dtype=float)*size/source_size
    if grid.ndim != 3 or grid.shape[2] != 2 or min(grid.shape[:2]) < 2 or not np.isfinite(grid).all():
        raise ValueError('warp_uv_px must be a finite grid of at least 2 x 2 UV pixel positions')
    rows, columns = grid.shape[:2]
    y, x = np.mgrid[0:rows, 0:columns]
    source = np.stack((x/(columns-1)*art.width, y/(rows-1)*art.height), axis=-1)
    canvas = Image.new('RGBA', (size, size))
    for row in range(rows-1):
        for column in range(columns-1):
            for corners in [[(0, 0), (0, 1), (1, 0)], [(1, 1), (1, 0), (0, 1)]]:
                target = np.array([grid[row+r, column+c] for r, c in corners])
                origin = np.array([source[row+r, column+c] for r, c in corners])
                transform = np.column_stack((target, np.ones(3)))
                if abs(np.linalg.det(transform)) < 1e-7: continue
                inverse = np.linalg.solve(transform, origin).T
                lo = np.maximum(np.floor(target.min(axis=0)).astype(int)-1, 0)
                hi = np.minimum(np.ceil(target.max(axis=0)).astype(int)+1, size)
                if np.any(hi <= lo): continue
                affine = inverse.copy(); affine[:, 2] += inverse[:, :2]@lo
                patch = art.transform(tuple(hi-lo), Image.Transform.AFFINE, tuple(affine.ravel()), Image.Resampling.BICUBIC)
                clip = Image.new('L', patch.size)
                ImageDraw.Draw(clip).polygon([tuple(p-lo) for p in target], fill=255)
                patch.putalpha(ImageChops.multiply(patch.getchannel('A'), clip))
                canvas.alpha_composite(patch, tuple(lo))
    return canvas


def sticker_layer(layer, part, index, directory, logos, size):
    panel = panel_lookup(index, layer['panel'], part)
    if panel['shared_with'] and not layer.get('allow_shared', False):
        raise ValueError(f'{layer["panel"]} shares texels; explicitly allow_shared for symmetric artwork')
    with Image.open(artwork_path(layer, logos)) as source:
        art = source.convert('RGBA')
        box = art.getchannel('A').getbbox()
        if box is None: raise ValueError(f'empty artwork: {layer["image"]}')
        art = art.crop(box)
    if layer.get('variant', '').startswith('#'):
        tinted = Image.new('RGBA', art.size, layer['variant'])
        tinted.putalpha(art.getchannel('A')); art = tinted
    if 'warp_uv_px' in layer:
        image = warp_sticker(art, layer['warp_uv_px'], size, index['size'])
        image.putalpha(ImageChops.multiply(image.getchannel('A'), layer_mask(layer, part, index, directory, size, layer.get('clip', True))))
        return image
    frame = np.asarray(panel['frame_uv'])[:, 1:]
    width = layer.get('width', .25)
    dimensions = layer.get('size', [width, width*np.linalg.norm(frame[:, 0])/np.linalg.norm(frame[:, 1])*art.height/art.width])
    dimensions = np.array(dimensions, float)
    if np.any(dimensions <= 0): raise ValueError('sticker dimensions must be positive')
    centre = np.array(layer['position'], float)
    theta = np.deg2rad(layer.get('rotation', 0))
    rotation = np.array([[np.cos(theta), -np.sin(theta)], [np.sin(theta), np.cos(theta)]])
    # Rotate in texture-pixel metric, not a normalized panel square. Otherwise
    # a 90-degree turn on a long thin halo panel would stretch the logo.
    span = np.linalg.norm(frame, axis=0)
    corners = (np.array([[0, 0], [1, 0], [0, 1]])-.5)*dimensions*span
    local = corners @ rotation.T / span + centre
    uv = panel_points(panel, local, layer.get('flip', 'auto'))*size
    forward = np.column_stack(((uv[1]-uv[0])/art.width, (uv[2]-uv[0])/art.height))
    if abs(np.linalg.det(forward)) < 1e-12: raise ValueError('collapsed sticker transform')
    inverse = np.linalg.inv(forward)
    affine = np.column_stack((inverse, -inverse@uv[0]))
    image = art.transform((size, size), Image.Transform.AFFINE, tuple(affine.ravel()), Image.Resampling.BICUBIC)
    image.putalpha(ImageChops.multiply(image.getchannel('A'), layer_mask(layer, part, index, directory, size, layer.get('clip', True))))
    return image


def seam_dilate(image, coverage, padding=24):
    """Extend the nearest covered texel into gutters without changing UV0 pixels."""
    if padding < 0: raise ValueError('seam padding must be nonnegative')
    valid = np.asarray(coverage) > 127
    if not valid.any(): raise ValueError('part has no UV coverage')
    if not padding: return image.convert('RGB')
    distance, nearest = ndi.distance_transform_edt(~valid, return_indices=True)
    gutter = (~valid) & (distance <= padding)
    pixels = np.array(image.convert('RGB'))
    pixels[gutter] = pixels[nearest[0][gutter], nearest[1][gutter]]
    return Image.fromarray(pixels)


def png_bytes(image):
    stream = BytesIO(); image.save(stream, format='PNG')
    return stream.getvalue()


def write_ora(path, layers, size, metadata):
    """Stream layers to the ZIP, retaining only the composite and current layer."""
    merged = Image.new('RGBA', (size, size))
    root = ET.Element('image', w=str(size), h=str(size), name=path.stem, version='0.0.3')
    stack = ET.SubElement(root, 'stack')
    with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr('mimetype', 'image/openraster', compress_type=zipfile.ZIP_STORED)
        for number, (name, layer) in enumerate(layers):
            source = f'data/layer-{number:03}.png'
            archive.writestr(source, png_bytes(layer))
            merged.alpha_composite(layer)
            # OpenRaster serialises the topmost layer first.
            stack.insert(0, ET.Element('layer', name=name, src=source, x='0', y='0', opacity='1.0',
                                       visibility='visible', **{'composite-op': 'svg:src-over'}))
        archive.writestr('stack.xml', ET.tostring(root, encoding='utf-8', xml_declaration=True))
        archive.writestr('mergedimage.png', png_bytes(merged))
        thumbnail = merged.copy(); thumbnail.thumbnail((256, 256))
        archive.writestr('Thumbnails/thumbnail.png', png_bytes(thumbnail))
        archive.writestr('livery.json', json.dumps(metadata, indent=2)+'\n')
    return merged


def read_ora(path):
    """Read normal layers/groups with offsets, visibility and opacity, not stale mergedimage."""
    with zipfile.ZipFile(path) as archive:
        if sum(item.file_size for item in archive.infolist()) > 2*1024**3:
            raise ValueError('OpenRaster archive exceeds 2 GiB uncompressed')
        root = ET.fromstring(archive.read('stack.xml'))
        width, height = int(root.attrib['w']), int(root.attrib['h'])
        if width != height or not 1 <= width <= 4096:
            raise ValueError('OpenRaster image must be square and at most 4096')

        def render(node):
            op = node.get('composite-op', 'svg:src-over')
            if op != 'svg:src-over':
                raise ValueError(f'unsupported OpenRaster blend mode {op}; convert to normal layers')
            canvas = Image.new('RGBA', (width, height))
            if node.get('visibility') == 'hidden': return canvas
            if node.tag == 'layer':
                with Image.open(BytesIO(archive.read(node.attrib['src']))) as source:
                    layer = source.convert('RGBA')
                if max(layer.size) > 8192: raise ValueError('oversized OpenRaster layer')
                canvas.alpha_composite(layer, (int(node.get('x', 0)), int(node.get('y', 0))))
            else:
                for child in reversed(list(node)):
                    canvas.alpha_composite(render(child))
                offset = (int(node.get('x', 0)), int(node.get('y', 0)))
                if offset != (0, 0):
                    shifted = Image.new('RGBA', (width, height))
                    shifted.alpha_composite(canvas, offset); canvas = shifted
            opacity = float(node.get('opacity', '1'))
            if not 0 <= opacity <= 1: raise ValueError('OpenRaster opacity outside [0,1]')
            if opacity != 1:
                canvas.putalpha(canvas.getchannel('A').point(lambda value: round(value*opacity)))
            return canvas

        metadata = json.loads(archive.read('livery.json')) if 'livery.json' in archive.namelist() else None
        return render(root.find('stack')), metadata


def compose(layout_path, templates, logos, output, size=None):
    """Publish only after every paint part and editable document has composed."""
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.flat-livery-', dir=output.parent) as temporary:
        stage = Path(temporary)
        manifest = compose_parts(layout_path, templates, logos, stage, size)
        output.mkdir(parents=True, exist_ok=True)
        # Manifest last: its presence never advertises an unfinished new set.
        for path in sorted(stage.iterdir(), key=lambda path: path.name == 'native-manifest.json'):
            os.replace(path, output/path.name)
    return manifest


def compose_parts(layout_path, templates, logos, output, size=None):
    layout = json.loads(layout_path.read_text())
    if layout.get('schema') != 1 or not layout.get('team'):
        raise ValueError('layout needs schema 1 and a team')
    if set(layout['parts']) != set(PARTS): raise ValueError('layout must describe all nine parts')
    index = load_index(templates)
    if layout.get('source_sha256') != index['source_sha256']:
        raise ValueError('layout/source blend fingerprint mismatch; audit panels before updating it')
    size = size or index['size']
    if not 1 <= size <= 4096: raise ValueError('size must be in [1,4096]')
    # Resolve every panel and artwork before replacing any previous output.
    for part, spec in layout['parts'].items():
        for layer in spec.get('base', []) + spec.get('stickers', []):
            if layer.get('panel'): panel_lookup(index, layer['panel'], part)
            for name in layer.get('panels', []): panel_lookup(index, name, part)
            for key, collection in [('region', 'regions'), ('face_mask', 'face_masks')]:
                if layer.get(key):
                    selection = index.get(collection, {}).get(layer[key])
                    if selection is None or selection['part'] != part:
                        raise ValueError(f'unknown {part} {key}: {layer[key]}')
        for layer in spec.get('stickers', []):
            panel = panel_lookup(index, layer['panel'], part)
            if panel['shared_with'] and not layer.get('allow_shared', False):
                raise ValueError(f'{layer["panel"]} shares texels; explicitly allow_shared for symmetric artwork')
            path = artwork_path(layer, logos)
            if not path.is_file(): raise FileNotFoundError(path)
    output.mkdir(parents=True, exist_ok=True)
    manifest = {'schema': 1, 'team': layout['team'], 'uv_channel': 0, 'size': size,
                'parts': list(PARTS), 'authoring': 'flat-uv0', 'source_sha256': index['source_sha256'],
                'objects': index['objects'], 'layout_sha256': sha256(layout_path),
                'panel_index_sha256': sha256(templates/'panels.json'),
                'compositor_sha256': sha256(Path(__file__)), 'artwork_sha256': {}, 'textures_sha256': {}}
    ora_index = {}
    for part in PARTS:
        spec = layout['parts'][part]
        def layers():
            yield 'Base paint', Image.new('RGBA', (size, size), color(spec['color'], layout.get('palette', {})))
            for number, layer in enumerate(spec.get('base', [])):
                yield layer.get('name', f'Base region {number+1}'), base_layer(layer, part, index, templates, size, layout.get('palette', {}))
            for number, layer in enumerate(spec.get('stickers', [])):
                path = artwork_path(layer, logos)
                manifest['artwork_sha256'][str(path.name)] = sha256(path)
                yield layer.get('name', f'Sticker {number+1}: {layer["image"]}'), sticker_layer(layer, part, index, templates, logos, size)
        ora_index[part] = {'schema': 1, 'team': layout['team'], 'part': part, 'uv_channel': 0,
                           'panel_index_sha256': manifest['panel_index_sha256'], 'source_sha256': index['source_sha256']}
        merged = write_ora(output/f'{part}.ora', layers(), size, ora_index[part])
        coverage = read_mask(templates, f'{part}-coverage.png', size)
        seam_dilate(merged, coverage, layout.get('padding', 24)*size/index['size']).save(output/f'{part}.png')
        manifest['textures_sha256'][part] = sha256(output/f'{part}.png')
        count = 1+len(spec.get('base', []))+len(spec.get('stickers', []))
        print(f'COMPOSE_UV0 {part}: {count} editable layers, {size}px', flush=True)
    (output/'native-manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    # Krita/GIMP may discard nonstandard ZIP entries when saving an ORA. Keep
    # the same contract beside the documents so those saves remain importable.
    (output/'ora-index.json').write_text(json.dumps(ora_index, indent=2)+'\n')
    return manifest


def reimport_oras(source, templates, output, padding=24):
    index = load_index(templates)
    documents = []
    team = None
    sidecar_path = source/'ora-index.json'
    sidecar = json.loads(sidecar_path.read_text()) if sidecar_path.exists() else {}
    # Validate the whole set before replacing PNGs or their manifest.
    for part in PARTS:
        path = source/f'{part}.ora'
        image, metadata = read_ora(path)
        metadata = metadata or sidecar.get(part)
        if metadata is None:
            raise ValueError(f'{path}: missing livery.json and ora-index.json metadata')
        if (metadata.get('schema') != 1 or metadata.get('part') != part or metadata.get('uv_channel') != 0
                or metadata.get('source_sha256') != index['source_sha256']
                or metadata.get('panel_index_sha256') != sha256(templates/'panels.json')
                or image.size != (index['size'], index['size'])):
            raise ValueError(f'{path}: incompatible panel index, dimensions or native metadata')
        team = team or metadata['team']
        if metadata['team'] != team: raise ValueError('OpenRaster documents must describe one team')
        documents.append((part, path))
    output.mkdir(parents=True, exist_ok=True)
    manifest = {'schema': 1, 'team': team, 'uv_channel': 0, 'size': index['size'], 'parts': list(PARTS),
                'authoring': 'flat-uv0-openraster', 'source_sha256': index['source_sha256'],
                'objects': index['objects'], 'panel_index_sha256': sha256(templates/'panels.json'),
                'ora_sha256': {}, 'textures_sha256': {}}
    for part, path in documents:
        image, _ = read_ora(path)
        seam_dilate(image, read_mask(templates, f'{part}-coverage.png', index['size']), padding).save(output/f'{part}.png')
        manifest['ora_sha256'][part] = sha256(path)
        manifest['textures_sha256'][part] = sha256(output/f'{part}.png')
    (output/'native-manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('layout', type=Path, nargs='?')
    parser.add_argument('--templates', type=Path, default=DEFAULT_TEMPLATES)
    parser.add_argument('--logos', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--reimport', type=Path, metavar='ORA_DIRECTORY')
    parser.add_argument('--size', type=int, help='smaller diagnostic output; installer requires 4096')
    parser.add_argument('--padding', type=int, default=24, help='reimport seam dilation pixels')
    args = parser.parse_args()
    if args.reimport:
        if args.layout or args.logos or args.size: parser.error('--reimport is exclusive with layout/--logos/--size')
        reimport_oras(args.reimport, args.templates, args.out, args.padding)
    else:
        if not args.layout or not args.logos: parser.error('composition needs a layout and --logos')
        compose(args.layout, args.templates, args.logos, args.out, args.size)


if __name__ == '__main__':
    main()
