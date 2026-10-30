"""Safe, reproducible Gen22 conversion. Invoke through build_gen22.sh.

Never execute the source file's embedded scripts. Positions in bake NPZ files
are metres in the same +X left, +Y up, +Z forward space as the exported glTF.
"""
import argparse
import json
import math
from pathlib import Path
import sys

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
import driver_steer  # noqa: E402

ATLAS = 2048
NODES = ('body', 'wheel_fl', 'wheel_fr', 'wheel_rl', 'wheel_rr',
         'upright_fl', 'upright_fr', 'steering_wheel', 'drs_flap', 'driver', 'helmet',
         'paddle_up', 'paddle_down')
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


def decimate(obj, target):
    count = triangles(obj)
    if count <= target:
        return
    select([obj])
    mod = obj.modifiers.new('Silhouette budget', 'DECIMATE')
    mod.ratio = target / count
    mod.use_collapse_triangulate = True
    # UV seams and material boundaries remain valid through the opponent LOD.
    bpy.ops.object.modifier_apply(modifier=mod.name)


def material(name, color, rough=.5, metal=0):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    shader = mat.node_tree.nodes.get('Principled BSDF')
    shader.inputs['Base Color'].default_value = (*color, 1)
    shader.inputs['Metallic'].default_value = metal
    shader.inputs['Roughness'].default_value = rough
    if name in ('paint', 'helmet_paint'):
        shader.inputs['Coat Weight'].default_value = .35
        shader.inputs['Coat Roughness'].default_value = .24
    mat.diffuse_color = (*color, 1)
    return mat


def image_texture(mat, name, pixels, noncolor=False):
    height, width = pixels.shape[:2]
    img = bpy.data.images.new(name, width=width, height=height, alpha=True)
    if noncolor:
        img.colorspace_settings.name = 'Non-Color'
    img.pixels.foreach_set(np.flipud(pixels).astype(np.float32).ravel())
    img.pack()
    tex = mat.node_tree.nodes.new('ShaderNodeTexImage')
    tex.image = img
    return tex


