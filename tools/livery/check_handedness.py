"""Check that each region's paint texel frame winds the same way as the template.

Usage: python3 -I tools/livery/check_handedness.py MAP.npz BAKE_DIR
"""
import sys, numpy as np
M=np.load(sys.argv[1]); bake=sys.argv[2]; names=list(M['names'])
for part in ['main_body','front_wing','rear_wing']:
    names = list(M[f"{part}_names"]) if f"{part}_names" in M else list(M["names"])
    xy=M[f'{part}_xy'].astype(float); reg=M[f'{part}_region']; sn=M[f'{part}_snap'].astype(float)
    d=np.load(f'{bake}/{part}.npz'); pos=d['pos'].astype(float); nrm=d['nrm'].astype(float)
    A=slice(0,-1)
    du=pos[A,1:]-pos[A,:-1]; dv=pos[1:,A]-pos[:-1,A]
    tu=xy[A,1:]-xy[A,:-1]; tv=xy[1:,A]-xy[:-1,A]
    r=reg[A,A]; n=nrm[A,A]
    ok=(r>=0)&(reg[A,1:]==r)&(reg[1:,A]==r)&(sn[A,A]<1)&(np.linalg.norm(du,axis=-1)<.05)&(np.linalg.norm(dv,axis=-1)<.05)
    # orientation of texel frame on surface relative to the normal
    surf=np.sign(np.einsum('...k,...k',np.cross(du,dv),n))
    # template: image x right, y down; viewed "from outside" => use (x, -y)
    tmpl=np.sign(tu[...,0]*(-tv[...,1]) - (-tu[...,1])*tv[...,0])
    h=surf*tmpl
    print('==',part)
    for k in np.unique(r[ok]):
        m=ok&(r==k)
        if m.sum()<300: continue
        print(f'  {names[k]:13s} correct={np.mean(h[m]>0):.2f} n={m.sum()}')
