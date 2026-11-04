"""CPU-only diagnostics for the immutable full-detail quarry captures.

Writes private numerical evidence and image derivatives; never imports Unreal or
wgpu. Diagnostic regressions/transforms are hypothesis tests, not corrected scores.
Run with numerical-library threads limited to two on the shared machine.
"""
import argparse
import hashlib
import json
from pathlib import Path
import time

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage

ROOT = Path("D:/Coding/renderlab-work/ue-m5/Quarry")
REFERENCE = Path("D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry")


def stats(x):
    x = np.asarray(x)
    if not x.size:
        return dict(n=0)
    return dict(n=int(x.size), rms=float(np.sqrt(np.mean(x*x))), mean=float(x.mean()),
                quantiles=np.quantile(x, [0, .01, .1, .5, .9, .99, 1]).tolist())


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def gray(v, cover, valid):
    rgb = np.repeat((255*np.clip(np.nan_to_num(v), 0, 1)).astype('u1')[..., None], 3, -1)
    rgb[~cover] = 0
    rgb[cover & ~valid] = [0, 255, 0]
    return rgb


def error_rgb(e, cover, valid):
    # Red = positive, blue = negative, green = undefined. Fixed +/-1 scale.
    t = np.clip(np.nan_to_num(e), -1, 1)[..., None]
    rgb = np.where(t >= 0, 220 + t*(np.array([255, 0, 0])-220),
                   220 - t*(np.array([0, 0, 255])-220)).astype('u1')
    rgb[~cover] = 0
    rgb[cover & ~valid] = [0, 255, 0]
    return rgb


def sheet(panels, names, output, crop=None, zoom=1):
    images = [Image.fromarray(a) for a in panels]
    if crop:
        images = [im.crop(crop) for im in images]
    w, h = images[0].size
    canvas = Image.new('RGB', (w*len(images)*zoom, h*zoom+26), '#202020')
    draw = ImageDraw.Draw(canvas)
    for i, (im, name) in enumerate(zip(images, names)):
        canvas.paste(im.resize((w*zoom, h*zoom), Image.Resampling.NEAREST), (i*w*zoom, 26))
        draw.text((i*w*zoom+4, 5), name, fill='white')
    canvas.save(output)


