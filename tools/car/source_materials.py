"""Original-UV source artwork and portable PBR replacements for Cycles materials.

UV0 retains source artwork and AO. UV1 is the direct template on paint and
metre-scaled detail on unpainted composite. Paint detail normals bake onto UV0.
"""
import math
from pathlib import Path
import shutil

import bpy
import numpy as np

PARTS = {
    'body_object': 'main_body_stickers',
    'floor_rb_mixer': 'floor',
    'front_wing_mixer': 'front_wing',
    'rear_wing_no_mod': 'rear_wing',
    'rear_wing_no_mod._drs': 'rear_wing',
    'body_top_wing': 'winglets',
    'body_side_top_wings': 'winglets',
    'mirror_main_body_at01': 'mirrors',
    'cockpit_inner': 'cockpit_foam',
}
AO = {'main_body_stickers': 'main_body'}


def material(name, color, roughness=.4, metallic=0):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    shader = mat.node_tree.nodes.get('Principled BSDF')
    shader.inputs['Base Color'].default_value = (*color, 1)
    shader.inputs['Roughness'].default_value = roughness
    shader.inputs['Metallic'].default_value = metallic
    mat.diffuse_color = (*color, 1)
    return mat


def image_node(mat, image, uv='OriginalUV'):
    node = mat.node_tree.nodes.new('ShaderNodeTexImage')
    node.image = image
    coords = mat.node_tree.nodes.new('ShaderNodeUVMap')
    coords.uv_map = uv
    mat.node_tree.links.new(coords.outputs['UV'], node.inputs['Vector'])
    return node


def generated_image(name, rgba, noncolor=False):
    height, width = rgba.shape[:2]
    image = bpy.data.images.new(name, width=width, height=height, alpha=True)
    if noncolor: image.colorspace_settings.name = 'Non-Color'
    image.pixels.foreach_set(np.asarray(rgba, dtype=np.float32).ravel())
    image.pack()
    return image


def attach_normal(mat, image, uv='OriginalUV', strength=1):
    nodes, links = mat.node_tree.nodes, mat.node_tree.links
    tex = image_node(mat, image, uv)
    normal = nodes.new('ShaderNodeNormalMap')
    normal.uv_map = uv
    normal.inputs['Strength'].default_value = strength
    links.new(tex.outputs['Color'], normal.inputs['Color'])
    links.new(normal.outputs['Normal'], nodes.get('Principled BSDF').inputs['Normal'])


