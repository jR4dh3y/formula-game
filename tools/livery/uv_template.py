"""Export untouched FP04RB UV0 and generate labelled 4096 paint templates.

Run with python3 -I. The exporter opens Blender with autoexec disabled under
the shared Blender lock. No scripts from the source blend or Downloads run.
The index, masks and triangles are independent of every team's artwork.
"""
import argparse
from collections import Counter, defaultdict
import json
import math
from pathlib import Path
import subprocess
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy import ndimage as ndi
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components

sys.path.insert(0, str(Path(__file__).resolve().parent))
from uv_layout import DEFAULT_TEMPLATES, PARTS, REPO, sha256


def export_meshes(output):
    import bpy
    sys.path.insert(0, str(REPO / 'tools/car'))
    from source_materials import PARTS as SOURCE_PARTS

    output.mkdir(parents=True, exist_ok=True)
    data = defaultdict(lambda: defaultdict(list))
    objects = []
    chart_offset = Counter()
    deps = bpy.context.evaluated_depsgraph_get()
    # Source pose is slightly yawed. This canonical frame is geometry metadata,
    # not a paint projector. UVs are never changed.
    yaw = -.007
    rotation = np.array([[math.cos(yaw), math.sin(yaw), 0],
                         [-math.sin(yaw), math.cos(yaw), 0], [0, 0, 1]])
    sources = sorted(bpy.data.collections['fp04_tr_model'].all_objects, key=lambda o: o.name)
    for source in sources:
        if source.type != 'MESH' or not source.data.polygons or source.hide_render:
            continue
        part = SOURCE_PARTS.get(source.name, '').removesuffix('_stickers')
        if '_wheel_cover_' in source.name:
            part = 'front_covers' if 'front' in source.name else 'rear_covers'
        if part not in PARTS:
            continue
        mesh = bpy.data.meshes.new_from_object(source.evaluated_get(deps),
                                               preserve_all_data_layers=True, depsgraph=deps)
        mesh.calc_loop_triangles()
        uv = np.empty((len(mesh.loops), 2), np.float32)
        mesh.uv_layers[0].data.foreach_get('uv', uv.ravel())
        objects.append({'object': source.name, 'part': part, 'loops': len(mesh.loops),
                        'uv0_sha256': __import__('hashlib').sha256(uv.tobytes()).hexdigest()})
        vertex_ids = np.empty(len(mesh.loops), np.int32)
        mesh.loops.foreach_get('vertex_index', vertex_ids)
        triangles = np.array([tri.loops[:] for tri in mesh.loop_triangles])
        _, keys = np.unique(np.column_stack((vertex_ids, np.round(uv, 6))), axis=0, return_inverse=True)
        a, b = keys[triangles].ravel(), np.roll(keys[triangles], 1, axis=1).ravel()
        graph = coo_matrix((np.ones(len(a)), (a, b)), shape=(keys.max()+1,)*2).tocsr()
        count, labels = connected_components(graph, directed=False)
        charts = labels[keys[triangles[:, 0]]] + chart_offset[part]
        chart_offset[part] += count
        mesh.transform(source.matrix_world)
        vertices = np.empty((len(mesh.vertices), 3), np.float32)
        mesh.vertices.foreach_get('co', vertices.ravel())
        vertices = (vertices - [.045, 0, 0]) @ rotation.T
        xyz = vertices[vertex_ids[triangles]]
        # Image coordinates throughout the authoring format: origin top left.
        uv[:, 1] = 1 - uv[:, 1]
        for key, value in [('uv', uv[triangles]), ('xyz', xyz), ('chart', charts)]:
            data[part][key].append(value)
        bpy.data.meshes.remove(mesh)
    if set(data) != set(PARTS):
        raise ValueError(f'missing paint parts: {set(PARTS)-set(data)}')
    for part, arrays in data.items():
        np.savez_compressed(output / f'{part}-mesh.npz', **{k: np.concatenate(v) for k, v in arrays.items()})
    (output / 'objects.json').write_text(json.dumps(objects, indent=2)+'\n')
    (output / 'mesh-source.json').write_text(json.dumps({'source_sha256': sha256(Path(bpy.data.filepath))})+'\n')


