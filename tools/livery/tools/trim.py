import json, numpy as np
from PIL import Image
from scipy import ndimage
rep=json.loads(open('decal_report.txt').read()[7:])
for r in rep:
    if "error" in r: print("ERR", r); continue
    im=Image.open(r['path']); arr=np.array(im); a=arr[:,:,3]>8
    lab,n=ndimage.label(ndimage.binary_dilation(a,iterations=6))
    if n>1:
        sizes=ndimage.sum(a,lab,range(1,n+1)); big=sizes.max()
        mask=np.isin(lab,[i+1 for i,s in enumerate(sizes) if s>0.15*big]); arr[~mask]=0
        ys,xs=np.nonzero(arr[:,:,3]>8); x0,x1,y0,y1=xs.min(),xs.max()+1,ys.min(),ys.max()+1
        Image.fromarray(arr[y0:y1,x0:x1]).save(r['path'])
        r['x']+=int(x0); r['y']+=int(y0); r['w']=int(x1-x0); r['h']=int(y1-y0)
    print(r['name'], r['x'], r['y'], r['w'], r['h'])
json.dump([r for r in rep if "error" not in r],open('decal_report.json','w'))