def make_materials():
    # Remove source materials first, including their trademark-bearing images.
    for mat in list(bpy.data.materials):
        bpy.data.materials.remove(mat)
    specs = {
        'paint': ((.55, .57, .6), .3, .08),
        'carbon': ((.012, .014, .017), .62, 0),
        'tyre': ((.025, .026, .028), .85, 0),
        'tyre_marking': ((.95, .65, .015), .65, 0),
        'metal': ((.3, .32, .35), .3, .8),
        'rubber': ((.016, .018, .02), .75, 0),
        'suit': ((.14, .18, .25), .8, 0),
        'helmet_paint': ((.8, .8, .8), .28, .08),
        'visor': ((.045, .07, .09), .17, .65),
        'mirror': ((.6, .65, .7), .1, 1),
        'wheel_screen': ((.008, .012, .016), .55, 0),
        'wheel_leds': ((.05, .9, .22), .3, .1),
        'emissive_red': ((.5, .002, .002), .3, 0),
    }
    mats = {n: material(n, *spec) for n, spec in specs.items()}
    for n in ('paint', 'suit', 'helmet_paint'):
        tex = image_texture(mats[n], n + '_placeholder', np.ones((8, 8, 4)))
        mats[n].node_tree.links.new(tex.outputs['Color'], mats[n].node_tree.nodes.get('Principled BSDF').inputs['Base Color'])
    # A restrained 2x2 twill normal map, not a baked lighting pattern.
    y, x = np.mgrid[:256, :256] / 16
    orientation = (np.floor(x) + np.floor(y)) % 4 < 2
    height = np.where(orientation, np.sin(x * math.tau), np.sin(y * math.tau)) * .12
    dy, dx = np.gradient(height)
    normal = np.stack((-dx * 2, -dy * 2, np.ones_like(dx)), axis=-1)
    normal /= np.linalg.norm(normal, axis=-1, keepdims=True)
    tex = image_texture(mats['carbon'], 'carbon_twill_normal', np.concatenate((normal * .5 + .5, np.ones((256, 256, 1))), axis=-1), True)
    bump = mats['carbon'].node_tree.nodes.new('ShaderNodeNormalMap')
    bump.inputs['Strength'].default_value = .3
    links = mats['carbon'].node_tree.links
    links.new(tex.outputs['Color'], bump.inputs['Color'])
    links.new(bump.outputs['Normal'], mats['carbon'].node_tree.nodes.get('Principled BSDF').inputs['Normal'])
    rough = .56 + .07*np.where(orientation,np.cos(x*math.tau),np.cos(y*math.tau))
    rgba = np.concatenate((np.repeat(rough[...,None],3,axis=2),np.ones((256,256,1))),axis=-1)
    tex = image_texture(mats['carbon'],'carbon_twill_roughness',rgba,True)
    links.new(tex.outputs['Color'],mats['carbon'].node_tree.nodes.get('Principled BSDF').inputs['Roughness'])
    for n in ('wheel_leds', 'emissive_red'):
        sh = mats[n].node_tree.nodes.get('Principled BSDF')
        sh.inputs['Emission Color'].default_value = sh.inputs['Base Color'].default_value
        sh.inputs['Emission Strength'].default_value = .8
    return mats


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
    if name == 'body_object': return 40000
    if name == 'floor_rb_mixer': return 14000
    if name == 'front_wing_mixer': return 18000
    if name == 'rear_wing_no_mod': return 13000
    if name == 'rear_wing_no_mod._drs': return 4500
    if name == 'body_top_wing': return 1800
    if name == 'racer': return 17000
    if name == 'shell': return 6500
    if node == 'helmet': return 450 if 'vizor' not in name else 1000
    if node.startswith('wheel_'):
        if name.startswith('wheel.'): return 11500
        if 'rim_' in name: return 2200
        if 'cover_' in name: return 1800
        if 'ventil' in name: return 120
        return 450
    if node == 'steering_wheel':
        if name.startswith('paddle'): return 350
        if 'body.' in name: return 17000
        if 'top_buttons' in name: return 10000
        if 'grip' in name: return 3000
        if 'knob' in name or 'switch' in name: return 1800
        if 'leds' in name: return 1600
        return 800
    if 'mirror_main' in name: return 2000
    if 'cockpit_inner' in name: return 2200
    if 'seatbelt_mechanism' in name: return 180
    if 'seat' in name: return 900
    if 'brake' in name: return 900
    if 'arm' in name or 'rod' in name: return 1100
    if 'led_' in name: return 250
    return 900


def material_for(name, old, centre, normal):
    lower = old.lower()
    if name == 'racer': return 'suit'
    if name == 'shell' and 'audi' in lower: return 'helmet_paint'
    if name.startswith('vizor') or 'polycarbonate' in lower: return 'visor'
    if name.startswith('shell') or 'helmet shell' in name or 'vent' in name: return 'rubber'
    if name == 'ferrari_dashboard_screen': return 'wheel_screen'
    if 'display_' in name: return 'wheel_screen'
    if 'leds' in name: return 'wheel_leds' if 'translucent' in lower else 'rubber'
    if 'rear_led_emmiter' in name: return 'emissive_red'
    if name.startswith('wheel.'): return 'tyre'
    if 'mirror_reflective' in name: return 'mirror'
    if name == 'body_object':
        # Cockpit tub walls face inward inside the opening; keep them bare carbon.
        if -1.1 < centre.y < -.05 and abs(centre.x) < .25 and .35 < centre.z < .78 and centre.x*normal.x < -.015:
            return 'carbon'
        # So is the underside. Only faces looking clearly down qualify, which
        # keeps the paint/carbon border off the curved lower sidepods.
        if normal.z < -.6:
            return 'carbon'
    if name in ('body_object', 'body_top_wing', 'body_side_top_wings', 'mirror_main_body_at01'):
        return 'paint'
    # Only endplate faces receive paint; the wing elements remain bare carbon.
    if name in ('front_wing_mixer', 'rear_wing_no_mod') and abs(centre.x) > (.78 if name.startswith('front') else .51) and abs(normal.x)>.45:
        return 'paint'
    if 'rubber' in lower or 'plastic' in lower or 'felt' in lower or 'fabric' in lower: return 'rubber'
    if any(s in lower for s in ('metal', 'chrome', 'brass', 'degradable')): return 'metal'
    return 'carbon'