def analyze(args):
    args.output.mkdir(parents=True, exist_ok=False)
    start = time.monotonic()
    light = np.array([.4, .8, .3]); light /= np.linalg.norm(light)
    results = dict(reference=str(args.reference), root=str(args.root), cases=[])
    for pose in range(3):
        rp = args.reference / f'pose-{pose}-reference.npz'
        with np.load(rp) as a:
            ref, cover = a['ref'].copy(), a['cover'].copy()
        normal = np.fromfile(args.reference / f'pose-{pose}-normal.f32', '<f4').reshape(*ref.shape, 4)[..., :3]
        world = np.fromfile(args.reference / f'pose-{pose}-world.f32', '<f4').reshape(*ref.shape, 4)[..., :3]
        # Receiver shader cross(dpdx,dpdy) points away from the camera here.
        # Orient to the view before describing a geometric light terminator.
        eye=np.array([5.2+(.01 if pose==1 else 0),3.4,6.2])
        toward_eye=np.sum(normal*(eye-world),axis=-1)
        normal=np.where((toward_eye<0)[...,None],-normal,normal)
        nl = normal @ light
        prod = np.fromfile(args.reference / f'pose-{pose}-seed-0-production.u32', '<u4').reshape(ref.shape)/256
        partial = cover & (ref > 0) & (ref < 1)
        dist = ndimage.distance_transform_edt(~partial)
        for mode in ('vsm', 'hrt'):
            run = f'full-{mode}-capture-01'
            path = args.root/'extracted'/run/f'pose-{pose}-steady-direct.npz'
            with np.load(path) as a:
                v, valid, shadowed, unshadowed = [a[k].copy() for k in ('visibility', 'valid', 'shadowed', 'unshadowed')]
            e = v-ref
            p = partial & valid
            lum = unshadowed @ [.2126, .7152, .0722]
            row = dict(mode=mode, pose=pose, input_sha256=digest(path), reference_sha256=digest(rp),
                       partial=stats(e[p]), production_same=stats((prod-ref)[p]),
                       covered=stats(e[cover & valid]), missing_partial=stats(nl[partial & ~valid]),
                       missing_all=stats(nl[cover & ~valid]), missing_n=stats(lum[cover & ~valid]),
                       denominator=stats(lum[cover & valid]), normals_length=stats(np.linalg.norm(normal[cover], axis=-1)),
                       unshadowed_rgb=stats(unshadowed[cover].ravel()), shadowed_rgb=stats(shadowed[cover].ravel()))
            row['by_nl'] = []
            for lo, hi in zip([-1,-.2,-.04,0,.04,.1,.2,.4,.6,.8],[-.2,-.04,0,.04,.1,.2,.4,.6,.8,1.001]):
                s = (nl>=lo) & (nl<hi)
                row['by_nl'].append(dict(interval=[lo,hi], partial=stats(e[p&s]),
                    all=stats(e[cover&valid&s]), partial_missing=int((partial&~valid&s).sum())))
            row['populations'] = {}
            for name,s in [('lit',ref==1),('umbra',ref==0),('partial',partial)]:
                sel=cover&valid&s
                row['populations'][name]=dict(error=stats(e[sel]), visibility=stats(v[sel]),
                    geometric_nl=stats(nl[sel]), large_error_nl=stats(nl[sel&(abs(e)>.5)]),
                    over_002=int((sel&(abs(e)>.002)).sum()),over_01=int((sel&(abs(e)>.01)).sum()),
                    over_half=int((sel&(abs(e)>.5)).sum()),
                    extreme_flips=int((sel&(abs(e)>.95)).sum()))
            row['by_reference'] = []
            for lo,hi in zip([0,.1,.25,.5,.75,.9],[.1,.25,.5,.75,.9,1.001]):
                s=p&(ref>=lo)&(ref<hi)
                row['by_reference'].append(dict(interval=[lo,hi],error=stats(e[s]),visibility=stats(v[s])))
            row['by_distance_to_partial'] = [dict(interval=[lo,hi],error=stats(e[cover&valid&(dist>=lo)&(dist<hi)]))
                for lo,hi in [(0,1),(1,2),(2,4),(4,8),(8,16),(16,1000)]]
            # A global ratio/offset is falsified by structure left after this fit.
            design=np.column_stack([ref[p],np.ones(p.sum())])
            coef=np.linalg.lstsq(design,v[p],rcond=None)[0]
            row['affine_diagnostic']=dict(coefficients=coef.tolist(), residual=stats(v[p]-design@coef),
                centered_error=stats(e[p]-e[p].mean()), ref_candidate_correlation=float(np.corrcoef(ref[p],v[p])[0,1]))
            chans=np.divide(shadowed,unshadowed,out=np.full_like(shadowed,np.nan),where=unshadowed>1e-6)
            spread=np.max(np.where(np.isfinite(chans),abs(chans-v[...,None]),0),axis=-1)
            row['channel_spread']=stats(spread[valid])
            row['ratio_hypotheses']={name:stats((candidate-ref)[p]) for name,candidate in
                [('identity',v),('sqrt',np.sqrt(np.maximum(v,0))),('square',v*v)]}
            row['missing_breakdown']=dict(source_only=int((cover&~np.load(path)['ue_cover']).sum()),
                exact_zero_denominator=int((cover&(lum==0)).sum()),
                positive_subthreshold_denominator=int((cover&(lum>0)&(lum<=1e-6)).sum()))
            row['partial_binary']=dict(zero=int((p&(v==0)).sum()),one=int((p&(v==1)).sum()),
                near_endpoints=int((p&((v<.01)|(v>.99))).sum()),
                ideal_single_bernoulli_rms=float(np.sqrt(np.mean(ref[p]*(1-ref[p])))))
            # Single straight-edge disk test: changing width alone is symmetric
            # about V=.5. Complex quarry occluders do not satisfy this model.
            z=np.linspace(-1,1,10001)
            cdf=lambda z:.5+(np.arcsin(z)+z*np.sqrt(np.maximum(0,1-z*z)))/np.pi
            signed_distance=np.interp(ref[p],cdf(z),z)
            row['edge_width_diagnostic']=[dict(radius_scale=float(scale),
                residual=stats(v[p]-cdf(np.clip(signed_distance/scale,-1,1))))
                for scale in [.5,1,2]]
            states=['cut' if pose==0 else 'motion-first','settle-1','settle-7','settle-31','steady']
            vs=[];state_results=[]
            for state in states:
                with np.load(path.with_name(f'pose-{pose}-{state}-direct.npz')) as data:
                    sv=data['visibility'];vs.append(sv[p])
                    state_results.append(dict(state=state,error=stats((sv-ref)[p]),
                        near_endpoints=int((p&((sv<.01)|(sv>.99))).sum())))
            vs=np.stack(vs)
            row['temporal_diagnostic']=dict(states=state_results,
                sparse_five_frame_mean_error=stats(vs.mean(0)-ref[p]),
                changed_classification=int(np.any((vs>.5)!=(vs[0]>.5),axis=0).sum()),
                mean_per_pixel_variance=float(vs.var(0,ddof=1).mean()))
            # Shift tests require valid neighbours and the SAME starting domain.
            shift_domain=p.copy()
            for dy in range(-3,4):
                for dx in range(-3,4):
                    shift_domain &= np.roll(valid,(dy,dx),(0,1))
            shifts=[]
            for dy in range(-3,4):
                for dx in range(-3,4):
                    shifts.append(dict(dx=dx,dy=dy,error=stats((np.roll(v,(dy,dx),(0,1))-ref)[shift_domain])))
            row['image_shift_diagnostic']=sorted(shifts,key=lambda d:d['error']['rms'])[:5]
            results['cases'].append(row)
            panels=[gray(ref,cover,cover),gray(v,cover,valid),error_rgb(e,cover,valid)]
            for label,im in zip(('reference','visibility','error'),panels):
                Image.fromarray(im).save(args.output/f'{mode}-{pose}-{label}.png')
            if pose==0:
                # Select spatially separated worst partials, plus solid flips.
                indices=np.argsort(np.where(p,abs(e),-1).ravel())[::-1]
                points=[]
                for i in indices:
                    y,x=np.unravel_index(i,ref.shape)
                    if all((x-a)**2+(y-b)**2>60**2 for a,b in points): points.append((int(x),int(y)))
                    if len(points)==8: break
                for kind in ('lit','umbra'):
                    sel=cover&valid&((ref==1) if kind=='lit' else (ref==0))
                    y,x=np.unravel_index(np.argmax(np.where(sel,abs(e),-1)),ref.shape)
                    points.append((int(x),int(y)))
                row['crops']=[]
                for x,y in points:
                    box=(x-32,y-32,x+33,y+33)
                    sheet(panels,['reference','UE '+mode,'signed error +/-1'],args.output/f'{mode}-crop-{x}-{y}.png',box,4)
                    row['crops'].append(dict(x=x,y=y,reference=float(ref[y,x]),candidate=float(v[y,x]),
                        nl=float(nl[y,x]),world=world[y,x].tolist(),normal=normal[y,x].tolist(),
                        unshadowed=unshadowed[y,x].tolist(),shadowed=shadowed[y,x].tolist()))
    results['seconds']=time.monotonic()-start
    (args.output/'diagnosis.json').write_text(json.dumps(results,indent=2,allow_nan=False)+'\n')
    print(json.dumps([dict(mode=r['mode'],pose=r['pose'],partial=r['partial'],populations=r['populations'],
                          affine=r['affine_diagnostic']) for r in results['cases']],indent=2))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=ROOT)
    parser.add_argument('--reference',type=Path,default=REFERENCE)
    parser.add_argument('--output',type=Path,required=True)
    analyze(parser.parse_args())
