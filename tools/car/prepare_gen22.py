"""Safe, reproducible Gen22 conversion. Invoke through build_gen22.sh.

Never execute the source file's embedded scripts. Preserve original texture UVs;
only live LCD/LED surfaces receive generated instrument UVs.
"""
import argparse
import json
import math
from pathlib import Path
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
import driver_steer  # noqa: E402
import source_materials  # noqa: E402
import wheel_bake  # noqa: E402
import tyre_bake  # noqa: E402
import livery_seams  # noqa: E402

NODES = ('body', 'wheel_fl', 'wheel_fr', 'wheel_rl', 'wheel_rr',
         'upright_fl', 'upright_fr', 'steering_wheel', 'drs_flap', 'driver', 'helmet',
         'paddle_up', 'paddle_down', 'drs_button')
WHEEL_SOURCE = dict(fl='wheel.Ft.L', fr='wheel.Ft.R', rl='wheel.Bk.L', rr='wheel.Bk.R')


def select(objects):
    bpy.ops.object.select_all(action='DESELECT')
    for obj in objects:
        obj.hide_set(False)
        obj.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]


def triangles(obj):
    return sum(len(p.vertices) - 2 for p in obj.data.polygons)


def gltf(v):
    return [round(v[0], 6), round(v[2], 6), round(-v[1], 6)]


def decimate(obj, target, protect_materials=()):
    """Collapse in physical space, retaining source UVs and baked pose attributes."""
    count = triangles(obj)
    if count <= target:
        livery_seams.constrain(obj)
        return
    livery_seams.split(obj)
    select([obj])
    protected={index for face in obj.data.polygons
               if protect_materials and obj.data.materials[face.material_index].name in protect_materials
               for index in face.vertices}
    group=None
    if protected:
        group=obj.vertex_groups.new(name='Retained control artwork')
        group.add(list(protected),1,'REPLACE')
    mod = obj.modifiers.new('Silhouette budget', 'DECIMATE')
    mod.ratio = target / count
    mod.use_collapse_triangulate = True
    mod.delimit = {'UV', 'MATERIAL', 'NORMAL'}
    if group:
        mod.vertex_group=group.name
        mod.invert_vertex_group=True
        mod.vertex_group_factor=1000
    bpy.ops.object.modifier_apply(modifier=mod.name)
    livery_seams.constrain(obj)
    retained=obj.vertex_groups.get('Retained control artwork')
    if retained: obj.vertex_groups.remove(retained)


def node_for(obj):
    chain = []
    p = obj
    while p:
        chain.append(p.name)
        p = p.parent
    if 'GT3 Racing Helmet  (New) PL' in chain:
        return 'helmet'
    if obj.name == 'racer':
        return 'driver'
    if obj.parent_bone == 'steering_wheel':
        return 'steering_wheel'
    if obj.parent_bone == 'drs_wing':
        return 'drs_flap'
    for key, source in WHEEL_SOURCE.items():
        if source in chain:
            return 'wheel_' + key
        if key.startswith('f') and source.replace('wheel.', 'wheelbrake.') in chain:
            return 'upright_' + key
    return 'body'


def target_for(name, node):
    if name == 'body_object': return 180000
    if name == 'floor_rb_mixer': return 110000
    if name == 'front_wing_mixer': return 80000
    if name.startswith('rear_wing_no_mod'): return 50000
    if node == 'driver': return 30000
    if node == 'helmet': return 16000 if name == 'shell' else 4000
    if node.startswith('wheel_'):
        if name.startswith('wheel.'): return 19000
        if 'rim_' in name: return 7000
        if 'cover_' in name: return 5000
        if 'ventil' in name: return 500
        return 1500
    if 'cockpit_inner' in name: return 12000
    if 'seatbelt_mechanism' in name: return 900
    return 5000