def unwrap(obj, mat_name):
    # Six continuous projection regions avoid thousands of tiny smart-UV islands
    # on decimated curved surfaces. Angle-based unwrap preserves density on the concave sidepods.
    select([obj])
    bpy.context.tool_settings.mesh_select_mode = (False, False, True)
    bpy.ops.object.mode_set(mode='EDIT')
    bm = bmesh.from_edit_mesh(obj.data)
    bm.normal_update()
    painted = {f for f in bm.faces if obj.data.materials[f.material_index].name == mat_name}
    labels = {}
    for face in painted:
        normal = sum((v.normal for v in face.verts), Vector()).normalized()
        axis = max(range(3), key=lambda i: abs(normal[i]))
        labels[face] = axis*2 + int(normal[axis]>0)
    # Absorb small disconnected patches into their largest neighbouring region.
    for _ in range(2):
        unseen = set(painted)
        while unseen:
            start = unseen.pop(); region = [start]; todo = [start]
            while todo:
                face = todo.pop()
                for edge in face.edges:
                    for neighbour in edge.link_faces:
                        if neighbour in unseen and labels[neighbour] == labels[start]:
                            unseen.remove(neighbour); todo.append(neighbour); region.append(neighbour)
            if len(region)<30:
                outside = [n for f in region for e in f.edges for n in e.link_faces if n in painted and labels[n]!=labels[start]]
                if outside:
                    label=max(set(labels[n] for n in outside),key=lambda label:sum(labels[n]==label for n in outside))
                    for f in region: labels[f]=label
    for face in bm.faces: face.select_set(face in painted)
    for edge in bm.edges:
        linked=[f for f in edge.link_faces if f in painted]
        edge.seam=len(linked)!=2 or labels[linked[0]]!=labels[linked[1]]
    bmesh.update_edit_mesh(obj.data)
    bpy.ops.uv.unwrap(method='ANGLE_BASED', margin=0)
    bpy.ops.uv.pack_islands(rotate=True, scale=True, margin_method='FRACTION', margin=.008)
    bpy.ops.object.mode_set(mode='OBJECT')


def repair_uv_faces(obj, mat_name, folded, separate=False):
    """Cut only raster-proven folds into planar islands, retaining every other UV."""
    mesh = obj.data
    uv = mesh.uv_layers.active.data
    world_area = uv_area = 0
    for tri in mesh.loop_triangles:
        if mesh.materials[tri.material_index].name != mat_name: continue
        a,b,c = [mesh.vertices[i].co for i in tri.vertices]
        world_area += (b-a).cross(c-a).length/2
        a,b,c = [Vector(uv[i].uv) for i in tri.loops]
        uv_area += abs((b-a).cross(c-a))/2
    density = math.sqrt(uv_area/max(world_area,1e-9))
    # Keep neighbouring repair faces in a shared planar chart. Isolating every
    # triangle wastes most of a 2048 atlas on padding around tiny islands.
    adjacency={index:set() for index in folded}
    edges={}
    for index in folded:
        face=mesh.polygons[index]
        for a,b in face.edge_keys:
            key=tuple(sorted((a,b)))
            if key in edges:
                other=edges[key]; adjacency[index].add(other); adjacency[other].add(index)
            else: edges[key]=index
    remaining=set(folded); chart=0
    while remaining:
        seed=remaining.pop(); group=[seed]; todo=[seed]
        normal=mesh.polygons[seed].normal.normalized()
        while todo:
            current=todo.pop()
            for neighbour in adjacency[current]:
                if not separate and neighbour in remaining and mesh.polygons[neighbour].normal.dot(normal)>.65:
                    remaining.remove(neighbour); group.append(neighbour); todo.append(neighbour)
        helper=Vector((1,0,0)) if abs(normal.x)<.9 else Vector((0,1,0))
        tangent=normal.cross(helper).normalized(); bitangent=normal.cross(tangent)
        offset=2+chart*2; chart+=1
        for index in group:
            face=mesh.polygons[index]
            for loop in face.loop_indices:
                co=mesh.vertices[mesh.loops[loop].vertex_index].co
                uv[loop].uv=(offset+co.dot(tangent)*density,offset+co.dot(bitangent)*density)
    print('UV REPAIR CHARTS',mat_name,chart,'for',len(folded),'faces',flush=True)
    select([obj])
    bpy.context.tool_settings.mesh_select_mode=(False,False,True)
    bpy.ops.object.mode_set(mode='EDIT')
    bm=bmesh.from_edit_mesh(mesh)
    for face in bm.faces:
        selected=mesh.materials[face.material_index].name==mat_name
        face.select_set(selected)
        face.hide_set(not selected)
        if selected:
            for loop in face.loops: loop.uv_select_vert_set(True)
    bmesh.update_edit_mesh(mesh)
    bpy.ops.uv.pack_islands(rotate=True,scale=True,margin_method='FRACTION',margin=.008)
    for face in bm.faces: face.hide_set(False)
    bmesh.update_edit_mesh(mesh)
    bpy.ops.object.mode_set(mode='OBJECT')


