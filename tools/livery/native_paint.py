"""Author 3D paint/decal shaders and bake emission into FP04RB's untouched UV0.

blender -b --factory-startup --disable-autoexec SOURCE.blend --python-exit-code 1 \
  --python tools/livery/native_paint.py -- SPEC.json --logos LOGOS --out OUTPUT

Positions are metres in the source frame: X left, Y rearward, Z up. An empty
removes the source's posed yaw. Decal width is physical; height follows the
transparent artwork's cropped aspect ratio. No template sheet is read.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

import bpy
import numpy as np
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / 'tools/car'))
from source_materials import PARTS


def linear(hex_color):
    """Convert authored sRGB swatches to shader linear values once."""
    rgb = [int(hex_color.lstrip('#')[i:i+2], 16) / 255 for i in (0, 2, 4)]
    return tuple(v / 12.92 if v <= .04045 else ((v + .055) / 1.055) ** 2.4 for v in rgb) + (1,)


class Paint:
    """Small node builder for physical region masks and planar decal projectors."""
    def __init__(self, part, spec, frame, logos):
        self.material = bpy.data.materials.new('Native paint ' + part)
        self.material.use_nodes = True
        self.nodes = self.material.node_tree.nodes
        self.links = self.material.node_tree.links
        self.nodes.clear()
        coord = self.nodes.new('ShaderNodeTexCoord')
        coord.object = frame
        split = self.nodes.new('ShaderNodeSeparateXYZ')
        self.links.new(coord.outputs['Object'], split.inputs[0])
        self.position = coord.outputs['Object']
        self.axes = [split.outputs[i] for i in range(3)]
        panel = self.nodes.new('ShaderNodeAttribute')
        panel.attribute_name = 'native_panel'
        self.panel = panel.outputs['Fac']
        self.normal = self.nodes.new('ShaderNodeNewGeometry').outputs['Normal']
        palette = spec['palette']
        self.color = linear(palette[spec['parts'][part]['base']])
        for region in spec['parts'][part].get('regions', []):
            self.color = self.mix(self.mask(region['where']), self.color, linear(palette[region['color']]))
        for decal in spec.get('decals', []):
            if decal['part'] != part:
                continue
            path = logos / decal['image']
            image = bpy.data.images.load(str(path.resolve()), check_existing=True)
            image.colorspace_settings.name = 'sRGB'
            image.alpha_mode = 'STRAIGHT'
            u, v = np.array(decal['right'], float), np.array(decal['up'], float)
            if abs(np.linalg.norm(u) - 1) > 1e-5 or abs(np.linalg.norm(v) - 1) > 1e-5 or abs(u @ v) > 1e-5:
                raise ValueError(f'decal axes must be orthonormal: {decal}')
            if not image.size[0] or not image.size[1]:
                raise ValueError(f'unreadable artwork: {path}')
            width = decal['width']
            height = width * image.size[1] / image.size[0]
            origin = np.array(decal['center'])
            curved = self.curved_projection(decal['path'], decal['up']) if 'path' in decal else None
            uv = self.nodes.new('ShaderNodeCombineXYZ')
            for axis, direction, extent in [(0, u, width), (1, v, height)]:
                if curved:
                    value = self.math('ADD', self.math('MULTIPLY', curved[axis], 1 / extent), .5)
                else:
                    dot = self.vector_dot(self.position, direction)
                    value = self.math('ADD', self.math('MULTIPLY', dot, 1 / extent), .5 - float(origin @ direction) / extent)
                self.links.new(value, uv.inputs[axis])
            tex = self.nodes.new('ShaderNodeTexImage')
            tex.image, tex.extension, tex.interpolation = image, 'CLIP', 'Linear'
            self.links.new(uv.outputs[0], tex.inputs['Vector'])
            mask = self.math('MULTIPLY', tex.outputs['Alpha'], self.mask(decal.get('where', {})))
            # Prevent the same projector printing through the opposite side/back.
            if 'facing' in decal:
                dot = self.vector_dot(self.normal, decal['facing'])
                mask = self.math('MULTIPLY', mask, self.math('GREATER_THAN', dot, decal.get('normal_min', .1)))
            self.color = self.mix(mask, self.color, tex.outputs['Color'])
        emission = self.nodes.new('ShaderNodeEmission')
        self.input(emission.inputs['Color'], self.color)
        output = self.nodes.new('ShaderNodeOutputMaterial')
        self.links.new(emission.outputs[0], output.inputs['Surface'])

    def input(self, socket, value):
        if isinstance(value, bpy.types.NodeSocket):
            self.links.new(value, socket)
        else:
            socket.default_value = value

    def math(self, operation, a, b=0):
        node = self.nodes.new('ShaderNodeMath')
        node.operation = operation
        self.input(node.inputs[0], a)
        self.input(node.inputs[1], b)
        return node.outputs[0]

    def vector_dot(self, vector, direction):
        node = self.nodes.new('ShaderNodeVectorMath')
        node.operation = 'DOT_PRODUCT'
        self.links.new(vector, node.inputs[0])
        self.input(node.inputs[1], direction)
        return node.outputs['Value']

    def curved_projection(self, points, up):
        """Unroll a measured 3D decal centreline by arc length and transverse offset."""
        points = np.asarray(points, dtype=float)
        lengths = np.linalg.norm(np.diff(points, axis=0), axis=1)
        if len(lengths) == 0 or np.any(lengths <= 0):
            raise ValueError('decal path needs distinct consecutive points')
        best, along, across, station = 1e20, 0, 0, 0
        for start, end, length in zip(points, points[1:], lengths):
            tangent = (end - start) / length
            transverse = np.asarray(up) - tangent * (np.asarray(up) @ tangent)
            if np.linalg.norm(transverse) < 1e-6:
                raise ValueError('decal up axis must not be parallel to its path')
            transverse /= np.linalg.norm(transverse)
            offset = self.nodes.new('ShaderNodeVectorMath')
            offset.operation = 'SUBTRACT'
            self.links.new(self.position, offset.inputs[0])
            offset.inputs[1].default_value = start
            projection = self.vector_dot(offset.outputs['Vector'], tangent)
            t = self.math('MINIMUM', self.math('MAXIMUM', projection, 0), float(length))
            # Closest segment wins, keeping adjacent sections continuous at joints.
            norm = self.vector_dot(offset.outputs['Vector'], offset.outputs['Vector'])
            twice_projection = self.math('MULTIPLY', projection, 2)
            reduction = self.math('MULTIPLY', t, self.math('SUBTRACT', twice_projection, t))
            distance = self.math('SUBTRACT', norm, reduction)
            choose = self.math('LESS_THAN', distance, best)
            candidate = self.math('ADD', projection, float(station - lengths.sum()/2))
            along = self.math('ADD', along, self.math('MULTIPLY', choose, self.math('SUBTRACT', candidate, along)))
            lateral = self.vector_dot(offset.outputs['Vector'], transverse)
            across = self.math('ADD', across, self.math('MULTIPLY', choose, self.math('SUBTRACT', lateral, across)))
            best = self.math('MINIMUM', best, distance)
            station += length
        return along, across

    def mix(self, factor, a, b):
        node = self.nodes.new('ShaderNodeMixRGB')
        self.input(node.inputs[0], factor)
        self.input(node.inputs[1], a)
        self.input(node.inputs[2], b)
        return node.outputs[0]

    def curve(self, axis, points):
        """Piecewise linear landmark boundary, constant beyond its end stations."""
        value = points[0][1]
        for (x0, y0), (x1, y1) in zip(points, points[1:]):
            if x1 <= x0:
                raise ValueError('curve stations must be strictly increasing')
            t = self.math('MULTIPLY', self.math('SUBTRACT', axis, x0), 1 / (x1 - x0))
            t = self.math('MINIMUM', self.math('MAXIMUM', t, 0), 1)
            value = self.math('ADD', value, self.math('MULTIPLY', t, y1 - y0))
        return value

    def mask(self, conditions):
        value = 1
        for name, bounds in conditions.get('normal_bounds', {}).items():
            direction = [int(axis == name) for axis in 'xyz']
            component = self.vector_dot(self.normal, direction)
            for operation, limit in [('GREATER_THAN', bounds[0]), ('LESS_THAN', bounds[1])]:
                value = self.math('MULTIPLY', value, self.math(operation, component, limit))
        if 'panels' in conditions:
            matches = 0
            for panel in conditions['panels']:
                match = self.math('LESS_THAN', self.math('ABSOLUTE', self.math('SUBTRACT', self.panel, panel)), .1)
                matches = self.math('MAXIMUM', matches, match)
            value = self.math('MULTIPLY', value, matches)
        for name, bounds in conditions.get('bounds', {}).items():
            axis = self.axes['xyz'.index(name)]
            for op, limit in [('GREATER_THAN', bounds[0]), ('LESS_THAN', bounds[1])]:
                value = self.math('MULTIPLY', value, self.math(op, axis, limit))
        for item in conditions.get('curves', []):
            axis = self.axes['xyz'.index(item['axis'])]
            if item.get('absolute'):
                axis = self.math('ABSOLUTE', axis)
            limit = self.curve(self.axes['xyz'.index(item['along'])], item['points'])
            value = self.math('MULTIPLY', value, self.math('GREATER_THAN' if item['above'] else 'LESS_THAN', axis, limit))
        return value


def part_for(name):
    if '_wheel_cover_' in name:
        return 'front_covers' if 'front' in name else 'rear_covers'
    return PARTS.get(name, '').removesuffix('_stickers') or None


def tag_panels(obj):
    """Label original connected UV charts before modifiers interpolate geometry."""
    mesh = obj.data
    mesh.calc_loop_triangles()
    uv = np.array([loop.uv[:] for loop in mesh.uv_layers[0].data])
    vertices = np.array([loop.vertex_index for loop in mesh.loops])
    triangles = np.array([tri.loops[:] for tri in mesh.loop_triangles])
    _, keys = np.unique(np.column_stack((vertices, np.round(uv, 5))), axis=0, return_inverse=True)
    a = keys[triangles].ravel()
    b = np.roll(keys[triangles], 1, axis=1).ravel()
    graph = coo_matrix((np.ones(len(a)), (a, b)), shape=(keys.max()+1, keys.max()+1)).tocsr()
    labels = connected_components(graph, directed=False)[1]
    panels = labels[keys[np.array([face.loop_start for face in mesh.polygons])]]
    attr = mesh.attributes.new('native_panel', 'INT', 'FACE')
    attr.data.foreach_set('value', panels.astype(np.int32))


def meshes(spec):
    """Evaluate source modifiers; UV0 is copied, never authored or repacked."""
    deps = bpy.context.evaluated_depsgraph_get()
    result = {}
    records = []
    for source in list(bpy.data.collections['fp04_tr_model'].all_objects):
        if source.type != 'MESH' or not source.data.polygons or source.hide_render:
            continue
        part = part_for(source.name)
        if part not in spec['parts']:
            continue
        original = bpy.data.meshes.new_from_object(source.evaluated_get(deps), preserve_all_data_layers=True, depsgraph=deps)
        uv = np.empty(len(original.loops) * 2, np.float32)
        original.uv_layers[0].data.foreach_get('uv', uv)
        bpy.data.meshes.remove(original)
        tag_panels(source)
        deps.update()
        mesh = bpy.data.meshes.new_from_object(source.evaluated_get(deps), preserve_all_data_layers=True, depsgraph=deps)
        if len(mesh.loops) * 2 != len(uv):
            raise RuntimeError('panel tagging changed evaluated topology: ' + source.name)
        # Adding a face attribute can change subdivision UV rounding. Retain the
        # untouched evaluation's UV0 bytes rather than rebaking that tiny drift.
        mesh.uv_layers[0].data.foreach_set('uv', uv)
        copied = np.empty_like(uv)
        mesh.uv_layers[0].data.foreach_get('uv', copied)
        if not np.array_equal(uv, copied):
            raise RuntimeError('source evaluated UV0 was not retained: ' + source.name)
        records.append({'object': source.name, 'part': part, 'loops': len(mesh.loops), 'uv0_sha256': hashlib.sha256(uv.tobytes()).hexdigest()})
        mesh.transform(source.matrix_world)
        obj = bpy.data.objects.new('Native ' + source.name, mesh)
        bpy.context.scene.collection.objects.link(obj)
        result.setdefault(part, []).append(obj)
    return result, records


def bake(groups, spec, logos, out, size, records, only):
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.cycles.device = 'CPU'
    scene.cycles.samples = 1
    scene.render.threads_mode = 'FIXED'
    scene.render.threads = 8
    scene.render.bake.margin = 24
    scene.render.bake.margin_type = 'EXTEND'
    scene.render.bake.use_clear = True
    scene.render.bake.use_selected_to_active = False
    # Emission bake writes texture colour, independently of scene lighting.
    scene.view_settings.view_transform = 'Standard'
    frame = bpy.data.objects.new('Native painting frame', None)
    scene.collection.objects.link(frame)
    frame.location = spec['frame']['origin']
    frame.rotation_euler[2] = spec['frame']['yaw']
    for part, objects in groups.items():
        if only and part not in only:
            continue
        paint = Paint(part, spec, frame, logos)
        bpy.ops.object.select_all(action='DESELECT')
        for obj in objects:
            obj.select_set(True)
            obj.hide_render = False
            obj.hide_set(False)
            obj.data.materials.clear()
            obj.data.materials.append(paint.material)
            for face in obj.data.polygons:
                face.material_index = 0
            obj.data.uv_layers.active_index = 0
            obj.data.uv_layers[0].active_render = True
        bpy.context.view_layer.objects.active = objects[0]
        if len(objects) > 1:
            bpy.ops.object.join()
        obj = bpy.context.object
        before = np.empty(len(obj.data.loops) * 2, np.float32)
        obj.data.uv_layers[0].data.foreach_get('uv', before)
        image = bpy.data.images.new('Native baked ' + part, width=size, height=size, alpha=False)
        image.colorspace_settings.name = 'sRGB'
        target = paint.nodes.new('ShaderNodeTexImage')
        target.image = image
        paint.nodes.active = target
        print(f'NATIVE_BAKE {part} {size} start', flush=True)
        bpy.ops.object.bake(type='EMIT')
        after = np.empty_like(before)
        obj.data.uv_layers[0].data.foreach_get('uv', after)
        if not np.array_equal(before, after):
            raise RuntimeError('UV0 changed during bake')
        image.filepath_raw = str((out / (part + '.png')).resolve())
        image.file_format = 'PNG'
        image.save()
        print(f'NATIVE_BAKE {part} saved', flush=True)
        bpy.data.images.remove(image)
        bpy.data.objects.remove(obj, do_unlink=True)
    manifest = {'schema': 1, 'team': spec['team'], 'size': size, 'spec_sha256': hashlib.sha256(json.dumps(spec, sort_keys=True).encode()).hexdigest(), 'uv_channel': 0, 'objects': records, 'parts': sorted(part for part in groups if not only or part in only), 'artwork_sha256': {name: hashlib.sha256((logos / name).read_bytes()).hexdigest() for name in sorted({d['image'] for d in spec['decals']})}, 'source_sha256': spec.get('source_sha256'), 'painter_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    (out / 'native-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    (out / 'paint-spec.json').write_text(json.dumps(spec, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('spec', type=Path)
    parser.add_argument('--logos', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--size', type=int, default=4096)
    parser.add_argument('--parts', nargs='+')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    spec = json.loads(args.spec.read_text())
    if spec.get('source_sha256'):
        with Path(bpy.data.filepath).open('rb') as source:
            actual = hashlib.file_digest(source, 'sha256').hexdigest()
        if actual != spec['source_sha256']:
            parser.error('source blend fingerprint differs; re-audit native panel IDs')
    for decal in spec['decals']:
        if not (args.logos / decal['image']).is_file():
            parser.error('missing artwork: ' + decal['image'])
    args.out.mkdir(parents=True, exist_ok=True)
    groups, records = meshes(spec)
    if set(groups) != set(spec['parts']):
        parser.error('source paint parts differ from spec: ' + str(set(groups) ^ set(spec['parts'])))
    keep = {o for items in groups.values() for o in items}
    for obj in list(bpy.data.objects):
        if obj not in keep:
            bpy.data.objects.remove(obj, do_unlink=True)
    for text in list(bpy.data.texts):
        bpy.data.texts.remove(text)
    bpy.data.orphans_purge(do_recursive=True)
    bake(groups, spec, args.logos, args.out, args.size, records, args.parts)


if __name__ == '__main__':
    main()