class Library:
    def __init__(self, directory):
        self.directory = directory.resolve()
        self.cache = {}
        self.images = {}
        self.originals = {mat.name: mat for mat in bpy.data.materials}
        # Recreate carbon twill as a tangent normal, rather than exporting Cycles nodes.
        y, x = np.mgrid[:256, :256] / 16
        orientation = (np.floor(x) + np.floor(y)) % 4 < 2
        height = np.where(orientation, np.sin(x*math.tau), np.sin(y*math.tau))*.12
        dy, dx = np.gradient(height)
        normal = np.stack((-dx*2, -dy*2, np.ones_like(dx)), axis=-1)
        normal /= np.linalg.norm(normal, axis=-1, keepdims=True)
        self.carbon_normal = generated_image('carbon_twill_normal', np.concatenate((normal*.5+.5, np.ones((256,256,1))), axis=-1), True)
        rough = .32 + .07*np.where(orientation, np.cos(x*math.tau), np.cos(y*math.tau))
        self.carbon_rough = generated_image('carbon_twill_roughness', np.concatenate((np.repeat(rough[...,None],3,axis=2), np.ones((256,256,1))),axis=-1), True)

    def load(self, path, noncolor=False):
        key = (str(path), noncolor)
        if key not in self.images:
            image = bpy.data.images.load(str(path), check_existing=False)
            if max(image.size) > 4096: image.scale(4096, 4096)
            if noncolor: image.colorspace_settings.name = 'Non-Color'
            image.pack()
            self.images[key] = image
        return self.images[key]

    def for_part(self, part, old, node):
        part_texture = PARTS.get(part)
        if '_wheel_cover_' in part:
            part_texture = 'front_covers' if 'front' in part else 'rear_covers'
        if part_texture and ('Carbon' in old or 'Paint' in old):
            key = ('paint', part_texture)
        elif node == 'drs_button': key = ('drs',)
        elif part == 'ferrari_dashboard_screen' or 'display_' in part: key = ('screen',)
        elif part == 'live_rev_strip': key = ('leds',)
        elif part.startswith('wheel.'): key = ('tyre',)
        elif part == 'shell' and old == 'audi_white': key = ('helmet',)
        else: key = ('source', old)
        if key in self.cache: return self.cache[key]
        if key[0] == 'paint':
            mat = material('source_paint_'+part_texture, (.32,.34,.36), .3)
            shader = mat.node_tree.nodes.get('Principled BSDF')
            shader.inputs['Coat Weight'].default_value = .16
            ao_path = self.directory/'shadow_maps'/('shadow_map_'+AO.get(part_texture,part_texture)+'.png')
            if ao_path.is_file(): self.occlusion(mat, self.load(ao_path, True))
            attach_normal(mat, self.carbon_normal, 'DetailUV', .16)
        elif key[0] == 'screen': mat = material('wheel_screen', (.005,.008,.01), .5)
        elif key[0] == 'leds':
            mat = material('wheel_leds', (1,1,1), .25)
            # Same driver-left-to-right groups as the game's fifteen RPM lamps.
            strip = np.ones((1,90,4), dtype=np.float32)
            for group, color in enumerate(((.015,.8,.04),(.9,.015,.02),(.12,.035,1))):
                strip[:,group*30:(group+1)*30,:3] = color
            tex = image_node(mat, generated_image('rev_groups', strip))
            tex.interpolation = 'Closest'
            shader = mat.node_tree.nodes.get('Principled BSDF')
            mat.node_tree.links.new(tex.outputs['Color'], shader.inputs['Base Color'])
            mat.node_tree.links.new(tex.outputs['Color'], shader.inputs['Emission Color'])
            shader.inputs['Emission Strength'].default_value = .5
        elif key[0] == 'drs':
            mat = material('wheel_drs', (1,1,1), .28)
            cap=np.full((128,128,4),(.02,.4,.04,1),dtype=np.float32)
            glyphs=['11110/10001/10001/10001/10001/10001/11110',
                    '11110/10001/10001/11110/10100/10010/10001',
                    '01111/10000/10000/01110/00001/00001/11110']
            for letter,glyph in enumerate(glyphs):
                for y,row in enumerate(glyph.split('/')):
                    for x,bit in enumerate(row):
                        if bit=='1': cap[43+y*6:49+y*6,13+letter*36+x*6:19+letter*36+x*6,:3]=.95
            tex=image_node(mat,generated_image('drs_legend',np.flipud(cap)))
            mat.node_tree.links.new(tex.outputs['Color'],mat.node_tree.nodes.get('Principled BSDF').inputs['Base Color'])
        elif key[0] == 'tyre':
            raise RuntimeError('Tyres must be baked from the source before material replacement')
        elif key[0] == 'helmet':
            mat = material('source_helmet', (1,1,1), .25)
            tex = image_node(mat,self.load(self.directory/'textures_audi'/'leclerc_helmet_texture_paint.png'))
            mat.node_tree.links.new(tex.outputs['Color'],mat.node_tree.nodes.get('Principled BSDF').inputs['Base Color'])
            self.occlusion(mat,self.load(self.directory/'shadow_maps'/'gt3_helmet_shadow_map.png',True))
        else:
            mat = self.recreate(old)
        self.cache[key] = mat
        return mat

    def occlusion(self, mat, image):
        # Blender's glTF exporter recognises this dedicated input as occlusionTexture.
        group = bpy.data.node_groups.get('glTF Material Output')
        if group is None:
            group = bpy.data.node_groups.new('glTF Material Output','ShaderNodeTree')
            group.interface.new_socket(name='Occlusion',in_out='INPUT',socket_type='NodeSocketFloat')
        node = mat.node_tree.nodes.new('ShaderNodeGroup'); node.node_tree=group
        tex = image_node(mat,image)
        mat.node_tree.links.new(tex.outputs['Color'],node.inputs['Occlusion'])

    def recreate(self, old):
        source = self.originals[old]
        color, rough, metal = (.018,.02,.024), .48, 0
        lower = old.lower()
        if 'brass' in lower: color,rough,metal=(.65,.38,.055),.22,1
        elif 'anodized' in lower: color,rough,metal=(.145,.145,.145),.23,1
        elif 'chrome' in lower or 'metal' in lower: color,rough,metal=(.55,.57,.6),.2,1
        elif 'white' in lower: color,rough=(.81,.81,.81),.4
        elif 'translucent' in lower:
            group = next(n for n in source.node_tree.nodes if n.type=='GROUP')
            color,rough=tuple(group.inputs['Base Color'].default_value[:3]),.24
        elif 'rubber' in lower: color,rough=(.012,.014,.016),.8
        elif 'felt' in lower or old in ('blacklather','leather2','helmet','suit','Fabric_seatbelts'): color,rough=(.035,.038,.042),.8
        elif 'polycarbonate' in lower or 'glass' in lower: color,rough,metal=(.035,.055,.07),.12,.35
        else:
            shader = next((n for n in source.node_tree.nodes if n.type=='BSDF_PRINCIPLED'),None)
            if shader and not shader.inputs['Base Color'].is_linked:
                color=tuple(shader.inputs['Base Color'].default_value[:3]); rough=shader.inputs['Roughness'].default_value; metal=shader.inputs['Metallic'].default_value
        mat=material('source_'+old, color, rough, metal)
        shader=mat.node_tree.nodes.get('Principled BSDF')
        if 'translucent' in lower:
            shader.inputs['Transmission Weight'].default_value=.15
            shader.inputs['Emission Color'].default_value=(*color,1)
            shader.inputs['Emission Strength'].default_value=.12
            shader.inputs['Coat Weight'].default_value=.25
        if 'carbon' in lower:
            shader.inputs['Base Color'].default_value=(.012,.015,.018,1)
            attach_normal(mat,self.carbon_normal,'DetailUV',.3)
            tex=image_node(mat,self.carbon_rough,'DetailUV')
            mat.node_tree.links.new(tex.outputs['Color'],shader.inputs['Roughness'])
        return mat

    @staticmethod
    def add_detail_uv(obj):
        layer=obj.data.uv_layers.new(name='DetailUV' if obj.data.uv_layers.get('livery') else 'livery')
        for face in obj.data.polygons:
            axis=max(range(3),key=lambda i: abs(face.normal[i]))
            axes=[i for i in range(3) if i!=axis]
            for index in face.loop_indices:
                point=obj.data.vertices[obj.data.loops[index].vertex_index].co
                layer.data[index].uv=(point[axes[0]]*56,point[axes[1]]*56)
        obj.data.uv_layers.active=obj.data.uv_layers['OriginalUV']
        obj.data.uv_layers['OriginalUV'].active_render=True