def overlapping_uv_faces(obj, mat_name):
    """Test triangle intersections in UV space, including folds below one texel."""
    mesh=obj.data; mesh.calc_loop_triangles(); uv=mesh.uv_layers.active.data
    buckets={}; previous=[]; folded=set()
    def cross(a,b,c):
        return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
    def intersection_area(first, second):
        polygon=list(first)
        sign=1 if cross(*second)>0 else -1
        for a,b in zip(second,second[1:]+second[:1]):
            if not polygon: return 0
            clipped=[]; last=polygon[-1]; dlast=sign*cross(a,b,last)
            for point in polygon:
                distance=sign*cross(a,b,point)
                if (distance>=0)!=(dlast>=0):
                    t=dlast/(dlast-distance)
                    clipped.append((last[0]+t*(point[0]-last[0]),last[1]+t*(point[1]-last[1])))
                if distance>=0: clipped.append(point)
                last=point; dlast=distance
            polygon=clipped
        return abs(sum(a[0]*b[1]-a[1]*b[0] for a,b in zip(polygon,polygon[1:]+polygon[:1])))/2
    for triangle in mesh.loop_triangles:
        if mesh.materials[triangle.material_index].name!=mat_name: continue
        points=[uv[i].uv[:] for i in triangle.loops]
        if abs(cross(*points))<1e-14: continue
        low=(min(p[0] for p in points),min(p[1] for p in points))
        high=(max(p[0] for p in points),max(p[1] for p in points))
        cells=[(x,y) for x in range(math.floor(low[0]*64),math.floor(high[0]*64)+1)
                    for y in range(math.floor(low[1]*64),math.floor(high[1]*64)+1)]
        candidates={index for cell in cells for index in buckets.get(cell,())}
        for index in candidates:
            other,lo,hi,face=previous[index]
            if low[0]>=hi[0] or low[1]>=hi[1] or high[0]<=lo[0] or high[1]<=lo[1]: continue
            if intersection_area(points,other)>1e-12:
                folded.update((triangle.polygon_index,face))
        index=len(previous); previous.append((points,low,high,triangle.polygon_index))
        for cell in cells: buckets.setdefault(cell,[]).append(index)
    return folded


def undersampled_uv_faces(obj, mat_name):
    """Reject collapsed or severely stretched UVs on otherwise visible geometry."""
    mesh=obj.data; mesh.calc_loop_triangles(); uv=mesh.uv_layers.active.data
    areas=[]
    for triangle in mesh.loop_triangles:
        if mesh.materials[triangle.material_index].name!=mat_name: continue
        a,b,c=[mesh.vertices[i].co for i in triangle.vertices]
        world=(b-a).cross(c-a).length/2
        a,b,c=[Vector(uv[i].uv) for i in triangle.loops]
        texture=abs((b-a).cross(c-a))/2
        areas.append((triangle.polygon_index,world,texture))
    density=sum(t for _,_,t in areas)/max(sum(w for _,w,_ in areas),1e-9)
    return {index for index,world,texture in areas if world>1e-7 and
            (texture<world*density*.12 or texture>world*density*8)}


