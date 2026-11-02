"""Assert the shipped GLBs, optionally re-importing them in clean Blender.

blender -b --disable-autoexec --factory-startup --python tools/car/check_gen22.py -- assets/cars/gen22
python3 -I tools/car/check_gen22.py assets/cars/gen22
"""
import argparse
import io
import json
import math
from pathlib import Path
import struct
import sys

REQUIRED = {'body','wheel_fl','wheel_fr','wheel_rl','wheel_rr','steering_wheel','drs_flap','driver','helmet',
            'paddle_up','paddle_down','drs_button'}
CHILDREN = {'helmet':'driver','paddle_up':'steering_wheel','paddle_down':'steering_wheel','drs_button':'steering_wheel'}
MATERIALS = {'wheel_screen','wheel_leds','wheel_drs'}


def read_glb(path):
    data = path.read_bytes()
    magic,version,length = struct.unpack_from('<4sII',data)
    assert magic == b'glTF' and version == 2 and length == len(data)
    size,kind = struct.unpack_from('<II',data,12)
    assert kind == 0x4e4f534a
    document = json.loads(data[20:20+size])
    binary_start = 20+size+8
    return document,data[binary_start:]


def vectors(document, binary, index):
    accessor=document['accessors'][index]
    assert accessor['type']=='VEC3' and accessor['componentType']==5126
    result = [(0.,0.,0.)]*accessor['count']
    if 'bufferView' in accessor:
        view=document['bufferViews'][accessor['bufferView']]
        offset=view.get('byteOffset',0)+accessor.get('byteOffset',0)
        stride=view.get('byteStride',12)
        result=[struct.unpack_from('<3f',binary,offset+i*stride) for i in range(accessor['count'])]
    if 'sparse' in accessor:
        sparse=accessor['sparse']
        indices,values=sparse['indices'],sparse['values']
        start=document['bufferViews'][indices['bufferView']].get('byteOffset',0)+indices.get('byteOffset',0)
        data=document['bufferViews'][values['bufferView']].get('byteOffset',0)+values.get('byteOffset',0)
        fmt={5121:'<B',5123:'<H',5125:'<I'}[indices['componentType']]
        for i in range(sparse['count']):
            vertex=struct.unpack_from(fmt,binary,start+i*struct.calcsize(fmt))[0]
            result[vertex]=struct.unpack_from('<3f',binary,data+i*12)
    return result


def check_template_uv(document, binary, index):
    accessor = document['accessors'][index]
    assert accessor['type'] == 'VEC2' and accessor['componentType'] == 5126
    view = document['bufferViews'][accessor['bufferView']]
    offset = view.get('byteOffset', 0) + accessor.get('byteOffset', 0)
    stride = view.get('byteStride', 8)
    for vertex in range(accessor['count']):
        uv = struct.unpack_from('<2f', binary, offset + vertex * stride)
        assert all(-1e-6 <= value <= 1 + 1e-6 for value in uv), ('paint UV1 outside template', uv)