def finish_materials(nodes):
    """Match the source's dark composite without a runtime colour override."""
    used={mat for obj in nodes.values() for mat in obj.data.materials if mat}
    for mat in used:
        shader=mat.node_tree.nodes.get('Principled BSDF')
        if 'Carbon' in mat.name and not mat.name.startswith('source_wheel_'):
            shader.inputs['Specular IOR Level'].default_value=.18
        if 'Carbon' in mat.name and not mat.name.startswith('source_wheel_'):
            shader.inputs['Base Color'].default_value=(.004,.005,.007,1)
    images={node.image for mat in used for node in mat.node_tree.nodes if node.type=='TEX_IMAGE' and node.image}
    large_colour={'textures_audi_main_body_stickers','textures_audi_floor','textures_audi_front_wing','textures_audi_rear_wing'}
    for image in images:
        limit=4096 if image.name.split('.')[0] in large_colour or image.name.startswith('converted_steering_wheel_body') else 2048
        if max(image.size)>limit: image.scale(limit,limit)


def export_models(nodes, args, metadata, decimate, select, triangles):
    import sys
    sys.path.insert(0,str(Path(__file__).resolve().parent))
    import driver_steer
    finish_materials(nodes)
    bake_paint_detail(nodes, args.work)
    decimate(nodes['helmet'], 45000)
    for filename in ('car.glb','car_lod1.glb','car_lod2.glb'):
        if filename == 'car_lod1.glb':
            # Opponents have no wheel, controls or animated fingers. Keep a static driver.
            for name in ('steering_wheel','paddle_up','paddle_down','drs_button'):
                bpy.data.objects.remove(nodes.pop(name), do_unlink=True)
            for name in ('driver','helmet'):
                for attr in list(nodes[name].data.attributes):
                    if attr.name.startswith('steer_'): nodes[name].data.attributes.remove(attr)
            import bmesh
            body=nodes['body']; bm=bmesh.new(); bm.from_mesh(body.data)
            layer=bm.faces.layers.int.get('player_cockpit_detail')
            if layer:
                bmesh.ops.delete(bm,geom=[f for f in bm.faces if f[layer]],context='FACES')
            from mathutils import Matrix
            proxy_material=material('source_cockpit_proxy',(.002,.002,.002),.9)
            body.data.materials.append(proxy_material)
            proxy=bmesh.ops.create_cube(bm,size=1,
                matrix=Matrix.Translation((0,-.3,.42)) @ Matrix.Diagonal((.48,.85,.05,1)))
            proxy_vertices=set(proxy['verts'])
            for face in bm.faces:
                if all(vertex in proxy_vertices for vertex in face.verts):
                    face.material_index=len(body.data.materials)-1
            bm.to_mesh(body.data); bm.free()
            high={name:obj.copy() for name,obj in nodes.items()}
            for name,obj in high.items():
                obj.data=nodes[name].data.copy()
                bpy.context.scene.collection.objects.link(obj)
                obj.hide_render=True
            total=sum(triangles(obj) for obj in nodes.values())
            for name,obj in nodes.items():
                decimate(obj,max(100,int(triangles(obj)*140000/total)))
            bake_lod_normals(nodes,high,args.work)
            for obj in high.values(): bpy.data.objects.remove(obj,do_unlink=True)
            resize_images(nodes, 2048)
        elif filename == 'car_lod2.glb':
            total=sum(triangles(obj) for obj in nodes.values())
            for name,obj in nodes.items():
                decimate(obj,max(40,int(triangles(obj)*45000/total)))
            resize_images(nodes, 1024)
        morphed=[obj for obj in nodes.values() if driver_steer.to_shape_keys(obj)]
        select(list(nodes.values()))
        bpy.ops.export_scene.gltf(filepath=str((args.output/filename).resolve()),export_format='GLB',
            use_selection=True,export_yup=True,export_apply=True,export_animations=False,
            export_cameras=False,export_lights=False,export_materials='EXPORT',export_extras=False,
            export_morph=True,export_morph_normal=True,export_image_format='AUTO')
        for obj in morphed: driver_steer.clear_shape_keys(obj)
        print('EXPORTED',filename,sum(triangles(o) for o in nodes.values()),flush=True)
    shutil.copyfile(Path(__file__).with_name('teams.json'),args.output/'teams.json')


