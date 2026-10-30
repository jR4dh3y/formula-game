"""Run with python3 -I tools/car/test_livery.py."""
import importlib.util
from pathlib import Path
import unittest

import numpy as np

spec = importlib.util.spec_from_file_location('livery',Path(__file__).with_name('livery.py'))
livery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(livery)
COLORS = dict(primary='#FF0000',secondary='#0000FF',accent='#00FF00',carbon='#111111')


class LiveryTests(unittest.TestCase):
    def points(self, ys, zs):
        return np.array([[0,y,z] for y,z in zip(ys,zs)],dtype=float)

    def test_split_slopes_in_height_with_rearward_distance(self):
        p = self.points([.3,.3],[0,1])
        out = livery.evaluate_style(p,COLORS,dict(style='split',split_y=.2,split_slope=.2))
        np.testing.assert_array_equal(out,[[255,0,0],[0,0,255]])

    def test_fade_clamps_and_interpolates(self):
        out = livery.evaluate_style(self.points([0]*5,[0,.25,.5,.75,1]),COLORS,dict(style='fade',fade_z0=.25,fade_z1=.75))
        np.testing.assert_array_equal(out,[[255,0,0],[255,0,0],[127.5,0,127.5],[0,0,255],[0,0,255]])
        with self.assertRaises(ValueError):
            livery.evaluate_style(self.points([0],[0]),COLORS,dict(style='fade',fade_z0=.8,fade_z1=.2))

    def test_stripe_band_and_nose_precedence(self):
        p = self.points([.5,.5,.1],[.05,.5,.9])
        out = livery.evaluate_style(p,COLORS,dict(style='stripe',stripe_y=.5,stripe_w=.1,stripe_color='secondary',nose_tip='accent'))
        np.testing.assert_array_equal(out,[[0,255,0],[0,0,255],[255,0,0]])
        out = livery.evaluate_style(p,COLORS,dict(style='band',band_z=.5,band_w=.1,band_color='secondary'))
        np.testing.assert_array_equal(out,[[255,0,0],[0,0,255],[255,0,0]])

    def test_solid_and_unknown_style(self):
        p = self.points([0],[0])
        np.testing.assert_array_equal(livery.evaluate_style(p,COLORS,dict(style='solid')),[[255,0,0]])
        with self.assertRaises(ValueError): livery.evaluate_style(p,COLORS,dict(style='invalid'))

    def test_normalise_reverses_gltf_forward_for_nose_to_rear_styles(self):
        out = livery.normalise(np.array([[1,0,3],[-1,2,-3]]),np.array([[-1,0,-3],[1,2,3]]))
        np.testing.assert_array_equal(out,[[1,0,0],[-1,1,1]])

    def test_padding_preserves_islands_and_does_not_wrap_edges(self):
        rgb = np.zeros((5,5,3)); mask = np.zeros((5,5),bool)
        rgb[2,0]=[255,0,0]; mask[2,0]=True
        result = livery.dilate(rgb,mask,1)
        np.testing.assert_array_equal(result[2,1],[255,0,0])
        np.testing.assert_array_equal(result[2,4],[0,0,0])
        np.testing.assert_array_equal(result[2,0],rgb[2,0])

    def test_side_projection_reads_correctly_on_both_sides(self):
        # Equal screen coordinates traverse the complete word on opposite sides.
        u = np.linspace(.01,.99,101)
        p = np.zeros((2,len(u),3)); p[...,1]=.5
        p[0,:,0]=.5; p[1,:,0]=-.5
        p[0,:,2]=.5-u; p[1,:,2]=u-.5
        n = np.zeros_like(p); n[0,:,0]=1; n[1,:,0]=-1
        font = livery.ROOT/'game/fonts/TitilliumWeb-Bold.ttf'
        result = livery.project_text(np.zeros_like(p),p,n,'Red Bull',np.ones(3)*255,font,0,.5,1,.2,.3)
        np.testing.assert_array_equal(result[0],result[1])
        self.assertGreater(result.max(),0)
        self.assertFalse(np.allclose(result[0],result[0,::-1]))

    def test_side_projection_excludes_top_and_inward_faces(self):
        from unittest.mock import patch
        p = np.array([[.5,.5,0.],[-.5,.5,0.],[.5,.5,0.],[.5,.5,0.]])
        n = np.array([[1.,0.,0.],[-1.,0.,0.],[0.,1.,0.],[-1.,0.,0.]])
        with patch.object(livery, 'text_mask', return_value=np.ones((2,2))):
            result = livery.project_text(np.zeros_like(p),p,n,'X',np.ones(3),None,0,.5,1,.2,.3)
        np.testing.assert_array_equal(result, [[1,1,1],[1,1,1],[0,0,0],[0,0,0]])

    def test_every_roster_style_and_helmet_is_supported(self):
        import json
        data = json.loads((Path(__file__).with_name('teams.json')).read_text())
        points = np.array([[[.1,.7,.1],[.2,.8,.2]],[[.3,.9,.3],[.4,1.,.4]]])
        atlas = dict(position=points,mask=np.ones((2,2),bool))
        for team in data['teams']:
            with self.subTest(team=team['id']):
                self.assertEqual(livery.evaluate_style(points,team['colors'],team['livery']).shape,points.shape)
                for driver in team['drivers']:
                    output = livery.helmet(driver,atlas)
                    self.assertTrue(np.isfinite(output).all())
                    self.assertEqual(output.shape,points.shape)


if __name__ == '__main__': unittest.main()