def inspect(path, meta):
    doc,binary = read_glb(path)
    nodes = {n['name']:n for n in doc['nodes']}
    lod = path.name != 'car.glb'
    distant = path.name == 'car_lod2.glb'
    required = REQUIRED - {'steering_wheel','paddle_up','paddle_down','drs_button'} if lod else REQUIRED
    assert required <= nodes.keys(), required-nodes.keys()
    if lod:
        assert not {'steering_wheel','paddle_up','paddle_down','drs_button'} & nodes.keys()
        assert not any('targets' in p for mesh in doc['meshes'] for p in mesh['primitives'])
    roots = set(doc['scenes'][doc.get('scene',0)]['nodes'])
    for i,node in enumerate(doc['nodes']):
        assert node['name'] in REQUIRED|{'upright_fl','upright_fr'},node['name']
        assert i in roots or node['name'] in CHILDREN,node['name']
        assert node.get('scale',[1,1,1]) == [1,1,1]
        assert all(abs(a-b)<1e-5 for a,b in zip(node.get('rotation',[0,0,0,1]),[0,0,0,1]))
    for child,parent in CHILDREN.items():
        if child not in nodes: continue
        assert doc['nodes'].index(nodes[child]) in nodes[parent]['children'],child
    # One morph target per baked steering, shift and DRS pose on every driver/helmet primitive.
    poses = len(meta['driver_poses'])+len(meta['driver_shift_poses'])+1
    assert meta['driver_drs_pose']==poses-1
    assert meta['drs_button_travel']==[0,0,.003]
    if not lod:
        centre = [a+b+c for a,b,c in zip(nodes['steering_wheel']['translation'],
                                        nodes['drs_button']['translation'],meta['drs_button_travel'])]
        # Measure the shipped glove, including sparse morph offsets, rather than
        # accepting a moving rig bone whose skin never reaches the button.
        gaps = [float('inf'),float('inf')]
        for primitive in doc['meshes'][nodes['driver']['mesh']]['primitives']:
            positions = vectors(doc,binary,primitive['attributes']['POSITION'])
            offsets = vectors(doc,binary,primitive['targets'][meta['driver_drs_pose']]['POSITION'])
            for press in (0,1):
                gaps[press] = min(gaps[press], min(sum((v[i]+press*offset[i]-centre[i])**2
                                                      for i in range(3))**.5
                                                   for v,offset in zip(positions,offsets)))
        print('DRS GLOVE GAP',*gaps)
        assert gaps[1] < .008 and gaps[1] < gaps[0]*.5, ('thumb misses DRS cap',gaps)
        for name in ('driver','helmet'):
            for primitive in doc['meshes'][nodes[name]['mesh']]['primitives']:
                assert len(primitive.get('targets',[]))==poses,(name,len(primitive.get('targets',[])))
                checked = poses if name == 'driver' else len(meta['driver_poses'])
                for target in primitive['targets'][:checked]:
                    position = doc['accessors'][target['POSITION']]
                    # A target count alone can pass even if decimation erased the motion.
                    assert max(abs(v) for key in ('min','max') for v in position[key]) > .001, (name, 'empty pose')
    assert meta['paddles']['up_angle']<0<meta['paddles']['down_angle']
    materials = [m['name'] for m in doc['materials']]
    if not lod:
        assert MATERIALS <= set(materials),MATERIALS-set(materials)
        wheel_materials=[m for m in doc['materials'] if m['name'].startswith('source_wheel_')]
        assert any('steering_wheel_stickers' in m['name'] for m in wheel_materials)
        assert any('rough_steering_wheel' in m['name'] for m in wheel_materials)
        assert all('baseColorTexture' in m['pbrMetallicRoughness']
                   and 'metallicRoughnessTexture' in m['pbrMetallicRoughness']
                   and 'normalTexture' in m for m in wheel_materials), 'missing source wheel bakes'
    else: assert not any(m.startswith('wheel_') for m in materials)
    tyres = [m for m in doc['materials'] if m['name'].split('.')[0] == 'tyre']
    assert len(tyres) == 1, 'missing textured rubber material'
    tyre = tyres[0]['pbrMetallicRoughness']
    assert tyre.get('metallicFactor', 1) == 0
    assert .65 <= tyre['roughnessFactor'] <= .75
    assert tyre['baseColorTexture'].get('texCoord', 0) == 0
    assert tyre.get('baseColorFactor', [1,1,1,1]) == [1,1,1,1]
    painted = [m for m in doc['materials'] if m['name'].startswith('source_paint_')]
    assert len(painted) >= 8, ('missing per-part artwork', len(painted))
    assert sum('occlusionTexture' in m for m in painted) >= 6
    assert all('baseColorTexture' not in m['pbrMetallicRoughness'] for m in painted), 'team artwork is bound at runtime'
    if meta.get('livery_template'):
        for material in painted:
            for texture in ('occlusionTexture', 'normalTexture'):
                if texture in material:
                    assert material[texture].get('texCoord', 0) == 0, (material['name'], texture, 'must retain OriginalUV')
    if lod:
        baked = [m for m in painted if '_lod' in m['name']]
        assert baked and all('normalTexture' in m for m in baked), 'missing LOD projection normals'
    if not lod: assert any('Brass' in m['name'] and ('metallicRoughnessTexture' in m['pbrMetallicRoughness'] or m['pbrMetallicRoughness'].get('metallicFactor',1)>.9) for m in doc['materials'])
    if not lod: assert any('Translucent' in m['name'] and max(m.get('emissiveFactor',[0]))>0 for m in doc['materials'])
    counts = {}
    low=[float('inf')]*3; high=[-float('inf')]*3
    primitive_count = 0
    for name,node in nodes.items():
        mesh = doc['meshes'][node['mesh']]
        primitives = mesh['primitives']
        primitive_count += len(primitives)
        counts[name] = sum(doc['accessors'][p['indices']]['count']//3 for p in primitives)
        assert all(p.get('mode',4)==4 for p in primitives)
        for primitive in primitives:
            if meta.get('livery_template') and materials[primitive['material']].startswith('source_paint_'):
                assert {'TEXCOORD_0', 'TEXCOORD_1'} <= primitive['attributes'].keys(), (name, 'missing original/template UVs')
                if meta['livery_template'] == 2025:
                    check_template_uv(doc, binary, primitive['attributes']['TEXCOORD_1'])
            if materials[primitive['material']].split('.')[0] == 'tyre':
                assert 'TEXCOORD_0' in primitive['attributes'], (name, 'tyre UV0 missing')
            position=doc['accessors'][primitive['attributes']['POSITION']]
            translation=node.get('translation',[0,0,0])
            if name in CHILDREN:
                parent=nodes[CHILDREN[name]].get('translation',[0,0,0])
                translation=[a+b for a,b in zip(translation,parent)]
            for axis in range(3):
                low[axis]=min(low[axis],position['min'][axis]+translation[axis])
                high[axis]=max(high[axis],position['max'][axis]+translation[axis])
        print('NODE',name,'translation',node.get('translation',[0,0,0]),'rotation',node.get('rotation',[0,0,0,1]),'triangles',counts[name],'materials',[materials[p['material']] for p in primitives])
    for key,wheel in meta['wheels'].items():
        location = nodes['wheel_'+key].get('translation',[0,0,0])
        assert max(abs(a-b) for a,b in zip(location,wheel['centre'])) < 1e-4
        assert .3 < wheel['radius'] < .4
        assert .2 < wheel['width'] < .5
        # Sample the actual silhouette all around the axle, in both LODs.
        # UV-space decimation previously shrank opponent tyres into flat slabs.
        mesh = doc['meshes'][nodes['wheel_'+key]['mesh']]
        tread = [v for p in mesh['primitives'] if materials[p['material']].split('.')[0] == 'tyre'
                 for v in vectors(doc, binary, p['attributes']['POSITION'])]
        support = [max(y*math.cos(a*math.tau/72)+z*math.sin(a*math.tau/72)
                       for _, y, z in tread) for a in range(72)]
        assert min(support) > wheel['radius']*.975, (key, 'flattened tyre', min(support))
        assert max(support) < wheel['radius']*1.025, (key, 'oversized tyre', max(support))
    for name,key in [('steering_wheel','steering_wheel'),('drs_flap','drs')]:
        if name in nodes: assert max(abs(a-b) for a,b in zip(nodes[name]['translation'],meta[key]['pivot']))<1e-4
    assert meta['steering_wheel']['axis']==[0,0,1]
    assert meta['drs']['axis']==[1,0,0] and .2<=abs(meta['drs']['open_angle'])<=.3
    assert abs(meta['cockpit_eye'][0])<.08 and .7<meta['cockpit_eye'][1]<.9
    assert not doc.get('animations') and not doc.get('skins')
    total = sum(counts.values())
    assert total <= (50000 if distant else 150000 if lod else 3000000),total
    if not lod: assert counts['steering_wheel']>=1200000, counts['steering_wheel']
    assert counts['driver']+counts['helmet']<=80000
    assert path.stat().st_size<=300_000_000
    if not lod:
        assert counts==meta['faces'],(counts,meta['faces'])
        assert max(abs(a-b) for a,b in zip(low,meta['extents']['min']))<1e-4,(low,meta['extents'])
        assert max(abs(a-b) for a,b in zip(high,meta['extents']['max']))<1e-4,(high,meta['extents'])
        assert 2<high[0]-low[0]<2.2 and 1.1<high[1]<1.3 and 5.3<high[2]-low[2]<5.8
        assert abs(low[1])<1e-4
    try:
        from PIL import Image
        for image in doc.get('images',[]):
            view = doc['bufferViews'][image['bufferView']]
            offset = view.get('byteOffset',0)
            size = Image.open(io.BytesIO(binary[offset:offset+view['byteLength']])).size
            print('TEXTURE',image.get('name'),size)
            assert max(size)<=(1024 if distant else 2048 if lod else 4096)
    except ImportError:
        pass  # Blender image sizes are checked below after real re-import.
    print('GLB',path,'triangles',total,'primitives',primitive_count,'bytes',path.stat().st_size)
    # Preserve distinct source materials instead of merging cockpit hardware.
    assert primitive_count <= 180,primitive_count
    return counts


def reimport(path, expected):
    import bpy
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(path.resolve()))
    actual = {}
    for obj in bpy.context.scene.objects:
        if obj.type!='MESH': continue
        count = sum(len(p.vertices)-2 for p in obj.data.polygons)
        actual[obj.name] = count
        print('REIMPORT',obj.name,'location',list(obj.location),'rotation',list(obj.rotation_quaternion),'scale',list(obj.scale),'triangles',count)
        assert all(len(p.vertices)==3 for p in obj.data.polygons)
    assert actual==expected,(actual,expected)
    for obj in bpy.context.scene.objects:
        if obj.type == 'MESH':
            assert obj.data.uv_layers, (obj.name, 'missing original UVs')
    for image in bpy.data.images:
        if image.source=='VIEWER': continue
        print('REIMPORT TEXTURE',image.name,list(image.size))
        assert max(image.size)<=(1024 if path.name=='car_lod2.glb' else 2048 if path.name=='car_lod1.glb' else 4096)