def triangle_pixels(triangle, size):
    """Sample pixel centres, with half-open edges avoiding adjacent-face overlaps."""
    p = np.asarray(triangle)*size
    lo = np.maximum(np.floor(p.min(axis=0)).astype(int), 0)
    hi = np.minimum(np.ceil(p.max(axis=0)).astype(int), size)
    if np.any(hi <= lo):
        return None
    a, b, c = p
    edge1, edge2 = b-a, c-a
    determinant = np.linalg.det(np.array([edge1, edge2]))
    if abs(determinant) < 1e-10:
        return None
    y, x = np.mgrid[lo[1]:hi[1], lo[0]:hi[0]]
    dx, dy = x+.5-a[0], y+.5-a[1]
    u = (dx*edge2[1]-dy*edge2[0])/determinant
    v = (dy*edge1[0]-dx*edge1[1])/determinant
    inside = (u > 1e-7) & (v > 1e-7) & (u+v < 1-1e-7)
    return y[inside], x[inside]


def boundary_polygons(triangles):
    """Retain actual concave boundaries and holes, rather than a convex hull."""
    vertices, inverse = np.unique(np.round(triangles.reshape(-1, 2), 6), axis=0, return_inverse=True)
    ids = inverse.reshape(-1, 3)
    edges = np.concatenate((ids[:, [0, 1]], ids[:, [1, 2]], ids[:, [2, 0]]))
    edges.sort(axis=1)
    unique, counts = np.unique(edges, axis=0, return_counts=True)
    adjacency = defaultdict(set)
    for a, b in unique[counts == 1]:
        if a == b: continue
        adjacency[int(a)].add(int(b)); adjacency[int(b)].add(int(a))
    paths = []
    while adjacency:
        start = next(iter(adjacency)); current = start; path = [start]
        while current in adjacency:
            other = next(iter(adjacency[current]))
            adjacency[current].remove(other); adjacency[other].remove(current)
            if not adjacency[current]: del adjacency[current]
            if other in adjacency and not adjacency[other]: del adjacency[other]
            path.append(other); current = other
            if current == start: break
        if len(path) > 3:
            # Evaluated subdivision can make long boundaries. RDP keeps the
            # index small while masks retain every triangle exactly.
            paths.append(simplify(vertices[path], .35/4096).tolist())
    return paths


def simplify(points, tolerance):
    if len(points) <= 3:
        return points
    direction = points[-1]-points[0]
    if np.linalg.norm(direction) < 1e-12:
        middle = len(points)//2
        return np.concatenate((simplify(points[:middle+1], tolerance)[:-1], simplify(points[middle:], tolerance)))
    delta = points-points[0]
    distance = np.abs(direction[0]*delta[:, 1]-direction[1]*delta[:, 0]) / np.linalg.norm(direction)
    farthest = int(distance.argmax())
    if distance[farthest] <= tolerance:
        return points[[0, -1]]
    return np.concatenate((simplify(points[:farthest+1], tolerance)[:-1], simplify(points[farthest:], tolerance)))


def view_axes(normal, side):
    axis = int(np.abs(normal).argmax())
    if axis == 0:
        return np.array([0, side, 0]), np.array([0, 0, -1]), axis
    if axis == 1:
        return np.array([-np.sign(normal[1]) or 1, 0, 0]), np.array([0, 0, -1]), axis
    return np.array([1, 0, 0]), np.array([0, -1, 0]), axis


