"""Assert the shipped GLBs, optionally re-importing them in clean Blender.

blender -b --disable-autoexec --factory-startup --python tools/car/check_gen22.py -- assets/cars/gen22
python3 -I tools/car/check_gen22.py assets/cars/gen22
"""
import argparse
import io
import json
from pathlib import Path
import struct
import sys

REQUIRED = {'body','wheel_fl','wheel_fr','wheel_rl','wheel_rr','steering_wheel','drs_flap','driver','helmet',
            'paddle_up','paddle_down'}
CHILDREN = {'helmet':'driver','paddle_up':'steering_wheel','paddle_down':'steering_wheel'}
MATERIALS = {'paint','suit','helmet_paint','wheel_screen','wheel_leds'}


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
    view=document['bufferViews'][accessor['bufferView']]
    assert accessor['type']=='VEC3' and accessor['componentType']==5126
    offset=view.get('byteOffset',0)+accessor.get('byteOffset',0)
    stride=view.get('byteStride',12)
    return [struct.unpack_from('<3f',binary,offset+i*stride) for i in range(accessor['count'])]


def inspect(path, meta):
    doc,binary = read_glb(path)
    nodes = {n['name']:n for n in doc['nodes']}
    assert REQUIRED <= nodes.keys(), REQUIRED-nodes.keys()
    roots = set(doc['scenes'][doc.get('scene',0)]['nodes'])
    for i,node in enumerate(doc['nodes']):
        assert node['name'] in REQUIRED|{'upright_fl','upright_fr'},node['name']
        assert i in roots or node['name'] in CHILDREN,node['name']
        assert node.get('scale',[1,1,1]) == [1,1,1]
        assert all(abs(a-b)<1e-5 for a,b in zip(node.get('rotation',[0,0,0,1]),[0,0,0,1]))
    for child,parent in CHILDREN.items():
        assert doc['nodes'].index(nodes[child]) in nodes[parent]['children'],child
    # One morph target per baked steering and shift pose on every driver/helmet primitive.
    poses = len(meta['driver_poses'])+len(meta['driver_shift_poses'])
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
    assert MATERIALS <= set(materials),MATERIALS-set(materials)
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
            if materials[primitive['material']]=='tyre_marking':
                sign=1 if name.endswith('l') else -1
                normals=vectors(doc,binary,primitive['attributes']['NORMAL'])
                assert min(normal[0]*sign for normal in normals)>.99,(name,'inward compound ring')
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
    for name,key in [('steering_wheel','steering_wheel'),('drs_flap','drs')]:
        assert max(abs(a-b) for a,b in zip(nodes[name]['translation'],meta[key]['pivot']))<1e-4
    assert meta['steering_wheel']['axis']==[0,0,1]
    assert meta['drs']['axis']==[1,0,0] and .2<=abs(meta['drs']['open_angle'])<=.3
    assert abs(meta['cockpit_eye'][0])<.08 and .7<meta['cockpit_eye'][1]<.9
    assert not doc.get('animations') and not doc.get('skins')
    total = sum(counts.values())
    lod = path.name=='car_lod1.glb'
    assert total <= (90000 if lod else 300000),total
    assert counts['steering_wheel']<=45000
    assert counts['driver']+counts['helmet']<=35000
    assert path.stat().st_size<=60_000_000
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
            assert max(size)<=1024
    except ImportError:
        pass  # Blender image sizes are checked below after real re-import.
    print('GLB',path,'triangles',total,'primitives',primitive_count,'bytes',path.stat().st_size)
    # Two independently animated paddles add one primitive each to both LODs.
    assert primitive_count <= (22 if lod else 42),primitive_count
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
    import importlib.util
    spec=importlib.util.spec_from_file_location('gen22_prepare',Path(__file__).with_name('prepare_gen22.py'))
    helper=importlib.util.module_from_spec(spec); spec.loader.exec_module(helper)
    for name,material in [('body','paint'),('driver','suit'),('helmet','helmet_paint')]:
        obj=bpy.data.objects[name]
        folded=helper.overlapping_uv_faces(obj,material)
        assert not folded,(name,'exported UV overlaps',len(folded))
        coordinates=[obj.data.uv_layers.active.data[i].uv for face in obj.data.polygons
                     if obj.data.materials[face.material_index].name==material for i in face.loop_indices]
        low=min(min(uv) for uv in coordinates); high=max(max(uv) for uv in coordinates)
        assert low>=8/2048-1e-5 and high<=1-8/2048+1e-5,(name,'atlas border padding',low,high)
        print('REIMPORT ATLAS',material,'overlaps',len(folded),'border pixels',round(min(low,1-high)*2048,2))
    for image in bpy.data.images:
        if image.source=='VIEWER': continue
        print('REIMPORT TEXTURE',image.name,list(image.size))
        assert max(image.size)<=1024


def main():
    argv = sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else sys.argv[1:]
    parser = argparse.ArgumentParser()
    parser.add_argument('directory',type=Path)
    args = parser.parse_args(argv)
    meta = json.loads((args.directory/'car.json').read_text())
    for file in ('car.glb','car_lod1.glb'):
        path = args.directory/file
        expected = inspect(path,meta)
        if 'bpy' in sys.modules: reimport(path,expected)
    # Team images are external and must be complete, square sRGB atlases.
    teams_path = args.directory/'teams.json'
    assert teams_path.is_file(), teams_path
    teams = json.loads(teams_path.read_text())
    for team in teams['teams']:
        paths = [Path('paint.png'),Path('suit.png')]+[Path(d['id'])/'helmet.png' for d in team['drivers']]
        for file in paths:
            path = args.directory/'liveries'/team['id']/file
            assert path.is_file(),path
            if 'bpy' in sys.modules:
                import bpy
                image = bpy.data.images.load(str(path.resolve()),check_existing=False)
                assert list(image.size)==[2048,2048],path
                bpy.data.images.remove(image)
            else:
                from PIL import Image
                with Image.open(path) as image: assert image.size==(2048,2048),path
    print('LIVERIES',len(teams['teams']),'teams verified')
    print('GEN22 CONTRACT PASS')


if __name__=='__main__': main()