def check_tyre_textures(directory):
    from PIL import Image, ImageChops
    for lod, size in [('',2048), ('lod1',2048), ('lod2',1024)]:
        variants = []
        for compound in ('soft', 'medium', 'hard'):
            path = directory / 'tyres' / lod / (compound + '.png')
            assert path.is_file(), path
            with Image.open(path) as image:
                assert image.size == (size,size), (path, image.size)
                variants.append(image.convert('RGB'))
        assert ImageChops.difference(variants[0], variants[1]).getbbox(), 'compound maps are identical'
    for name in ('rubber', 'marking-mask'):
        assert (directory / 'tyres' / (name + '.png')).is_file(), name


def main():
    argv = sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else sys.argv[1:]
    parser = argparse.ArgumentParser()
    parser.add_argument('directory',type=Path)
    args = parser.parse_args(argv)
    meta = json.loads((args.directory/'car.json').read_text())
    for file in ('car.glb','car_lod1.glb','car_lod2.glb'):
        path = args.directory/file
        expected = inspect(path,meta)
        if 'bpy' in sys.modules: reimport(path,expected)
    if 'bpy' not in sys.modules:
        check_tyre_textures(args.directory)
    teams_path = args.directory/'teams.json'
    assert teams_path.is_file(), teams_path
    teams = json.loads(teams_path.read_text())
    assert teams['teams'], 'teams must not be empty'
    assert all('livery' not in team and 'suit' not in team for team in teams['teams'])
    print('GEN22 CONTRACT PASS')


if __name__=='__main__': main()