def bake_paint_detail(nodes, directory):
    """Free UV1 for the template without dropping the painted carbon normals.

    Bevy supports two UV channels. Bake the existing metre-scaled DetailUV
    normals in the OriginalUV tangent frame, then discard that temporary map.
    Unpainted primitives keep their metre-scaled coordinates in UV1.
    """
    import wheel_bake
    wheel_bake.setup()
    scene = bpy.context.scene
    scene.cycles.samples = 4
    scene.render.bake.use_selected_to_active = False
    for obj in nodes.values():
        painted = [m for m in obj.data.materials if m and m.name.startswith('source_paint_')]
        if not painted:
            continue
        targets = []
        scratch = bpy.data.images.new('detail_bake_scratch', 16, 16)
        scratch_nodes = []
        for mat in obj.data.materials:
            if mat in painted:
                image = bpy.data.images.new('detail_' + mat.name, 2048, 2048, alpha=False)
                image.generated_color = (.5, .5, 1, 1)
                image.colorspace_settings.name = 'Non-Color'
                tex = mat.node_tree.nodes.new('ShaderNodeTexImage')
                tex.image = image
                targets.append((mat, image))
            else:
                tex = mat.node_tree.nodes.new('ShaderNodeTexImage')
                tex.image = scratch
                scratch_nodes.append((mat, tex))
            mat.node_tree.nodes.active = tex
        obj.data.uv_layers.active = obj.data.uv_layers['OriginalUV']
        obj.data.uv_layers['OriginalUV'].active_render = True
        bpy.ops.object.select_all(action='DESELECT')
        obj.hide_set(False)
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
        print('BAKE PAINT DETAIL', obj.name, len(targets), flush=True)
        bpy.ops.object.bake(type='NORMAL')
        for mat, image in targets:
            image.filepath_raw = str((directory / (image.name + '.png')).resolve())
            image.file_format = 'PNG'
            image.save()
            image.pack()
            attach_normal(mat, image)
        for mat, tex in scratch_nodes:
            mat.node_tree.nodes.remove(tex)
        bpy.data.images.remove(scratch)
    for obj in nodes.values():
        detail = obj.data.uv_layers.get('DetailUV')
        if detail:
            obj.data.uv_layers.remove(detail)
    for mat in {m for obj in nodes.values() for m in obj.data.materials if m}:
        for node in mat.node_tree.nodes:
            if node.type in {'UVMAP', 'NORMAL_MAP'} and node.uv_map == 'DetailUV':
                node.uv_map = 'livery'