def bake(obj, mat_name, output, extents, attempt=0):
    if attempt==0:
        for iteration in range(8):
            folded=overlapping_uv_faces(obj,mat_name)|undersampled_uv_faces(obj,mat_name)
            print('UV INTERSECTION CHECK',mat_name,len(folded),'overlapping faces',flush=True)
            if not folded: break
            repair_uv_faces(obj,mat_name,folded,separate=iteration>=2)
        else: raise RuntimeError(mat_name+': continuous UV overlap remains')
    mesh = obj.data
    mesh.calc_loop_triangles()
    uv = mesh.uv_layers.active.data
    pos = np.zeros((ATLAS, ATLAS, 3), np.float32)
    norm = np.zeros_like(pos)
    coverage = np.zeros((ATLAS, ATLAS), np.uint8)
    overlaps = 0
    owner = np.full((ATLAS,ATLAS),-1,np.int32)
    folded = set()
    for tri in mesh.loop_triangles:
        if mesh.materials[tri.material_index].name != mat_name:
            continue
        coords = np.array([uv[i].uv[:] for i in tri.loops])
        coords[:, 0] *= ATLAS
        coords[:, 1] = (1 - coords[:, 1]) * ATLAS
        low = np.maximum(np.floor(coords.min(axis=0)).astype(int), 0)
        high = np.minimum(np.ceil(coords.max(axis=0)).astype(int), ATLAS - 1)
        if np.any(high < low): continue
        a, b, c = coords
        det = (b[1]-c[1])*(a[0]-c[0]) + (c[0]-b[0])*(a[1]-c[1])
        if abs(det) < 1e-9: continue
        yy, xx = np.mgrid[low[1]:high[1]+1, low[0]:high[0]+1] + .5
        w0 = ((b[1]-c[1])*(xx-c[0])+(c[0]-b[0])*(yy-c[1]))/det
        w1 = ((c[1]-a[1])*(xx-c[0])+(a[0]-c[0])*(yy-c[1]))/det
        w2 = 1-w0-w1
        inside = (w0 >= -1e-7) & (w1 >= -1e-7) & (w2 >= -1e-7)
        y, x = np.nonzero(inside)
        y += low[1]; x += low[0]
        weights = np.stack((w0[inside],w1[inside],w2[inside]), axis=-1)
        vertices = [mesh.vertices[i] for i in tri.vertices]
        pp = np.array([gltf(obj.matrix_world @ v.co) for v in vertices])
        nn = np.array([gltf(obj.matrix_world.to_3x3() @ v.normal) for v in vertices])
        conflict = (coverage[y,x]!=0) & (np.linalg.norm(pos[y,x] - weights @ pp,axis=-1)>.00001)
        overlaps += int(np.count_nonzero(conflict))
        if np.any(conflict):
            folded.add(tri.polygon_index)
            folded.update(int(i) for i in owner[y[conflict],x[conflict]])
        owner[y,x] = tri.polygon_index
        pos[y,x] = weights @ pp
        norm[y,x] = weights @ nn
        coverage[y,x] = 1
    if overlaps:
        if attempt>=10: raise RuntimeError(f'{mat_name}: {overlaps} overlapping UV texels after repair')
        print('UV REPAIR',mat_name,len(folded),'folded faces;',overlaps,'overlapping texels',flush=True)
        repair_uv_faces(obj,mat_name,folded,separate=True)
        return bake(obj,mat_name,output,extents,attempt+1)
    norm /= np.maximum(np.linalg.norm(norm, axis=-1, keepdims=True), 1e-8)
    np.savez_compressed(output / (mat_name + '.npz'), position=pos, normal=norm,
                        mask=coverage, extents=np.array([extents['min'], extents['max']]))
    print('BAKE', mat_name, int(coverage.sum()), 'covered texels; overlaps', overlaps, flush=True)


def add_compound_rings(nodes, mats, scene):
    # Generic compound rings contain no branding or lettering.
    for key in WHEEL_SOURCE:
        obj = nodes['wheel_'+key]
        vertices, faces = [], []
        side = 1 if key.endswith('l') else -1
        outer = max(side*v.co.x for v in obj.data.vertices)
        for i in range(192):
            theta = i*math.tau/192
            for radius in (.316,.323):
                vertices.append((side*(outer-.014), radius*math.cos(theta), radius*math.sin(theta)))
        for i in range(192):
            a,b=2*i,2*((i+1)%192)
            faces.append((a+1,b+1,b,a) if side>0 else (a,b,b+1,a+1))
        mesh=bpy.data.meshes.new('Compound ring')
        mesh.from_pydata(vertices,[],faces)
        mesh.materials.append(mats['tyre_marking'])
        ring=bpy.data.objects.new('Compound ring',mesh)
        scene.collection.objects.link(ring)
        ring.location=obj.location
        select([obj,ring])
        bpy.ops.object.join()