def split_panels(part, xyz, charts, normals):
    centre = xyz.mean(axis=1)
    result = []
    for chart in sorted(np.unique(charts)):
        ids = np.flatnonzero(charts == chart)
        p = centre[ids]; lo = xyz[ids].min(axis=(0, 1)); hi = xyz[ids].max(axis=(0, 1))
        side = 'left' if p[:, 0].mean() > 0 else 'right'
        if part == 'main_body' and hi[1]-lo[1] > 4 and hi[2] > 1:
            # The two source side-view islands contain the complete cover,
            # sidepod and chassis. Named subpanels share this planar layout.
            cover = (p[:, 1] > 2.25) & (p[:, 2] > .64)
            cowl = cover & (normals[ids, 2] > np.abs(normals[ids, 0]))
            sidepod = (p[:, 1] > 1.25) & ~cover
            nose = p[:, 1] < 1.25
            for name, select in [(f'engine-cover-{side}', cover & ~cowl), (f'top-cowl-{side}', cowl), (f'sidepod-{side}', sidepod), (f'nose-side-{side}', nose)]:
                result.append((name, ids[select], int(chart)))
        elif part == 'main_body' and hi[1]-lo[1] > 3 and hi[2] > .7 and lo[1] < 0:
            nose = p[:, 1] < 1.2
            rim = (p[:, 1] >= 1.2)
            result.extend([('nose-top', ids[nose], int(chart)), ('cockpit-rim', ids[rim], int(chart))])
        elif part == 'main_body' and hi[2] > .85 and hi[1]-lo[1] > .6:
            name = f'halo-{side}' if hi[1]-lo[1] > 1 else 'halo-front'
            result.append((name, ids, int(chart)))
        elif part in ('front_wing', 'rear_wing'):
            if hi[2]-lo[2] > .25:
                normal_x = np.average(normals[ids, 0])
                outer = normal_x * (1 if side == 'left' else -1) > 0
                name = f'{part.replace("_", "-")}-endplate-{side}-{"outer" if outer else "inner"}'
            else:
                facing = 'top' if np.mean(normals[ids, 2]) > 0 else 'bottom'
                if part == 'front_wing':
                    element = int(np.searchsorted([.18, .21, .27], hi[2]))+1
                    if hi[0]-lo[0] < .05:
                        name = f'front-wing-element-{element:02}-edge-{side}-chart-{chart:02}'
                    else:
                        if abs(np.mean(normals[ids, 2])) < .1: facing = 'shared-top-bottom'
                        suffix = '' if lo[0] < -.5 and hi[0] > .5 else '-'+side
                        name = f'front-wing-element-{element:02}{suffix}-{facing}'
                elif hi[1] < 4.58:
                    name = f'rear-wing-main-{facing}'
                elif hi[0]-lo[0] < .2:
                    name = f'rear-wing-drs-tip-{side}-{facing}'
                else:
                    name = 'rear-wing-drs-shared-top-bottom'
            result.append((name, ids, int(chart)))
        else:
            result.append((f'{part.replace("_", "-")}-{side}-{chart:02}', ids, int(chart)))
    # Tiny chart names remain explicit and unique instead of pretending each
    # unlabeled hardware sliver is a recognised anatomical surface.
    seen = Counter()
    unique = []
    for name, ids, chart in result:
        if not len(ids): continue
        seen[name] += 1
        if seen[name] > 1: name += f'-chart-{chart:02}'
        unique.append((name, ids, chart))
    return unique


