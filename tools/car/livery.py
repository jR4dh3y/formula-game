"""Generate Gen22 team textures from rasterised surface positions, without Blender.

Run: python3 -I tools/car/livery.py [--bake raw/gen22/bake]
Coordinates supplied to evaluate_style follow teams.json's livery_styles contract.
"""
import argparse
import json
from pathlib import Path
import shutil
import time

import numpy as np
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]


def color(value):
    if len(value) != 7 or value[0] != '#':
        raise ValueError('Expected #RRGGBB colour: ' + value)
    return np.array([int(value[i:i+2],16) for i in (1,3,5)], dtype=np.float32)


def evaluate_style(position, colors, style):
    """Evaluate documented team shapes at arbitrary normalised surface points."""
    palette = {key:color(value) for key,value in colors.items()}
    y,z = position[...,1], position[...,2]
    output = np.broadcast_to(palette['primary'], position.shape).copy()
    name = style['style']
    if name == 'split':
        output[y < style['split_y'] + style['split_slope']*z] = palette['secondary']
    elif name == 'fade':
        start,end = style['fade_z0'],style['fade_z1']
        if end <= start: raise ValueError('fade_z1 must be greater than fade_z0')
        t = np.clip((z-start)/(end-start),0,1)[...,None]
        output = palette['primary']*(1-t)+palette['secondary']*t
    elif name == 'stripe':
        output[np.abs(y-style['stripe_y']) <= style['stripe_w']/2] = palette[style['stripe_color']]
    elif name == 'band':
        output[np.abs(z-style['band_z']) <= style['band_w']/2] = palette[style['band_color']]
    elif name != 'solid':
        raise ValueError('Unknown livery style: ' + name)
    if style.get('nose_tip'):
        output[z <= .12] = palette[style['nose_tip']]
    return output


def normalise(position, extents):
    low,high = extents
    result = (position-low)/np.maximum(high-low,1e-6)
    result[...,0] = position[...,0]/max(abs(low[0]),abs(high[0]))
    result[...,2] = 1-result[...,2]
    return result


def dilate(rgb, mask, iterations=12):
    """Extend nearest edge colours into UV gutters without blending across seams."""
    result = rgb.copy()
    filled = mask.astype(bool).copy()
    for _ in range(iterations):
        previous = filled.copy()
        for dy,dx in ((0,1),(0,-1),(1,0),(-1,0)):
            src_y = slice(max(0,-dy),min(mask.shape[0],mask.shape[0]-dy))
            src_x = slice(max(0,-dx),min(mask.shape[1],mask.shape[1]-dx))
            dst_y = slice(max(0,dy),min(mask.shape[0],mask.shape[0]+dy))
            dst_x = slice(max(0,dx),min(mask.shape[1],mask.shape[1]+dx))
            use = ~filled[dst_y,dst_x] & previous[src_y,src_x]
            result[dst_y,dst_x][use] = result[src_y,src_x][use]
            filled[dst_y,dst_x][use] = True
    return result


def text_mask(text, font):
    face = ImageFont.truetype(str(font), 150)
    bounds = face.getbbox(text)
    image = Image.new('L',(bounds[2]-bounds[0]+12,bounds[3]-bounds[1]+12))
    ImageDraw.Draw(image).text((6-bounds[0],6-bounds[1]),text,font=face,fill=255)
    return np.asarray(image,dtype=np.float32)/255


def project_text(rgb, position, normal, text, ink, font, centre_z, centre_y, width, height, min_x):
    """Side projection reads left-to-right from outside either side of the car."""
    bitmap = text_mask(text,font)
    x,y,z = position[...,0],position[...,1],position[...,2]
    # +X viewer sees rearward (negative glTF Z) as screen-right; -X is opposite.
    u = .5-np.sign(x)*(z-centre_z)/width
    v = .5-(y-centre_y)/height
    visible = (np.abs(x)>min_x) & (np.sign(x)*normal[...,0]>-.05) & (u>=0)&(u<1)&(v>=0)&(v<1)
    ix = np.clip((u*bitmap.shape[1]).astype(int),0,bitmap.shape[1]-1)
    iy = np.clip((v*bitmap.shape[0]).astype(int),0,bitmap.shape[0]-1)
    alpha = (bitmap[iy,ix]*visible)[...,None]
    return rgb*(1-alpha)+ink*alpha


