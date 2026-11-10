"""CPU evidence: visibility deltas and unamplified final-image fringe sheets."""
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

OUT = Path('D:/Coding/renderlab-work/p-rts2')
CASE = 'camera-1px-r0.04'
SHAPE = (288,512,4)
DTYPE = np.dtype([('counts','<u4',(4,)),('moments','<f4',(4,)),('radiance','<f4',(4,))])
base = OUT/'runs/base'/CASE/'TR1-Auto32'
def read(p):
    return np.fromfile(p,dtype='<f2').reshape(SHAPE).astype('f4')
def display(a):
    # Standard sRGB display transfer, at the fixture's existing exposure.
    a=np.maximum(a[...,:3],0)
    a=a/(1+a)
    return np.uint8(np.clip(np.where(a<=.0031308,a*12.92,1.055*a**(1/2.4)-.055),0,1)*255+.5)
summary=[]
for variant in sorted((OUT/'runs').iterdir()):
    if variant.name=='base': continue
    directory=variant/CASE/'TR1-Auto32'
    rows=[]
    frames=sorted(directory.glob('*-visibility.rgba16f'))
    if not frames: continue
    # Fix the crop at the greatest final-image difference over the sequence.
    peak=(-1,0,0)
    for p in frames:
        f=int(p.name[:4]); a=read(base/f'{f:04}.rgba16f'); b=read(directory/f'{f:04}.rgba16f')
        delta=np.max(np.abs(display(a).astype(float)-display(b).astype(float)),axis=2)
        y,x=np.unravel_index(delta.argmax(),delta.shape)
        if delta[y,x]>peak[0]: peak=(float(delta[y,x]),int(x),int(y))
    x=max(0,min(416,peak[1]-48)); y=max(0,min(240,peak[2]-24))
    for page in range((len(frames)+39)//40):
        sheet=Image.new('RGB',(1200,10*126),(24,24,24)); draw=ImageDraw.Draw(sheet)
        for j,p in enumerate(frames[page*40:(page+1)*40]):
            f=int(p.name[:4]); a=read(base/p.name)[...,0]; b=read(p)[...,0]
            ref_path=OUT/'runs/base'/CASE/'reference'/f'pose-{f}.film.bin'
            film=np.fromfile(ref_path,dtype=DTYPE).reshape(288,512)
            c=film['counts'].astype(float); n=c[...,2]+c[...,3]
            ref=np.divide(c[...,0]+c[...,1],256*n,out=np.ones(n.shape),where=n>0)
            rows.append(dict(frame=f,changed=int(np.count_nonzero(a!=b)),max_visibility_delta=float(np.max(np.abs(a-b))),
                             baseline_rms=float(np.sqrt(np.mean((a-ref)**2))),candidate_rms=float(np.sqrt(np.mean((b-ref)**2)))))
            images=[film['radiance'].astype(float)/256,read(base/f'{f:04}.rgba16f'),read(directory/f'{f:04}.rgba16f')]
            col=j%4; row=j//4; left=col*300; top=row*126
            draw.text((left,top),f'{variant.name} f{f} ref/base/cand',(255,255,255))
            for k,img in enumerate(images):
                tile=Image.fromarray(display(img)[y:y+48,x:x+96]).resize((96,96),Image.Resampling.NEAREST)
                sheet.paste(tile,(left+k*100,top+22))
        path=OUT/'sheets'/f'{variant.name}-{page}.png';path.parent.mkdir(exist_ok=True)
        sheet.save(path)
    summary.append(dict(variant=variant.name,crop=[x,y,96,48],max_display_codes=peak[0],frames=rows))
(OUT/'scores.json').write_text(json.dumps(summary,indent=2))
print(json.dumps([dict(variant=v['variant'],frames=len(v['frames']),max_display_codes=v['max_display_codes']) for v in summary],indent=2))