def panel_record(part, name, ids, chart, uv, xyz, normals, areas):
    weight = areas[ids]
    normal = np.average(normals[ids], axis=0, weights=weight)
    # Mean normals cancel on closed/two-sided charts; use total projected area.
    axis = int(np.average(np.abs(normals[ids]), axis=0, weights=weight).argmax())
    if abs(normal[axis]) < .01: normal[axis] = 1
    normal[np.arange(3) != axis] = 0
    side = 1 if xyz[ids, :, 0].mean() > 0 else -1
    right, down, axis = view_axes(normal, side)
    points = xyz[ids].reshape(-1, 3)
    target = uv[ids].reshape(-1, 2)
    source = np.column_stack((points @ right, points @ down, np.ones(len(points))))
    fit = np.linalg.lstsq(source, target, rcond=None)[0]
    r = fit[0]; r /= max(np.linalg.norm(r), 1e-12)
    d = np.array([-r[1], r[0]])
    if np.dot(d, fit[1]) < 0: d *= -1
    oriented = target @ np.stack((r, d), axis=1)
    lo, hi = oriented.min(axis=0), oriented.max(axis=0)
    frame = np.column_stack((r*lo[0]+d*lo[1], r*(hi[0]-lo[0]), d*(hi[1]-lo[1])))
    front_fit = np.linalg.lstsq(np.column_stack((points, np.ones(len(points)))), target, rcond=None)[0]
    front = -front_fit[1]
    front /= max(np.linalg.norm(front), 1e-12)
    return {'part': part, 'chart': chart, 'bbox_uv': [target.min(axis=0).tolist(), target.max(axis=0).tolist()],
            'polygons_uv': boundary_polygons(uv[ids]), 'frame_uv': frame.tolist(),
            'orientation_degrees': math.degrees(math.atan2(r[1], r[0])),
            'mirrored': bool(np.linalg.det(frame[:, 1:]) < 0), 'front_uv': front.tolist(),
            'projection_axis': 'xyz'[axis], 'mean_projection_cosine': float(np.average(np.abs(normals[ids, axis]), weights=weight)),
            'world_bbox': [points.min(axis=0).tolist(), points.max(axis=0).tolist()],
            'mask': f'masks/{name}.png', 'shared_with': []}


def write_index(directory, index):
    (directory / 'panels.json').write_text(json.dumps(index, indent=2)+'\n')
    compact = {}
    for name, panel in index['panels'].items():
        def area(polygon):
            p = np.asarray(polygon)
            return abs(np.sum(p[:, 0]*np.roll(p[:, 1], -1)-p[:, 1]*np.roll(p[:, 0], -1)))
        polygon = max(panel['polygons_uv'], key=area, default=[])
        compact[name] = {key: panel[key] for key in ('part', 'bbox_uv', 'orientation_degrees',
                                                    'mirrored', 'front_uv', 'shared_with', 'mask')}
        compact[name]['polygon_uv'] = np.asarray(polygon).round(5).tolist()
    (directory / 'panel-index.json').write_text(json.dumps(compact, separators=(',', ':'))+'\n')


def face_masks(part, uv, normals, directory, size):
    """Geometric viewing masks, exported once as flat 2D authoring selections."""
    selections = {
        'left-side': (normals[:, 0] > .7) & (np.abs(normals[:, 2]) < .6),
        'right-side': (normals[:, 0] < -.7) & (np.abs(normals[:, 2]) < .6),
        'top': normals[:, 2] > .7,
        'bottom': normals[:, 2] < -.7,
        'front': normals[:, 1] < -.7,
        'rear': normals[:, 1] > .7,
    }
    result = {}
    for name, selected in selections.items():
        mask = Image.new('L', (size, size)); draw = ImageDraw.Draw(mask)
        for triangle in uv[selected]:
            draw.polygon([tuple(point) for point in triangle*size], fill=255)
        path = f'masks/{part}-{name}-facing.png'
        mask.save(directory/path)
        result[f'{part}:{name}'] = {'part': part, 'mask': path}
    return result