def export_models(nodes, args, metadata):
    slots = [m.name for m in nodes['body'].data.materials]
    for filename, budget in [('car.glb',299000),('car_lod1.glb',88000)]:
        if filename == 'car_lod1.glb':
            total = sum(triangles(o) for o in nodes.values())
            for obj in nodes.values(): decimate(obj, int(triangles(obj)*budget/total))
            # Three wheel materials collapse to two for distant opponents.
            for obj in nodes.values():
                for p in obj.data.polygons:
                    mat = obj.data.materials[p.material_index].name
                    if mat in ('mirror','emissive_red','rubber'): p.material_index=slots.index('carbon')
                    elif mat in ('metal','tyre_marking'): p.material_index=slots.index('carbon')
        total = sum(triangles(o) for o in nodes.values())
        if total > budget:
            for obj in nodes.values(): decimate(obj,int(triangles(obj)*budget/total*.99))
        # Driver poses ride along as vertex attributes until this point.
        morphed = [obj for obj in nodes.values() if driver_steer.to_shape_keys(obj)]
        select(list(nodes.values()))
        bpy.ops.export_scene.gltf(filepath=str((args.output/filename).resolve()),export_format='GLB',
            use_selection=True,export_yup=True,export_apply=True,export_animations=False,
            export_cameras=False,export_lights=False,export_materials='EXPORT',export_extras=False,
            export_morph=True,export_morph_normal=True)
        for obj in morphed: driver_steer.clear_shape_keys(obj)
        if filename == 'car.glb':
            # The exporter removes a handful of degenerate source triangles.
            # Metadata describes the shipped artifact, including those removals.
            import struct
            data = (args.output/filename).read_bytes()
            length = struct.unpack_from('<I',data,12)[0]
            document = json.loads(data[20:20+length])
            metadata['faces'] = {node['name']:sum(document['accessors'][p['indices']]['count']//3 for p in document['meshes'][node['mesh']]['primitives']) for node in document['nodes']}
            (args.output/'car.json').write_text(json.dumps(metadata,indent=2)+'\n')
        print('EXPORT',filename,sum(triangles(o) for o in nodes.values()),flush=True)



def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('output', type=Path)
    parser.add_argument('--bake', type=Path, default=Path('raw/gen22/bake'))
    parser.add_argument('--rebake', action='store_true', help='Rebake and export an existing prepared.blend')
    parser.add_argument('--export-only', action='store_true', help='Export an existing prepared.blend after material/LOD changes')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    args.output.mkdir(parents=True, exist_ok=True)
    args.bake.mkdir(parents=True, exist_ok=True)
    if args.export_only or args.rebake:
        nodes = {name:bpy.data.objects[name] for name in NODES}
        metadata = json.loads((args.output/'car.json').read_text())
        for text in list(bpy.data.texts): bpy.data.texts.remove(text)
        if args.rebake:
            for name,mat in [('body','paint'),('driver','suit'),('helmet','helmet_paint')]:
                bake(nodes[name],mat,args.bake,metadata['extents'])
            bpy.ops.wm.save_as_mainfile(filepath=str((args.bake/'prepared.blend').resolve()))
        export_models(nodes,args,metadata)
        return
    scene = bpy.context.scene
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
    # Read names before deleting source materials.
    source_materials = {o.name:[m.name if m else '' for m in o.data.materials] for o in candidates}
    # Bake steering/shift poses first; capture() leaves the rig straight ahead.
    poses = driver_steer.capture([o for o in candidates if o.name not in skip and node_for(o) in ('driver','helmet')])
    deps = bpy.context.evaluated_depsgraph_get()
    records = []
    for source in candidates:
        if source.name in skip: continue
        node = node_for(source)
        mesh = bpy.data.meshes.new_from_object(source.evaluated_get(deps), preserve_all_data_layers=True, depsgraph=deps)
        mesh.transform(correction @ source.matrix_world)
        obj = bpy.data.objects.new('converted_' + source.name, mesh)
        scene.collection.objects.link(obj)
        obj['source_indices'] = [p.material_index for p in mesh.polygons]
        if source.name in poses:
            driver_steer.attach(obj, poses[source.name], correction.to_3x3())
        records.append((obj, source.name, node))
    # Leave only evaluated meshes. No lights, ground, rig, animation, or source code.
    keep = {obj for obj,_,_ in records}
    for obj in list(bpy.data.objects):
        if obj not in keep: bpy.data.objects.remove(obj, do_unlink=True)
    ground = min(v.co.z for obj,_,n in records if n.startswith('wheel_') for v in obj.data.vertices)
    shift = Matrix.Translation((0,0,-ground))
    correction = shift @ correction
    for obj,_,_ in records: obj.data.transform(shift)
    mats = make_materials()
    slots = list(mats)
    for obj, name, node in records:
        old = source_materials[name]
        indices = list(obj['source_indices'])
        obj.data.materials.clear()
        for mat in mats.values(): obj.data.materials.append(mat)
        for p, i in zip(obj.data.polygons, indices):
            p.material_index = slots.index(material_for(name, old[i] if i < len(old) else '', p.center, p.normal))
            p.use_smooth = True
        decimate(obj, target_for(name, node))
        if name in ('front_wing_mixer','rear_wing_no_mod'):
            # Cut an exact paint/carbon boundary through the curved endplates.
            limit = .85 if name.startswith('front') else .51
            bm = bmesh.new(); bm.from_mesh(obj.data)
            for plane in (-limit,limit):
                bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),
                    dist=1e-6,plane_co=(plane,0,0),plane_no=(1,0,0))
            for face in bm.faces:
                face.material_index=slots.index('paint' if abs(face.calc_center_median().x)>limit-1e-6 else 'carbon')
            bm.to_mesh(obj.data); bm.free()
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
        # Planar metre-scaled UVs for subtle carbon weave; painted faces are unwrapped below.
        if not obj.data.uv_layers: obj.data.uv_layers.new(name='UVMap')
        uv = obj.data.uv_layers.active.data
        for p in obj.data.polygons:
            axis = max(range(3), key=lambda i: abs(p.normal[i]))
            axes = [i for i in range(3) if i != axis]
            for loop in p.loop_indices:
                co = obj.data.vertices[obj.data.loops[loop].vertex_index].co
                uv[loop].uv = (co[axes[0]]*28, co[axes[1]]*28)
    # Reserve a strict combined budget for the posed driver and helmet.
    decimate(nodes['helmet'], 16000)
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
    bpy.ops.wm.save_as_mainfile(filepath=str((args.bake/'geometry.blend').resolve()))
    # Dedicated, non-overlapping atlases across every painted face.
    for name, mat in [('body','paint'),('driver','suit'),('helmet','helmet_paint')]:
        unwrap(nodes[name],mat)
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
    # Paddles hinge on the wheel and turn with it.
    for name,hinge in hinges.items():
        nodes[name].data.transform(Matrix.Translation(-hinge))
        # Local to the (unrotated) wheel; matrix_world is stale until the next update.
        nodes[name].parent = nodes['steering_wheel']
        nodes[name].matrix_parent_inverse = Matrix.Identity(4)
        nodes[name].location = hinge - steering
    add_compound_rings(nodes, mats, scene)
    bpy.context.view_layer.update()
    # Metadata is in glTF space; Blender +Z maps to glTF +Y.
    points = [o.matrix_world @ v.co for o in nodes.values() for v in o.data.vertices]
    low = [min(p[i] for p in points) for i in range(3)]
    high = [max(p[i] for p in points) for i in range(3)]
    extents = dict(min=gltf((low[0],high[1],low[2])), max=gltf((high[0],low[1],high[2])))
    metadata = dict(wheels={},extents=extents,faces={n:triangles(o) for n,o in nodes.items()},
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
    for name,mat in [('body','paint'),('driver','suit'),('helmet','helmet_paint')]:
        bake(nodes[name],mat,args.bake,extents)
    (args.output/'car.json').write_text(json.dumps(metadata,indent=2)+'\n')
    # This intermediate is local review data, never a shipping asset.
    for text in list(bpy.data.texts): bpy.data.texts.remove(text)
    # Preserve converted geometry for atlas inspection, outside shipped outputs.
    bpy.ops.wm.save_as_mainfile(filepath=str((args.bake/'prepared.blend').resolve()))
    export_models(nodes, args, metadata)


if __name__ == '__main__': main()