def paint(team, atlas, font):
    p,n = atlas['position'],atlas['normal']
    q = normalise(p,atlas['extents'])
    colors,style = team['colors'],team['livery']
    output = evaluate_style(q,colors,style)
    # Physical zones are deliberately independent of the packed UV islands.
    endplates = ((q[...,2]<.16)&(np.abs(p[...,0])>.81)) | ((q[...,2]>.90)&(np.abs(p[...,0])>.50))
    output[endplates] = color(colors[style['wing_endplates']])
    halo = (p[...,2]>.12)&(p[...,2]<1.3)&(p[...,1]>.74)&(np.abs(p[...,0])<.30)
    output[halo] = color(colors[style['halo']])
    # The monocoque's inner walls stay dark even when its exterior is painted.
    interior=(np.abs(p[...,0])<.24)&(p[...,1]<.78)&(p[...,2]>.05)&(p[...,2]<1.1)
    inward=(p[...,0]*n[...,0]<-.01)|(n[...,2]<-.6)
    output[interior&inward]=color(colors['carbon'])
    # Fine accent keyline accompanies the main sidepod sweep, using the same car-space seam.
    line = .285 + .16*q[...,2]
    output[(np.abs(q[...,1]-line)<.007)&(q[...,2]>.42)&(q[...,2]<.85)] = color(colors['accent'])
    ink = color(colors[style['text_color']])
    output = project_text(output,p,n,team['short'],ink,font,-.35,.52,1.0,.13,.20)
    output = project_text(output,p,n,team['short'],ink,font,-2.10,.72,.34,.09,.49)
    return output


def suit(team, atlas, font):
    p = atlas['position']
    colors = team['suit']
    output = np.broadcast_to(color(colors['primary']),p.shape).copy()
    # Contrast shoulder caps, arm stripes and a central zip. All follow posed geometry.
    output[(np.abs(p[...,0])>.11)&(p[...,1]>.48)] = color(colors['secondary'])
    output[(np.abs(p[...,0])>.16)&(np.abs(p[...,1]-.55)<.018)] = color(colors['accent'])
    output[np.abs(p[...,0])<.006] = color(colors['secondary'])
    return project_text(output,p,atlas['normal'],team['short'],color(colors['accent']),font,.62,.54,.25,.065,.02)


def helmet(driver, atlas):
    colors = driver['helmet']
    p = atlas['position']
    mask = atlas['mask'].astype(bool)
    lo,hi = p[mask].min(axis=0),p[mask].max(axis=0)
    q = (p-lo)/np.maximum(hi-lo,1e-5)
    x,y,z = q[...,0],q[...,1],q[...,2]
    out = np.broadcast_to(color(colors['primary']),p.shape).copy()
    style = colors['style']
    if style == 'stripe': shape = np.abs(x-.5)<.22
    elif style == 'chevron': shape = np.abs(y-(.45+.3*np.abs(x-.5)))<.13
    elif style == 'visor_band': shape = np.abs(y-.48)<.13
    elif style == 'halves': shape = x>.5
    else: raise ValueError('Unknown helmet style: '+style)
    out[shape] = color(colors['secondary'])
    out[(np.abs(y-(.65+.10*np.cos(z*6.28)))<.035)|(np.abs(x-.5)<.035)] = color(colors['accent'])
    return out


def save(path, rgb, mask):
    path.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(np.clip(dilate(rgb,mask),0,255).astype(np.uint8)).save(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--teams',type=Path,default=ROOT/'tools/car/teams.json')
    parser.add_argument('--bake',type=Path,default=ROOT/'raw/gen22/bake')
    parser.add_argument('--output',type=Path,default=ROOT/'assets/cars/gen22')
    parser.add_argument('--wait-seconds',type=float,default=120)
    args = parser.parse_args()
    deadline = time.monotonic()+args.wait_seconds
    while not args.teams.exists() and time.monotonic()<deadline:
        time.sleep(1)
    data = json.loads(args.teams.read_text())
    atlases = {name:dict(np.load(args.bake/(name+'.npz'))) for name in ('paint','suit','helmet_paint')}
    font = ROOT/'game/fonts/TitilliumWeb-Bold.ttf'
    for team in data['teams']:
        directory = args.output/'liveries'/team['id']
        save(directory/'paint.png',paint(team,atlases['paint'],font),atlases['paint']['mask'])
        save(directory/'suit.png',suit(team,atlases['suit'],font),atlases['suit']['mask'])
        for driver in team['drivers']:
            save(directory/driver['id']/'helmet.png',helmet(driver,atlases['helmet_paint']),atlases['helmet_paint']['mask'])
        print('LIVERY',team['short'],flush=True)
    args.output.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(args.teams,args.output/'teams.json')


if __name__ == '__main__': main()