def export_models(nodes, args, metadata):
    import struct
    source_materials.export_models(nodes, args, metadata, decimate, select, triangles)
    data = (args.output/'car.glb').read_bytes()
    length = struct.unpack_from('<I', data, 12)[0]
    document = json.loads(data[20:20+length])
    metadata['faces'] = {node['name']: sum(document['accessors'][p['indices']]['count']//3
                         for p in document['meshes'][node['mesh']]['primitives'])
                         for node in document['nodes'] if 'mesh' in node}
    (args.output/'car.json').write_text(json.dumps(metadata, indent=2)+'\n')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('output', type=Path)
    parser.add_argument('--work', type=Path, default=Path('raw/gen22/pbr'))
    parser.add_argument('--textures', type=Path, default=Path('raw/gen22/source-textures'))
    parser.add_argument('--template-year', choices=['2025', '2026'], default='2025')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    args.output.mkdir(parents=True, exist_ok=True)
    args.work.mkdir(parents=True, exist_ok=True)
    scene = bpy.context.scene
    bpy.context.view_layer.update()
    sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'livery'))
    import author_template_uv
    author_template_uv.apply(args.template_year)
    bpy.context.view_layer.update()
    centres = {key: bpy.data.objects[n].matrix_world.translation.copy() for key,n in WHEEL_SOURCE.items()}
    front = (centres['fl']+centres['fr'])/2
    rear = (centres['rl']+centres['rr'])/2
    heading = rear-front
    yaw = math.atan2(heading.x, heading.y)
    rotation = Matrix.Rotation(yaw, 4, 'Z')
    mid = rotation @ ((front+rear)/2)
    correction = Matrix.Translation((-mid.x,-mid.y,0)) @ rotation
    candidates = [o for o in bpy.data.collections['fp04_tr_model'].all_objects if o.type == 'MESH' and len(o.data.polygons) and not o.hide_render]
    skip = {'helmet','shoes.l','shoes.r','bee+_logo_grey-01',
            'steering_wheel_display_glass.002','steering_wheel_display_emmiter.002'}
    materials = source_materials.Library(args.textures)
    # Record source slots before assigning portable PBR replacements.
    original_materials = {o.name:[m.name if m else '' for m in o.data.materials] for o in candidates}
    # Bake steering/shift poses first; capture() leaves the rig straight ahead.
    poses = driver_steer.capture([o for o in candidates if o.name not in skip and node_for(o) in ('driver','helmet')])
    deps = bpy.context.evaluated_depsgraph_get()
    records = []
    wheel_materials = {}
    for source in candidates:
        if source.name in skip: continue
        node = node_for(source)
        mesh = bpy.data.meshes.new_from_object(source.evaluated_get(deps), preserve_all_data_layers=True, depsgraph=deps)
        obj = bpy.data.objects.new('converted_' + source.name, mesh)
        scene.collection.objects.link(obj)
        obj.matrix_world = source.matrix_world
        if node == 'steering_wheel' and source.name != 'ferrari_dashboard_screen':
            wheel_materials[source.name] = wheel_bake.bake(obj, args.work, materials)
        if source.name.startswith('wheel.') and ('tyre',) not in materials.cache:
            materials.cache[('tyre',)] = tyre_bake.bake(obj, args.work, args.output)
        obj.matrix_world = Matrix.Identity(4)
        # Preserve the authored UVs. Only instrumentation uses generated UVs below.
        active = mesh.uv_layers.active
        coordinates = [tuple(uv.uv) for uv in active.data] if active else None
        template = mesh.uv_layers.get('livery')
        template_coordinates = [tuple(uv.uv) for uv in template.data] if template else None
        for layer in list(mesh.uv_layers): mesh.uv_layers.remove(layer)
        uv = mesh.uv_layers.new(name='OriginalUV')
        if coordinates is not None:
            for loop, co in zip(uv.data, coordinates): loop.uv = co
        uv.active_render = True
        if template_coordinates is not None:
            template = mesh.uv_layers.new(name='livery')
            for loop, co in zip(template.data, template_coordinates): loop.uv = co
        mesh.uv_layers.active = uv
        mesh.transform(correction @ source.matrix_world)
        obj['source_indices'] = [p.material_index for p in mesh.polygons]
        detail = mesh.attributes.new('player_cockpit_detail', 'INT', 'FACE')
        interior = any(token in source.name.lower() for token in
                       ('cockpit_inner', 'seat', 'belt', 'steering_rod'))
        for face in detail.data: face.value = int(interior)
        if source.name in poses:
            driver_steer.attach(obj, poses[source.name], correction.to_3x3())
        records.append((obj, source.name, node))
        if source.name == driver_steer.DRS_BUTTON_SOURCE:
            cap = driver_steer.split_drs_button(obj, correction @ Vector(driver_steer.DRS_BUTTON_CENTRE))
            obj['source_indices'] = [p.material_index for p in obj.data.polygons]
            cap['source_indices'] = [p.material_index for p in cap.data.polygons]
            records.append((cap, 'drs_button_cap', 'drs_button'))
    # Leave only evaluated meshes. No lights, ground, rig, animation, or source code.
    keep = {obj for obj,_,_ in records}
    for obj in list(bpy.data.objects):
        if obj not in keep: bpy.data.objects.remove(obj, do_unlink=True)
    ground = min(v.co.z for obj,_,n in records if n.startswith('wheel_') for v in obj.data.vertices)
    shift = Matrix.Translation((0,0,-ground))
    correction = shift @ correction
    for obj,_,_ in records: obj.data.transform(shift)
    for obj, name, node in records:
        old = original_materials[name] if node != 'drs_button' else original_materials[driver_steer.DRS_BUTTON_SOURCE]
        if not old: old = ['chrome']
        indices = list(obj['source_indices'])
        obj.data.materials.clear()
        replacements = wheel_materials.get(name) or [materials.for_part(name, material_name, node) for material_name in old]
        for mat in replacements: obj.data.materials.append(mat)
        for p, i in zip(obj.data.polygons, indices):
            # A few source meshes reference absent material slots. Blender treats
            # these as slot zero; normalise that invalid source data once here.
            p.material_index = i if i < len(replacements) else 0
            p.use_smooth = True
        if name == 'steering_wheel_top_panel_leds.002':
            # Only the top fifteen lamps are live rev lights. Preserve each side
            # warning lens's original coloured PBR material.
            top=max(v.co.z for v in obj.data.vertices)
            mid=(min(v.co.x for v in obj.data.vertices)+max(v.co.x for v in obj.data.vertices))/2
            live=materials.for_part('live_rev_strip','', 'wheel_leds')
            obj.data.materials.append(live)
            # Preserve the source lens groups in the static export. The game
            # replaces this texture when it attaches live per-lamp RPM materials.
            import numpy as np
            lamps = [face for face in obj.data.polygons
                     if face.center.z>=top-.006 and abs(face.center.x-mid)<.05]
            lo=min(obj.data.vertices[i].co.x for face in lamps for i in face.vertices)
            hi=max(obj.data.vertices[i].co.x for face in lamps for i in face.vertices)
            samples=[]
            for face in lamps:
                source_mat=materials.originals[old[indices[face.index]]]
                group=next(n for n in source_mat.node_tree.nodes if n.type=='GROUP')
                color=tuple(group.inputs['Base Color'].default_value[:3])
                samples.append(((hi-face.center.x)/(hi-lo),color))
            strip=np.ones((1,256,4),dtype=np.float32)
            for x in range(256):
                strip[0,x,:3]=min(samples,key=lambda sample:abs(sample[0]-x/255))[1]
            shader=live.node_tree.nodes.get('Principled BSDF')
            tex=source_materials.image_node(live,source_materials.generated_image('source_rev_groups',strip))
            live.node_tree.links.new(tex.outputs['Color'],shader.inputs['Base Color'])
            live.node_tree.links.new(tex.outputs['Color'],shader.inputs['Emission Color'])
            for face in obj.data.polygons:
                if face.center.z>=top-.006 and abs(face.center.x-mid)<.05:
                    face.material_index=len(obj.data.materials)-1
        if any(mat.name.startswith('source_paint_') for mat in obj.data.materials):
            assert obj.data.uv_layers.get('livery'), ('paint part has no template UVs', name)
        materials.add_detail_uv(obj)
        if node not in ('steering_wheel', 'drs_button'):
            decimate(obj, target_for(name, node))
        print('PART', name, node, triangles(obj), flush=True)
    # The two shift paddles are one source bar; cut it at the hub so each can move.
    hub = correction @ Vector((.056,1.60,.589))
    index = next(i for i,(_,name,_) in enumerate(records) if name == driver_steer.PADDLE_SOURCE)
    halves, hinges, paddle_meta = driver_steer.split_paddles(records.pop(index)[0], hub.x)
    records += [(obj, driver_steer.PADDLE_SOURCE, node) for node,obj in halves.items()]
    # Consolidation reduces each node to one mesh and each material to one primitive.
    nodes = {}
    for name in NODES:
        objects = [o for o,_,n in records if n == name]
        if not objects: continue
        select(objects)
        if len(objects)>1: bpy.ops.object.join()
        obj = bpy.context.object
        obj.name = name
        nodes[name] = obj
    # Delete collapsed zero-area triangles before counting the shipped geometry.
    for obj in nodes.values():
        bm = bmesh.new(); bm.from_mesh(obj.data)
        bmesh.ops.triangulate(bm, faces=list(bm.faces))
        dead = [f for f in bm.faces if f.calc_area()<1e-14]
        bmesh.ops.delete(bm,geom=dead,context='FACES')
        loose = [v for v in bm.verts if not v.link_faces]
        bmesh.ops.delete(bm,geom=loose,context='VERTS')
        # Preserve machined edges instead of smoothing across sharp corners.
        for edge in bm.edges:
            if len(edge.link_faces)==2: edge.smooth=edge.calc_face_angle()<math.radians(40)
        bm.to_mesh(obj.data); bm.free()
    for text in list(bpy.data.texts): bpy.data.texts.remove(text)
    # Screen plane faces the driver. Texture top is physical top; left is driver's left.
    obj = nodes['steering_wheel']
    for mat in ('wheel_screen','wheel_leds'):
        polygons = [p for p in obj.data.polygons if obj.data.materials[p.material_index].name == mat]
        verts = [obj.data.vertices[obj.data.loops[i].vertex_index].co for p in polygons for i in p.loop_indices]
        if not verts: raise RuntimeError('Missing ' + mat)
        lo = [min(v[i] for v in verts) for i in range(3)]
        hi = [max(v[i] for v in verts) for i in range(3)]
        for p in polygons:
            for i in p.loop_indices:
                v = obj.data.vertices[obj.data.loops[i].vertex_index].co
                obj.data.uv_layers.active.data[i].uv = ((hi[0]-v.x)/(hi[0]-lo[0]), (v.z-lo[2])/(hi[2]-lo[2]))
    cap = nodes['drs_button']
    lo = [min(v.co[i] for v in cap.data.vertices) for i in range(3)]
    hi = [max(v.co[i] for v in cap.data.vertices) for i in range(3)]
    for loop in cap.data.loops:
        v = cap.data.vertices[loop.vertex_index].co
        cap.data.uv_layers.active.data[loop.index].uv = ((hi[0]-v.x)/(hi[0]-lo[0]), (v.z-lo[2])/(hi[2]-lo[2]))
    # Origins carry the animation contract, with baked local geometry and unit scale.
    for key in WHEEL_SOURCE:
        pivot = correction @ centres[key]
        for n in ('wheel_'+key, 'upright_'+key):
            if n in nodes:
                nodes[n].data.transform(Matrix.Translation(-pivot))
                nodes[n].location = pivot
    steering = correction @ Vector((.056,1.60,.589))
    drs = correction @ Vector((.0758088,4.624894,.959596))
    for name,pivot in [('steering_wheel',steering),('drs_flap',drs)]:
        nodes[name].data.transform(Matrix.Translation(-pivot))
        nodes[name].location = pivot
    nodes['helmet'].parent = nodes['driver']
    # The cap's local +Z in glTF points into the wheel.
    cap_centre = Vector([(lo[i]+hi[i])*.5 for i in range(3)])
    hinges['drs_button'] = cap_centre
    # Paddles and the cap follow the wheel.
    for name,hinge in hinges.items():
        nodes[name].data.transform(Matrix.Translation(-hinge))
        # Local to the (unrotated) wheel; matrix_world is stale until the next update.
        nodes[name].parent = nodes['steering_wheel']
        nodes[name].matrix_parent_inverse = Matrix.Identity(4)
        nodes[name].location = hinge - steering
    bpy.context.view_layer.update()
    # Metadata is in glTF space; Blender +Z maps to glTF +Y.
    points = [o.matrix_world @ v.co for o in nodes.values() for v in o.data.vertices]
    low = [min(p[i] for p in points) for i in range(3)]
    high = [max(p[i] for p in points) for i in range(3)]
    extents = dict(min=gltf((low[0],high[1],low[2])), max=gltf((high[0],low[1],high[2])))
    metadata = dict(livery_template=int(args.template_year), wheels={},extents=extents,faces={n:triangles(o) for n,o in nodes.items()},
                    steering_wheel=dict(pivot=gltf(steering),axis=[0,0,1]),
                    drs=dict(pivot=gltf(drs),axis=[1,0,0],open_angle=-.25),
                    cockpit_eye=gltf(correction @ Vector((.0636,2.075,.765))))
    metadata.update(driver_steer.metadata())
    metadata.update(paddle_meta)
    for key in WHEEL_SOURCE:
        obj = nodes['wheel_'+key]
        verts = [v.co for v in obj.data.vertices]
        metadata['wheels'][key] = dict(centre=gltf(obj.location),
            radius=round((max(v.z for v in verts)-min(v.z for v in verts))/2,5),
            width=round(max(v.x for v in verts)-min(v.x for v in verts),5))
    bpy.context.view_layer.update()
    (args.output/'car.json').write_text(json.dumps(metadata,indent=2)+'\n')
    for text in list(bpy.data.texts): bpy.data.texts.remove(text)
    bpy.ops.wm.save_as_mainfile(filepath=str((args.work/'prepared.blend').resolve()))
    export_models(nodes, args, metadata)


if __name__ == '__main__': main()
