"""Score RT1b's batched current-policy captures, retaining every failed pixel.

The canonical R1 film/native masks are read-only. F0's 1 mm reference never
supplies the acceptance populations. All generated files stay on E:.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import time

for key in ('OPENBLAS_NUM_THREADS','OMP_NUM_THREADS','NUMBA_NUM_THREADS'):
    os.environ[key]='2'
import numpy as np
from PIL import Image


def display(rgb):
    x=np.maximum(0,rgb);x=x/(1+x)
    return np.where(x<=.0031308,x*12.92,1.055*x**(1/2.4)-.055)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    spec=importlib.util.spec_from_file_location('rt1',Path(__file__).with_name('rt1-film-replay.py'))
    rt1=importlib.util.module_from_spec(spec);spec.loader.exec_module(rt1)
    rt1.lb.NATIVE=Path('D:/Coding/renderlab-work/ue-m5/Quarry/native-shadow')
    start=time.perf_counter();result={};identities={}
    images=a.directory/'images';images.mkdir(exist_ok=True)
    for pose in (0,1):
        directory=Path(f'E:/renderlab-work/m6-exact-quality/r1/quarry-viewed-p{pose}')
        d=rt1.film_reference(directory,pose)
        film=rt1.read(directory/f'pose-{pose}.film.bin',rt1.FILM)
        radiance=film['rest'][...,4:7].astype(float)/256
        hits=rt1.read(a.directory/f'pose-{pose}-baseline.hit.u32','<u4',(*rt1.SHAPE,4))
        valid=hits[...,0]!=0
        baseline=rt1.read(a.directory/f'pose-{pose}-baseline.u32','<u4')/256
        rows={}
        for path in sorted(a.directory.glob(f'pose-{pose}-*.u32')):
            if '.hit.' in path.name:continue
            name=path.stem.removeprefix(f'pose-{pose}-')
            candidate=rt1.read(path,'<u4')/256
            score=rt1.metrics(candidate,d,valid)
            ref=d['ref'];false=d['cover']&(d['n']>0)&valid&(ref>=.99)&(candidate<=.9)
            oldfalse=d['cover']&(d['n']>0)&valid&(ref>=.99)&(baseline<=.9)
            score.update(new_false=int((false&~oldfalse).sum()),cleared_false=int((oldfalse&~false).sum()))
            ys,xs=np.nonzero(false)
            with (a.directory/f'pose-{pose}-{name}-false.csv').open('w') as f:
                f.write('x,y,film,baseline,candidate,new\n')
                for y,x in zip(ys,xs):f.write(f'{x},{y},{ref[y,x]},{baseline[y,x]},{candidate[y,x]},{not oldfalse[y,x]}\n')
            hdrpath=path.with_suffix('.rgba16f')
            rgb=rt1.read(hdrpath,'<f2',(*rt1.SHAPE,4))[...,:3].astype(float)
            score['whole_hdr_rms']=float(np.sqrt(np.mean((rgb-radiance)**2)))
            score['display_rms_codes']=float(np.sqrt(np.mean((display(rgb)-display(radiance))**2))*255)
            Image.fromarray(np.uint8(np.clip(display(rgb),0,1)*255)).save(images/f'pose-{pose}-{name}-hdr.png')
            error=candidate-ref;im=np.zeros((*rt1.SHAPE,3),dtype='u1')
            im[...,0]=np.clip(-error*8*255,0,255)*d['cover'];im[...,2]=np.clip(error*8*255,0,255)*d['cover']
            Image.fromarray(im).save(images/f'pose-{pose}-{name}-error.png')
            chosen=[dict(x=int(x),y=int(y)) for y,x in list(zip(ys,xs))[::max(1,len(xs)//16)][:16]]
            if chosen:rt1.crops(images/f'pose-{pose}-{name}-false.png',chosen,[('Film',ref),('Baseline',baseline),(name,candidate)])
            with path.open('rb') as f:identities[path.name]=hashlib.file_digest(f,'sha256').hexdigest()
            with hdrpath.open('rb') as f:identities[hdrpath.name]=hashlib.file_digest(f,'sha256').hexdigest()
            rows[name]=score
        for case,name in [('vsm','VSM'),('hrt1','HRT')]:
            with np.load(rt1.lb.native_dir(case,pose)/'native.npz') as native:
                rows[name]=rt1.metrics(native['visibility'],d,native['valid'])
        times=json.loads((a.directory/f'pose-{pose}-timing.json').read_text())
        for name,row in rows.items():
            selected=[r for r in times if r['name']==name]
            if selected:
                row['receiver_ms']=float(np.median([r['receiver_ms'] for r in selected]))
                row['span_ms']=float(np.median([r['span_ms'] for r in selected]))
        result[str(pose)]=rows
    result['cpu_seconds']=time.perf_counter()-start
    (a.directory/'scores.json').write_text(json.dumps(result,indent=2))
    (a.directory/'capture-identities.json').write_text(json.dumps(identities,indent=2))
    for pose in ('0','1'):
        for name,r in result[pose].items():
            print(pose,name,'false',r['false_shadows'],'new',r.get('new_false'),
                  'RMS',round(r['partial_rms'],6),'flips',r['solid_flips'],'leaks',r['umbra_leaks'],
                  'lit',r['lit_darkened'],'edges',r['lost_umbra_edges'],'receiver_ms',r.get('receiver_ms'))
    print('CPU seconds',result['cpu_seconds'])


if __name__=='__main__':main()
