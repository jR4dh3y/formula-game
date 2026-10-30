"""Eevee review of the actual exported GLB, plus isolated source comparisons.

All Blender invocations must include --disable-autoexec. Use --factory-startup
for exported-car review; use the source blend only with --source-comparison.
"""
import argparse
import json
from pathlib import Path
import sys
import math

import bpy
from mathutils import Vector, Matrix


def aim(obj, target):
    obj.rotation_euler=(Vector(target)-obj.location).to_track_quat('-Z','Y').to_euler()


def studio(output):
    scene=bpy.context.scene
    # Eevee on the installed Blender uses the EEVEE identifier.
    scene.render.engine='BLENDER_EEVEE'
    scene.eevee.taa_render_samples=128
    scene.eevee.shadow_ray_count=4
    scene.render.resolution_x=1280
    scene.render.resolution_y=800
    scene.render.resolution_percentage=100
    scene.render.image_settings.file_format='PNG'
    scene.render.film_transparent=False
    scene.world=bpy.data.worlds.new('Review studio')
    scene.world.use_nodes=True
    scene.world.node_tree.nodes.get('Background').inputs[0].default_value=(.14,.17,.22,1)
    scene.world.node_tree.nodes.get('Background').inputs[1].default_value=.5
    scene.view_settings.view_transform='AgX'
    for location,power,size in [((1,-3,6),2200,5),((-4,0,3),1500,4),((2,4,5),2600,4)]:
        data=bpy.data.lights.new('Softbox','AREA');data.energy=power;data.shape='DISK';data.size=size
        obj=bpy.data.objects.new('Softbox',data);scene.collection.objects.link(obj);obj.location=location;aim(obj,(0,0,.4))
    bpy.ops.mesh.primitive_plane_add(size=200,location=(0,0,-.012))
    ground=bpy.context.object;ground.name='Review ground'
    mat=bpy.data.materials.new('Review grey');mat.use_nodes=True
    shader=mat.node_tree.nodes.get('Principled BSDF')
    shader.inputs['Base Color'].default_value=(.045,.055,.07,1)
    shader.inputs['Roughness'].default_value=.8
    ground.data.materials.append(mat)
    data=bpy.data.cameras.new('Review camera');camera=bpy.data.objects.new('Review camera',data);scene.collection.objects.link(camera);scene.camera=camera
    data.lens=55
    output.mkdir(parents=True,exist_ok=True)
    return scene,camera


def shot(scene,camera,path,location,target,lens=55):
    camera.location=location;camera.data.lens=lens;aim(camera,target)
    scene.render.filepath=str(path.resolve())
    bpy.ops.render.render(write_still=True)
    print('RENDER',path,flush=True)


def set_livery(directory,team):
    data=json.loads((directory/'teams.json').read_text())
    entry=next(t for t in data['teams'] if t['id']==team)
    images={'paint':directory/'liveries'/team/'paint.png','suit':directory/'liveries'/team/'suit.png',
            'helmet_paint':directory/'liveries'/team/entry['drivers'][0]['id']/'helmet.png'}
    for name,path in images.items():
        mat=bpy.data.materials[name]
        shader=mat.node_tree.nodes.get('Principled BSDF')
        tex=mat.node_tree.nodes.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(path.resolve()))
        mat.node_tree.links.new(tex.outputs['Color'],shader.inputs['Base Color'])


def source_comparison(args):
    # Retain only visible steering-wheel or front-wing meshes, with the exact posed
    # evaluated source geometry and no embedded node-group execution or textures.
    deps=bpy.context.evaluated_depsgraph_get()
    records=[]
    for source in list(bpy.data.objects):
        if source.type!='MESH' or source.hide_render: continue
        group='steering' if source.parent_bone=='steering_wheel' else 'wing' if source.name=='front_wing_mixer' else None
        if group is None or source.name.startswith('bee+') or 'display_glass' in source.name or 'display_emmiter' in source.name: continue
        mesh=bpy.data.meshes.new_from_object(source.evaluated_get(deps),depsgraph=deps)
        mesh.transform(source.matrix_world)
        obj=bpy.data.objects.new('Reference '+source.name,mesh);bpy.context.scene.collection.objects.link(obj)
        records.append((obj,group))
    keep={o for o,_ in records}
    for obj in list(bpy.data.objects):
        if obj not in keep: bpy.data.objects.remove(obj,do_unlink=True)
    grey=bpy.data.materials.new('Neutral reference');grey.use_nodes=True
    grey.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.12,.14,.17,1)
    grey.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value=.6
    for obj,_ in records:
        obj.data.materials.clear();obj.data.materials.append(grey)
        for p in obj.data.polygons: p.material_index=0;p.use_smooth=True
    scene,camera=studio(args.output)
    for group,loc,target in [('steering',(.31,2.13,.85),(.055,1.60,.60)),('wing',(2.25,-1.905,.995),(.70,-.375,.245))]:
        for obj,g in records: obj.hide_render=g!=group
        shot(scene,camera,args.output/('source-'+group+'.png'),loc,target,70)


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('directory',type=Path)
    parser.add_argument('--output',type=Path,default=Path('/tmp/gen22-previews'))
    parser.add_argument('--source-comparison',action='store_true')
    parser.add_argument('--teams',nargs='+',default=['redbull','ferrari','mclaren','mercedes'])
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    if args.source_comparison:
        source_comparison(args);return
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str((args.directory/'car.glb').resolve()))
    scene,camera=studio(args.output)
    metadata=json.loads((args.directory/'car.json').read_text())
    eye=metadata['cockpit_eye']
    cockpit=(eye[0],-eye[2],eye[1])
    views=[('front',(6,-8,3.4),(0,-.1,.4),55),('rear',(-6,8,3.5),(0,.15,.45),55),
           ('side',(10,0,1.8),(0,0,.4),55),('top',(0,-.38,14.5),(0,-.38,0),48),
           ('cockpit',cockpit,(cockpit[0],cockpit[1]-1.48,cockpit[2]-.32),20)]
    for team in args.teams:
        set_livery(args.directory,team)
        for view,loc,target,lens in views:
            bpy.data.objects['helmet'].hide_render=view=='cockpit'
            shot(scene,camera,args.output/(team+'-'+view+'.png'),loc,target,lens)
    bpy.data.objects['helmet'].hide_render=False
    grey=bpy.data.materials.new('Neutral comparison');grey.use_nodes=True
    grey.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.12,.14,.17,1)
    grey.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value=.6
    bpy.context.view_layer.material_override=grey
    # Identical subject-relative views to source comparisons, using exported geometry.
    for group,loc,target in [('steering',(.255,-.265,.855),(0,-.795,.605)),('wing',(2.2,-4.3,1.0),(.65,-2.77,.25))]:
        for obj in bpy.context.scene.objects:
            if obj.type=='MESH' and obj.name!='Review ground': obj.hide_render=(obj.name!='steering_wheel') if group=='steering' else obj.name!='body'
        shot(scene,camera,args.output/('export-'+group+'.png'),loc,target,70)
    bpy.context.view_layer.material_override=None
    # Prove the selected DRS opening direction on the exported hinged element.
    for obj in bpy.context.scene.objects:
        if obj.type=='MESH': obj.hide_render=False
    bpy.data.objects['drs_flap'].rotation_euler.x=-.25
    shot(scene,camera,args.output/'drs-open.png',(-3,5,1.8),(0,1.9,.7),70)


if __name__=='__main__': main()
