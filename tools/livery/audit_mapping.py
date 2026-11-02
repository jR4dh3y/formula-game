"""Measure coverage, orientation and distortion, without hiding collapsed/clamped cells.

python3 -I tools/livery/audit_mapping.py MAP BAKE --out REPORT.json
Area values use OBJ units squared. Unsupported parts have no geometry bake and
are reported as explicit fill-only texture overrides, not as covered geometry.
"""
import argparse
import json
from pathlib import Path
import numpy as np
from scipy import ndimage
from scipy.spatial import cKDTree


def audit(mapping, bake):
    result = {}
    for part in (k[:-3] for k in mapping if k.endswith('_xy')):
        xy = mapping[part+'_xy'].astype(float)
        fill, region = mapping[part+'_fill'], mapping[part+'_region']
        valid = np.isfinite(xy).all(-1)
        assert (np.isfinite(xy[...,0]) == np.isfinite(xy[...,1])).all(), part
        assert not (valid & (fill > 0)).any(), (part, 'mapping overlaps fill')
        assert not (valid & (region < 0)).any(), (part, 'missing region')
        names = list(mapping.get(part+'_names',mapping['names']))
        assert not (region >= len(names)).any(), (part, 'invalid region name')
        report = {'mapped_texels':int(valid.sum()), 'body_fill_texels':int((fill==1).sum()),
                  'carbon_fill_texels':int((fill==2).sum())}
        path = Path(bake)/(part+'.npz')
        if not path.exists():
            report['geometry'] = 'no bake; explicit neutral override'
            result[part] = report
            continue
        d = np.load(path);p=d['pos'].astype(float);mask=d['mask']
        report['occupied_texels'] = int(mask.sum())
        report['unmapped_texels'] = int((mask & ~valid).sum())
        report['unclassified_texels'] = int((mask & ~valid & (fill==0)).sum())
        du=p[:-1,1:]-p[:-1,:-1];dv=p[1:,:-1]-p[:-1,:-1]
        tu=xy[:-1,1:]-xy[:-1,:-1];tv=xy[1:,:-1]-xy[:-1,:-1]
        cross=np.cross(du,dv);area=np.linalg.norm(cross,axis=-1)
        r=region[:-1,:-1]
        cells=(r>=0)&(r==region[:-1,1:])&(r==region[1:,:-1])
        cells &= (np.linalg.norm(du,axis=-1)<.05)&(np.linalg.norm(dv,axis=-1)<.05)&(area>1e-12)
        cells &= np.isfinite(tu).all(-1)&np.isfinite(tv).all(-1)
        surface=np.einsum('ijk,ijk->ij',cross,d['nrm'][:-1,:-1])
        determinant=tu[...,0]*tv[...,1]-tu[...,1]*tv[...,0]
        for i in np.unique(r[cells]):
            c=cells&(r==i);weights=area[c]
            S=np.stack([du[c],dv[c]],-1);T=np.stack([tu[c],tv[c]],-1)
            _,R=np.linalg.qr(S);J=T@np.linalg.inv(R);sv=np.linalg.svd(J,compute_uv=False)
            aniso=sv[:,0]/np.maximum(sv[:,1],1e-12)
            folded=surface[c]*determinant[c]>1e-12
            collapsed=sv[:,1]<1e-3
            report.setdefault('regions',{})[str(names[i])] = {
                'cells':int(c.sum()), 'sampled_area':float(weights.sum()),
                'folded_area_pct':float(100*weights[folded].sum()/weights.sum()),
                'collapsed_area_pct':float(100*weights[collapsed].sum()/weights.sum()),
                'anisotropy_p50':float(np.median(aniso)),
                'anisotropy_p90':float(np.percentile(aniso,90))}
        labels, _ = ndimage.label(mask)
        border = mask & ~ndimage.binary_erosion(mask)
        rr,cc = np.nonzero(border & valid)
        pairs = cKDTree(p[rr,cc]).query_pairs(.015, output_type='ndarray')
        if len(pairs):
            aa,bb = pairs.T
            different = labels[rr[aa],cc[aa]] != labels[rr[bb],cc[bb]]
            same_patch = region[rr[aa],cc[aa]] == region[rr[bb],cc[bb]]
            aa,bb = aa[different & same_patch], bb[different & same_patch]
            if len(aa):
                jump = np.linalg.norm(xy[rr[aa],cc[aa]]-xy[rr[bb],cc[bb]],axis=1)
                report['same_patch_uv_seams'] = {'pairs':len(aa), 'template_jump_p50_px':float(np.median(jump)),
                    'template_jump_p90_px':float(np.percentile(jump,90)), 'template_jump_max_px':float(jump.max())}
        result[part] = report
    return result


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('map',type=Path);ap.add_argument('bake',type=Path)
    ap.add_argument('--out',type=Path,required=True)
    a=ap.parse_args();result=audit(dict(np.load(a.map)),a.bake)
    a.out.write_text(json.dumps(result,indent=2)+'\n')
    print(a.out)
    for part,r in result.items():
        print(part,'mapped',r['mapped_texels'],'unmapped',r.get('unmapped_texels','no bake'))
        for name,st in r.get('regions',{}).items():
            print(f"  {name}: folds {st['folded_area_pct']:.3f}% collapsed {st['collapsed_area_pct']:.3f}% aniso {st['anisotropy_p50']:.2f}/{st['anisotropy_p90']:.2f}")

if __name__=='__main__':
    main()
