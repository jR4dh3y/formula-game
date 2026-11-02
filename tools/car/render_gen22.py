"""Render the exported GLB and source blend under the same review lighting.

Always invoke Blender with -b --disable-autoexec. Source review evaluates the
original full-resolution mesh and shaders, with no material override.
"""
import argparse
import json
import math
from pathlib import Path
import sys
import tempfile

import bpy
from mathutils import Matrix, Vector


def aim(obj, target):
    obj.rotation_euler=(Vector(target)-obj.location).to_track_quat('-Z','Y').to_euler()


def studio(output):
    scene=bpy.context.scene
    scene.render.engine='BLENDER_EEVEE'
    scene.eevee.taa_render_samples=64
    scene.eevee.shadow_ray_count=2
    scene.render.threads_mode='FIXED'
    scene.render.threads=8
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
    output.mkdir(parents=True,exist_ok=True)
    return scene,camera


def source_meshes():
    sys.path.insert(0,str(Path(__file__).resolve().parent))
    import driver_steer
    import source_materials
    centres=[bpy.data.objects[name].matrix_world.translation.copy() for name in ('wheel.Ft.L','wheel.Ft.R','wheel.Bk.L','wheel.Bk.R')]
    front=(centres[0]+centres[1])/2;rear=(centres[2]+centres[3])/2
    heading=rear-front;rotation=Matrix.Rotation(math.atan2(heading.x,heading.y),4,'Z')
    mid=rotation@((front+rear)/2);correction=Matrix.Translation((-mid.x,-mid.y,0))@rotation
    sources=[obj for obj in bpy.data.collections['fp04_tr_model'].all_objects if obj.type=='MESH' and obj.data.polygons and not obj.hide_render]
    # The original rig starts turned. Use the same neutral wheel and driver pose.
    driver_steer.capture([])
    deps=bpy.context.evaluated_depsgraph_get()
    records=[]
    skip={'helmet','shoes.l','shoes.r','bee+_logo_grey-01','steering_wheel_display_glass.002','steering_wheel_display_emmiter.002'}
    for source in sources:
        if source.name in skip: continue
        mesh=bpy.data.meshes.new_from_object(source.evaluated_get(deps),preserve_all_data_layers=True,depsgraph=deps)
        obj=bpy.data.objects.new('Reference '+source.name,mesh);bpy.context.scene.collection.objects.link(obj)
        # Keep local coordinates for the source's Object/Generated shader inputs.
        obj.matrix_world=correction@source.matrix_world
        obj['wheel_reference']=source.parent_bone=='steering_wheel'
        obj['helmet_reference']='GT3 Racing Helmet  (New) PL' in [c.name for c in source.users_collection]
        records.append(obj)
    ground=min((o.matrix_world@v.co).z for o in records if 'wheel.' in o.name for v in o.data.vertices)
    for obj in records: obj.matrix_world=Matrix.Translation((0,0,-ground))@obj.matrix_world
    for obj in list(bpy.data.objects):
        if obj not in records: bpy.data.objects.remove(obj,do_unlink=True)
    # Apply the supplied original-UV artwork to the source reference as well.
    # Leave native Cycles shaders on all wheel/control hardware.
    textures=Path('raw/gen22/source-textures').resolve()
    for obj in records:
        name=obj.name.removeprefix('Reference ')
        part=source_materials.PARTS.get(name)
        if 'cover_' in name: part='front_covers' if 'front' in name else 'rear_covers'
        if not part: continue
        for slot,mat in enumerate(list(obj.data.materials)):
            if mat is None or 'Carbon' not in mat.name: continue
            mat=mat.copy();obj.data.materials[slot]=mat
            shader=next((n for n in mat.node_tree.nodes if n.type=='BSDF_PRINCIPLED'),None)
            if shader:
                tex=mat.node_tree.nodes.new('ShaderNodeTexImage')
                tex.image=bpy.data.images.load(str(textures/'textures_audi'/('textures_audi_'+part+'.png')))
                mat.node_tree.links.new(tex.outputs['Color'],shader.inputs['Base Color'])
    # Source embeds duplicate 8K/17K images. Bound review texture memory without
    # changing the full-resolution source geometry or its material graph.
    bpy.data.orphans_purge(do_recursive=True)
    for image in bpy.data.images:
        if image.users and max(image.size)>4096:
            width,height=image.size
            ratio=4096/max(width,height)
            image.scale(max(1,round(width*ratio)),max(1,round(height*ratio)))
    return records


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('directory',type=Path)
    parser.add_argument('--output',type=Path)
    parser.add_argument('--source-comparison',action='store_true')
    parser.add_argument('--lod',action='store_true')
    parser.add_argument('--lod2',action='store_true')
    parser.add_argument('--path-traced',action='store_true',help='match the source Cycles shader review')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    if args.output is None: args.output=Path(tempfile.mkdtemp(prefix='gen22-previews-'))
    if args.source_comparison:
        records=source_meshes();prefix='source'
    else:
        bpy.ops.wm.read_factory_settings(use_empty=True)
        bpy.ops.import_scene.gltf(filepath=str((args.directory/('car_lod2.glb' if args.lod2 else 'car_lod1.glb' if args.lod else 'car.glb')).resolve()))
        records=[obj for obj in bpy.context.scene.objects if obj.type=='MESH'];prefix='lod2' if args.lod2 else 'lod' if args.lod else 'export'
    scene,camera=studio(args.output)
    if args.source_comparison or args.path_traced:
        sys.path.insert(0,str(Path(__file__).resolve().parent))
        import wheel_bake
        wheel_bake.setup()
        scene.cycles.samples=32
    shots=[('full',(6,-8,3.4),(0,-.1,.4),55),
           ('steering',(.255,-.265,.855),(0,-.795,.605),70)]
    if not args.source_comparison:
        shots += [('rear',(-6,8,3.5),(0,.15,.45),55),('side',(10,0,1.8),(0,0,.4),55)]
    for view,loc,target,lens in shots:
        if view == 'steering' and (args.lod or args.lod2): continue
        for obj in records:
            wheel=obj.get('wheel_reference',False) if args.source_comparison else obj.name in ('steering_wheel','paddle_up','paddle_down','drs_button')
            obj.hide_render=view=='steering' and not wheel
        camera.location=loc;camera.data.lens=lens;aim(camera,target)
        path=args.output/(prefix+'-'+view+'.png');scene.render.filepath=str(path.resolve())
        bpy.ops.render.render(write_still=True)
        print('RENDER',path,flush=True)


if __name__=='__main__': main()