def resize_images(nodes, size):
    images={node.image for obj in nodes.values() for mat in obj.data.materials if mat
            for node in mat.node_tree.nodes if node.type=='TEX_IMAGE' and node.image}
    for image in images:
        if max(image.size)>size: image.scale(size,size)


def bake_lod_normals(nodes, high, directory):
    """Project player surfaces onto opponents in original UVs, per material.

    Painted body panels receive separate maps. Control artwork is protected
    during simplification. Mirrored
    source islands stay mirrored; carbon on other nodes retains its tileable map.
    """
    scene=bpy.context.scene
    scene.render.engine='CYCLES'
    scene.cycles.samples=1
    scene.cycles.bake_type='NORMAL'
    scene.render.bake.use_selected_to_active=True
    scene.render.bake.cage_extrusion=.004
    scene.render.bake.max_ray_distance=.012
    scene.render.bake.margin=8
    for name in ('body','drs_flap'):
        low=nodes[name]; source=high[name]
        # Baking the whole merged body into a single image loses the per-part layout.
        # Each painted material receives its own target, with all slots active together.
        targets=[]
        for index,mat in enumerate(list(low.data.materials)):
            if mat is None: continue
            if not mat.name.startswith('source_paint_'): continue
            if not any(p.material_index==index for p in low.data.polygons): continue
            copy=mat.copy();copy.name=mat.name+'_lod'
            low.data.materials[index]=copy
            image=bpy.data.images.new('lod_normal_'+mat.name,1024,1024,alpha=False)
            image.generated_color=(.5,.5,1,1);image.colorspace_settings.name='Non-Color'
            tex=copy.node_tree.nodes.new('ShaderNodeTexImage');tex.image=image
            copy.node_tree.nodes.active=tex
            targets.append((copy,image))
        if not targets: continue
        # Non-painted slots also need a bake target but do not ship that scratch image.
        scratch=bpy.data.images.new('normal_bake_scratch',16,16)
        scratch_nodes=[]
        for index,mat in enumerate(list(low.data.materials)):
            if any(mat==copy for copy,_ in targets): continue
            tex=mat.node_tree.nodes.new('ShaderNodeTexImage');tex.image=scratch
            mat.node_tree.nodes.active=tex
            scratch_nodes.append((mat,tex))
        bpy.ops.object.select_all(action='DESELECT')
        source.hide_set(False);source.hide_render=False;source.select_set(True)
        low.select_set(True);bpy.context.view_layer.objects.active=low
        print('BAKE LOD NORMAL',name,len(targets),flush=True)
        bpy.ops.object.bake(type='NORMAL')
        source.hide_render=True;source.hide_set(True)
        for mat,image in targets:
            image.filepath_raw=str((directory/(image.name+'.png')).resolve());image.file_format='PNG';image.save();image.pack()
            attach_normal(mat,image,strength=.35)
        for mat,tex in scratch_nodes: mat.node_tree.nodes.remove(tex)
        bpy.data.images.remove(scratch)
