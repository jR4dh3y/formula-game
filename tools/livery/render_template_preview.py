"""Render the authored UV1 on source geometry, with embedded scripts disabled."""
import argparse
import math
from pathlib import Path
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
import author_template_uv


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('sheet', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--front-only', action='store_true')
    parser.add_argument('--template-year', choices=['2025', '2026'], default='2025')
    parser.add_argument('--views', nargs='+', help='render only these named review views')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    args.output.mkdir(parents=True, exist_ok=True)
    author_template_uv.apply(args.template_year)
    if args.template_year == '2025':
        import install_liveries
        import numpy as np
        labels = np.load(author_template_uv.REPO / 'raw/gen22/livery-mapping/2025/islands.npy')
        install_liveries.padded_sheet(args.sheet, labels).save(args.output/'sheet-padded.png')
        args.sheet = args.output/'sheet-padded.png'
    sheet = bpy.data.images.load(str(args.sheet.resolve()))
    sheet.colorspace_settings.name = 'sRGB'
    sheet.alpha_mode = 'NONE'
    paint = bpy.data.materials.new('Direct template sheet')
    paint.use_nodes = True
    shader = paint.node_tree.nodes.get('Principled BSDF')
    shader.inputs['Roughness'].default_value = .38
    tex = paint.node_tree.nodes.new('ShaderNodeTexImage')
    tex.image = sheet
    coords = paint.node_tree.nodes.new('ShaderNodeUVMap')
    coords.uv_map = 'livery'
    paint.node_tree.links.new(coords.outputs['UV'], tex.inputs['Vector'])
    paint.node_tree.links.new(tex.outputs['Color'], shader.inputs['Base Color'])
    carbon = bpy.data.materials.new('Preview composite')
    carbon.diffuse_color = (.012, .014, .017, 1)
    carbon.use_nodes = True
    carbon.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value = carbon.diffuse_color
    deps = bpy.context.evaluated_depsgraph_get()
    objects = []
    for source in list(bpy.data.collections['fp04_tr_model'].all_objects):
        if source.type != 'MESH' or source.hide_render:
            continue
        if args.front_only and source.name != 'front_wing_mixer':
            continue
        mesh = bpy.data.meshes.new_from_object(source.evaluated_get(deps), preserve_all_data_layers=True, depsgraph=deps)
        mesh.transform(source.matrix_world)
        obj = bpy.data.objects.new('preview_'+source.name, mesh)
        bpy.context.scene.collection.objects.link(obj)
        mesh.materials.clear()
        mesh.materials.append(paint if mesh.uv_layers.get('livery') else carbon)
        for face in mesh.polygons:
            face.material_index = 0
        if args.template_year == '2025':
            sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'car'))
            import livery_seams
            livery_seams.constrain(obj)
        objects.append(obj)
    for obj in list(bpy.data.objects):
        if obj not in objects:
            bpy.data.objects.remove(obj, do_unlink=True)
    for text in list(bpy.data.texts):
        bpy.data.texts.remove(text)
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.cycles.samples = 8
    scene.render.resolution_x, scene.render.resolution_y = 1280, 720
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = 'PNG'
    scene.render.image_settings.color_mode = 'RGB'
    scene.render.image_settings.color_depth = '8'
    scene.render.image_settings.compression = 30
    # Standard preserves the sheet's display-referred sponsor colours. Keep
    # light levels below clipping rather than desaturating them with AgX.
    scene.view_settings.view_transform = 'Standard'
    scene.view_settings.look = 'None'
    scene.view_settings.exposure = 0
    scene.view_settings.gamma = 1
    world = bpy.data.worlds.new('Studio')
    world.use_nodes = True
    world.node_tree.nodes['Background'].inputs[0].default_value = (.3, .3, .3, 1)
    world.node_tree.nodes['Background'].inputs[1].default_value = .35
    scene.world = world
    light = bpy.data.lights.new('Softbox', 'AREA')
    light.energy, light.shape, light.size = 600, 'DISK', 5
    obj = bpy.data.objects.new('Softbox', light)
    scene.collection.objects.link(obj)
    obj.location = (1, 1, 6)
    light = bpy.data.lights.new('Sun', 'SUN')
    light.energy = 1.0
    obj = bpy.data.objects.new('Sun', light)
    scene.collection.objects.link(obj)
    obj.rotation_euler = (.4, -.3, -.5)
    bpy.ops.mesh.primitive_plane_add(size=200, location=(0, 0, .02))
    ground = bpy.context.object
    ground.data.materials.append(carbon)
    camera = bpy.data.objects.new('Review camera', bpy.data.cameras.new('Review camera'))
    scene.collection.objects.link(camera)
    scene.camera = camera
    camera.data.type = 'ORTHO'
    views = {
        'front-wing': ((.045, -3.7, 2.7), (.045, -.3, .2), 2.25),
        'front-wing-top': ((.045, -.3, 5), (.045, -.3, .2), 2.25),
        'front-endplate': ((-3, -.65, .6), (-.9, -.1, .28), .85),
    }
    if not args.front_only:
        views.update({
            'side-left': ((8, 1.8, .85), (.045, 1.8, .6), 6.1),
            'side-right': ((-8, 1.8, .85), (.045, 1.8, .6), 6.1),
            'top': ((.045, 1.8, 9), (.045, 1.8, .6), 6.1),
            'three-quarter': ((7, -7, 4.4), (.045, 1.8, .55), 6.7),
            'three-quarter-right': ((-7, -7, 4.4), (.045, 1.8, .55), 6.7),
            'fiorano-quarter': ((-7, -7, 1.8), (.045, 1.8, .55), 6.7),
            'front': ((.045, -8, 1.6), (.045, 1.8, .55), 3.5),
            'rear-wing': ((-2.6, 6, 2), (.075, 4.5, .8), 1.9),
            'rear-wing-back': ((.075, 7, 1.15), (.075, 4.5, .85), 1.8),
        })
    for name, (location, target, scale) in views.items():
        if args.views and name not in args.views:
            continue
        camera.location = location
        camera.rotation_euler = (Vector(target)-camera.location).to_track_quat('-Z', 'Y').to_euler()
        if name == 'top':
            camera.rotation_euler.rotate_axis('Z', -math.pi/2)
        camera.data.ortho_scale = scale
        scene.render.filepath = str((args.output/(name+'.png')).resolve())
        bpy.ops.render.render(write_still=True)
    bpy.ops.wm.save_as_mainfile(filepath=str((args.output/'review.blend').resolve()))


if __name__ == '__main__':
    main()