def generate(directory, material_maps, size, source):
    source_hash = sha256(source)
    if json.loads((directory / 'mesh-source.json').read_text())['source_sha256'] != source_hash:
        raise ValueError('cached mesh/source fingerprint mismatch; export the meshes again')
    index = {'schema': 1, 'uv_channel': 0, 'size': size,
             'source_sha256': source_hash,
             'objects': json.loads((directory / 'objects.json').read_text()),
             'panels': {}, 'regions': {}, 'face_masks': {}, 'parts': list(PARTS)}
    (directory / 'masks').mkdir(exist_ok=True)
    font = ImageFont.truetype('/usr/share/fonts/TTF/DejaVuSans.ttf', 26)
    small = ImageFont.truetype('/usr/share/fonts/TTF/DejaVuSans.ttf', 19)
    for part in PARTS:
        mesh = np.load(directory / f'{part}-mesh.npz', allow_pickle=False)
        uv, xyz, charts = mesh['uv'], mesh['xyz'], mesh['chart']
        cross = np.cross(xyz[:, 1]-xyz[:, 0], xyz[:, 2]-xyz[:, 0])
        areas = np.linalg.norm(cross, axis=1)
        normals = cross / np.maximum(areas[:, None], 1e-16)
        index['face_masks'].update(face_masks(part, uv, normals, directory, size))
        panels = split_panels(part, xyz, charts, normals)
        diagnostic_size = 1024
        owner = np.full((diagnostic_size,)*2, -1, np.int32)
        overlap = Counter()
        stretch = np.ones((diagnostic_size,)*2, np.float32)
        coverage = Image.new('L', (size, size)); cover_draw = ImageDraw.Draw(coverage)
        wire = Image.new('RGBA', (size, size)); wire_draw = ImageDraw.Draw(wire)
        records = []
        for ordinal, (name, ids, chart) in enumerate(panels):
            record = panel_record(part, name, ids, chart, uv, xyz, normals, np.maximum(areas, 1e-16))
            record['diagnostic_id'] = ordinal
            mask = Image.new('L', (size, size)); draw = ImageDraw.Draw(mask)
            axis = 'xyz'.index(record['projection_axis'])
            for i in ids:
                polygon = [tuple(p) for p in uv[i]*size]
                draw.polygon(polygon, fill=255); cover_draw.polygon(polygon, fill=255)
                # Subdivision wire is useful at 4096; modest alpha keeps tints readable.
                wire_draw.line(polygon+[polygon[0]], fill=(240, 240, 240, 60), width=1)
                pixels = triangle_pixels(uv[i], diagnostic_size)
                if pixels is not None:
                    y, x = pixels
                    previous = owner[y, x]
                    same = int(np.count_nonzero(previous == ordinal))
                    if same: overlap[(ordinal, ordinal)] += same
                    for other, count in zip(*np.unique(previous[(previous >= 0) & (previous != ordinal)], return_counts=True)):
                        overlap[tuple(sorted((ordinal, int(other))))] += int(count)
                    owner[y, x] = ordinal
                    stretch[y, x] = np.minimum(stretch[y, x], abs(normals[i, axis]))
            mask.save(directory / record['mask'])
            records.append((name, record))
        for (a, b), count in overlap.items():
            if count < 5: continue
            for current, other in ([(a, b)] if a == b else [(a, b), (b, a)]):
                records[current][1]['shared_with'].append({'panel': records[other][0], 'sample_texels': count})
        coverage.save(directory / f'{part}-coverage.png')
        map_name = 'body' if part == 'main_body' else part
        path = material_maps / f'material_map_{map_name}.png'
        if not path.exists(): raise FileNotFoundError(path)
        with Image.open(path) as original:
            tint = original.convert('RGB').resize((size, size), Image.Resampling.NEAREST)
        tint.save(directory / f'{part}-material-map.png')
        # Region names identify exact source swatches, never guessed anatomy.
        pixels = np.asarray(tint.resize((512, 512), Image.Resampling.NEAREST))
        colors, counts = np.unique(pixels.reshape(-1, 3)//16*16+8, axis=0, return_counts=True)
        region_list = []
        full = np.asarray(tint, dtype=np.int16)
        for color, count in sorted(zip(colors, counts), key=lambda pair: -pair[1]):
            if count < 150 or int(color.max()-color.min()) < 35: continue
            selected = np.max(np.abs(full-color.astype(np.int16)), axis=2) <= 15
            region_name = f'{part}:material-{len(region_list)+1:02}'
            mask_path = f'masks/{part}-material-{len(region_list)+1:02}.png'
            Image.fromarray(selected.astype(np.uint8)*255).save(directory / mask_path)
            index['regions'][region_name] = {'part': part, 'color': '#'+''.join(f'{int(c):02x}' for c in color), 'mask': mask_path}
            region_list.append(region_name)
        # Red = grazing/distorted, green = near-planar, purple = no reliable UV area.
        heat = np.zeros((diagnostic_size, diagnostic_size, 4), np.uint8)
        heat[..., 0] = (255*(1-stretch)).astype(np.uint8)
        heat[..., 1] = (220*stretch).astype(np.uint8)
        heat[..., 2] = 40; heat[..., 3] = (owner >= 0)*130
        Image.fromarray(heat).save(directory / f'{part}-stretch.png')
        template = Image.blend(Image.new('RGB', (size, size), '#20252b'), tint, .38).convert('RGBA')
        template.alpha_composite(Image.fromarray(heat).resize((size, size), Image.Resampling.NEAREST))
        template.alpha_composite(wire)
        d = ImageDraw.Draw(template)
        for name, record in records:
            mask = Image.open(directory / record['mask'])
            # Put labels inside the largest interior region, avoiding island gutters.
            distance = ndi.distance_transform_edt(np.asarray(mask.resize((512, 512))) > 127)
            y, x = np.unravel_index(distance.argmax(), distance.shape)
            x, y = x*size/512, y*size/512
            text = name + (' | MIRRORED' if record['mirrored'] else ' | upright')
            if record['shared_with']: text += ' | SHARED!'
            bbox = d.textbbox((0, 0), text, font=small)
            if distance.max() > 1.5:
                tx = max(6, min(x-bbox[2]/2, size-bbox[2]-6)); ty = max(100, min(y, size-55))
                d.rectangle((tx-4, ty-3, tx+bbox[2]+4, ty+25), fill=(0, 0, 0, 210))
                d.text((tx, ty), text, font=small, fill='white')
                direction = np.array(record['front_uv'])*65
                end = np.array([x, y])-np.array([0, 35])+direction
                start = np.array([x, y])-np.array([0, 35])
                d.line([tuple(start), tuple(end)], fill='#ffff00', width=5)
                perpendicular = np.array([-direction[1], direction[0]])*.15
                d.polygon([tuple(end), tuple(end-direction*.25+perpendicular), tuple(end-direction*.25-perpendicular)], fill='#ffff00')
                d.text(tuple(start+[8, -25]), 'FRONT', font=small, fill='#ffff00')
            index['panels'][name] = record
        d.rectangle((0, 0, size, 96), fill='#111111')
        d.text((18, 8), f'{part} | original UV0 | {size}px | FRONT arrows | MIRRORED = reflected UVs | SHARED = asymmetric art impossible', font=font, fill='white')
        d.text((18, 48), 'Material-map tint + UV wire | stretch: GREEN planar, RED grazing | tiny islands: see panels.json', font=font, fill='white')
        template.convert('RGB').save(directory / f'{part}.png')
        print(f'UV_TEMPLATE {part}: {len(records)} panels, {len(overlap)} overlap pairs, {len(region_list)} material swatches', flush=True)
    write_index(directory, index)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, default=DEFAULT_TEMPLATES)
    parser.add_argument('--source', type=Path, default=REPO / 'raw/gen22/fp04rb_new_rig.blend')
    parser.add_argument('--material-maps', type=Path, default=Path.home()/'Downloads/fp04rb/material_maps')
    parser.add_argument('--size', type=int, default=4096)
    parser.add_argument('--reuse-meshes', action='store_true')
    parser.add_argument('--export', action='store_true', help=argparse.SUPPRESS)
    argv = sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else None
    args = parser.parse_args(argv)
    if args.export:
        export_meshes(args.out)
        return
    if args.size != 4096: parser.error('native templates must be 4096 square')
    args.out.mkdir(parents=True, exist_ok=True)
    if not args.reuse_meshes:
        subprocess.run(['flock', '/tmp/f1-blender.lock', '/usr/sbin/blender', '-b', '--factory-startup',
                        '--disable-autoexec', str(args.source.resolve()), '--python-exit-code', '1',
                        '--python', str(Path(__file__).resolve()), '--', '--export', '--out', str(args.out.resolve())], check=True)
    generate(args.out, args.material_maps, args.size, args.source)


if __name__ == '__main__':
    main()
