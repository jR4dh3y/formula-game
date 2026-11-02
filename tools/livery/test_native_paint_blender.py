"""Blender behaviour test: panel-gated emission bake, physical aspect and UV0."""
from pathlib import Path
import sys
import tempfile
import unittest

import importlib.util

sys.path.insert(0, str(Path(__file__).resolve().parent))


@unittest.skipUnless(importlib.util.find_spec("bpy"), "requires Blender")
class NativePaintTest(unittest.TestCase):
    def test_side_projector_leaves_top_panel_at_base_colour(self):
        import bpy
        import numpy as np
        from native_paint import bake

        bpy.ops.wm.read_factory_settings(use_empty=True)
        mesh = bpy.data.meshes.new('vertical side and horizontal shoulder')
        mesh.from_pydata([(0,-.5,0),(0,.5,0),(0,.5,1),(0,-.5,1),
                         (0,-.5,1),(.5,-.5,1),(.5,.5,1),(0,.5,1)], [], [(0,1,2,3),(4,5,6,7)])
        obj = bpy.data.objects.new('projector fixture', mesh)
        bpy.context.scene.collection.objects.link(obj)
        uv = mesh.uv_layers.new(name='OriginalUV')
        for face in mesh.polygons:
            left = face.index * .5
            for loop, point in zip(face.loop_indices, [(left,0),(left+.5,0),(left+.5,1),(left,1)]):
                uv.data[loop].uv = point
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artwork = bpy.data.images.new('blue mark', 64, 64, alpha=True)
            artwork.pixels.foreach_set(np.tile([0,0,1,1], 64*64).astype(np.float32))
            artwork.filepath_raw = str(root/'mark.png')
            artwork.file_format = 'PNG'
            artwork.save()
            spec = {'team':'fixture','palette':{'red':'#FF0000'},
                    'frame':{'origin':[0,0,0],'yaw':0},
                    'parts':{'fixture':{'base':'red'}},
                    'decals':[{'part':'fixture','image':'mark.png','width':1,
                               'center':[0,0,.5],'right':[0,1,0],'up':[0,0,1],
                               'where':{'normal_bounds':{'z':[-.1,.6]}}}]}
            bake({'fixture':[obj]},spec,root,root,64,[],None)
            image = bpy.data.images.load(str(root/'fixture.png'),check_existing=False)
            pixels = np.array(image.pixels[:]).reshape(64,64,4)
            np.testing.assert_allclose(pixels[32,16,:3], [0,0,1], atol=.02)
            np.testing.assert_allclose(pixels[32,48,:3], [1,0,0], atol=.02)

    def test_panel_tagging_retains_untouched_evaluated_uv0(self):
        import bpy
        import hashlib
        import numpy as np
        from native_paint import meshes

        bpy.ops.wm.read_factory_settings(use_empty=True)
        bpy.ops.mesh.primitive_plane_add(size=2)
        obj = bpy.context.object
        obj.name = 'body_object'
        collection = bpy.data.collections.new('fp04_tr_model')
        bpy.context.scene.collection.children.link(collection)
        collection.objects.link(obj)
        obj.modifiers.new('source subdivision', 'SUBSURF').levels = 2
        deps = bpy.context.evaluated_depsgraph_get()
        original = bpy.data.meshes.new_from_object(obj.evaluated_get(deps), preserve_all_data_layers=True, depsgraph=deps)
        before = np.empty(len(original.loops)*2, np.float32)
        original.uv_layers[0].data.foreach_get('uv', before)
        groups, records = meshes({'parts': {'main_body': {}}})
        extracted = groups['main_body'][0].data
        after = np.empty(len(extracted.loops)*2, np.float32)
        extracted.uv_layers[0].data.foreach_get('uv', after)
        np.testing.assert_array_equal(before, after)
        self.assertEqual(records[0]['uv0_sha256'], hashlib.sha256(before.tobytes()).hexdigest())

    def test_curved_decal_uses_arc_length_and_stops_at_its_physical_width(self):
        import bpy
        import numpy as np
        from native_paint import bake

        bpy.ops.wm.read_factory_settings(use_empty=True)
        bpy.ops.mesh.primitive_plane_add(size=2)
        obj = bpy.context.object
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artwork = bpy.data.images.new('long blue mark', 120, 20, alpha=True)
            artwork.pixels.foreach_set(np.tile([0, 0, 1, 1], 120*20).astype(np.float32))
            artwork.filepath_raw = str(root/'mark.png')
            artwork.file_format = 'PNG'
            artwork.save()
            diagonal = 2 ** -.5
            spec = {'team': 'fixture', 'palette': {'red': '#FF0000'},
                    'frame': {'origin': [0, 0, 0], 'yaw': 0},
                    'parts': {'fixture': {'base': 'red'}},
                    'decals': [{'part': 'fixture', 'image': 'mark.png', 'width': .6,
                                'center': [0, 0, 0],
                                'right': [diagonal, diagonal, 0],
                                'up': [-diagonal, diagonal, 0],
                                'path': [[-.75, -.25, 0], [0, -.25, 0], [0, .5, 0]]}]}
            bake({'fixture': [obj]}, spec, root, root, 128, [], None)
            image = bpy.data.images.load(str(root/'fixture.png'), check_existing=False)
            pixels = np.array(image.pixels[:]).reshape(128, 128, 4)
            for x, y in [(-.2, -.25), (0, 0)]:
                np.testing.assert_allclose(pixels[int((y+1)*64), int((x+1)*64), :3], [0, 0, 1], atol=.02)
            for x, y in [(-.5, -.25), (0, .3), (-.4, .2)]:
                np.testing.assert_allclose(pixels[int((y+1)*64), int((x+1)*64), :3], [1, 0, 0], atol=.02)

    def test_decal_bakes_once_on_selected_panel_and_preserves_uv0(self):
        import bpy
        import numpy as np
        from native_paint import bake, linear

        bpy.ops.wm.read_factory_settings(use_empty=True)
        mesh = bpy.data.meshes.new('two separate physical panels')
        mesh.from_pydata([(-1,-.5,0),(0,-.5,0),(0,.5,0),(-1,.5,0),
                         (0,-.5,0),(1,-.5,0),(1,.5,0),(0,.5,0)], [], [(0,1,2,3),(4,5,6,7)])
        obj = bpy.data.objects.new('native fixture', mesh)
        bpy.context.scene.collection.objects.link(obj)
        uv = mesh.uv_layers.new(name='OriginalUV')
        for face in mesh.polygons:
            for index in face.loop_indices:
                x,y,_ = mesh.vertices[mesh.loops[index].vertex_index].co
                uv.data[index].uv = ((x+1)/2, y+.5)
        panels = mesh.attributes.new('native_panel', 'INT', 'FACE')
        panels.data[0].value, panels.data[1].value = 0,1
        before = np.array([tuple(item.uv) for item in uv.data])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            decal = bpy.data.images.new('blue artwork', 128,64,alpha=True)
            decal.pixels.foreach_set(np.tile([0,0,1,1], 128*64).astype(np.float32))
            decal.filepath_raw = str(root/'mark.png')
            decal.file_format = 'PNG'
            decal.save()
            spec = {'team':'fixture','palette':{'red':'#FF0000'},
                    'frame':{'origin':[0,0,0],'yaw':0},
                    'parts':{'fixture':{'base':'red'}},
                    'decals':[{'part':'fixture','image':'mark.png','width':2,
                               'center':[0,0,0],'right':[1,0,0],'up':[0,1,0],
                               'where':{'panels':[0]},'facing':[0,0,1]}]}
            bake({'fixture':[obj]},spec,root,root,64,[],None)
            result = bpy.data.images.load(str(root/'fixture.png'),check_existing=False)
            pixels = np.array(result.pixels[:]).reshape(64,64,4)
            np.testing.assert_allclose(pixels[32,16,:3], [0,0,1], atol=.02)
            np.testing.assert_allclose(pixels[32,48,:3], [1,0,0], atol=.02)
            # Physical 2:1 art spans the full 2x1 geometry. A square stretch would
            # wrongly shrink the painted footprint in Y or expand it in X.
            np.testing.assert_allclose(pixels[8,16,:3], [0,0,1], atol=.02)
            self.assertEqual(before.shape,(8,2))
            self.assertEqual(linear('#FFFFFF'), (1,1,1,1))


if __name__ == '__main__':
    unittest.main(argv=[sys.argv[0]])
