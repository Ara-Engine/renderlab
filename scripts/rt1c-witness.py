"""Replay S2's registered display metric on a current-source cropped film witness.

The HRT image is the retained matched, native-resolution post-TSR capture. No
fitted exposure, background substitution, or population exclusion is applied.
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw


def tone(rgb, stops):
    v = np.maximum(0, rgb * 2**stops)
    v = v / (1+v)
    v = np.where(v <= .0031308, 12.92*v, 1.055*v**(1/2.4)-.055)
    return np.rint(np.clip(v, 0, 1)*255).astype('u1')


def rms(a, b):
    return float(np.sqrt(np.mean((a.astype(float)-b.astype(float))**2)))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('directory', type=Path, help='current witness case directory')
    p.add_argument('--recorded-root', type=Path, default=Path('E:/renderlab-work/motion-benches/S2'))
    a = p.parse_args()
    case = a.directory.name
    rect = (704, 480, 1216, 768)
    shape = (288, 512)
    candidate_path = a.directory/'Interactive/0000.rgba16f'
    film_path = a.directory/'reference/pose-0.film.bin'
    dtype = np.dtype([('counts','<u4',4),('moments','<f4',4),('radiance','<f4',4)])
    film = np.fromfile(film_path, dtype=dtype).reshape(shape)['radiance'][...,:3].astype(float)/256
    candidate = np.fromfile(candidate_path, dtype='<f2').reshape(*shape,4)[...,:3].astype(float)
    result = dict(current_hdr_rms=rms(candidate,film),scope='native-pixel S2 crop; one current-source frame; not a full motion pass')
    inputs = [candidate_path, film_path]
    for stops in (0,4):
        suffix = '-plus4' if stops else ''
        native_path = a.recorded_root/f'unreal/{case}/HRT1/0000/final-radiance{suffix}.png'
        oldfilm_path = a.recorded_root/f'renderlab/{case}/film-full{suffix}.png'
        oldfast_path = a.recorded_root/f'renderlab/{case}/Interactive/0000-full{suffix}.png'
        native = np.asarray(Image.open(native_path).convert('RGB').crop(rect))
        oldfilm = np.asarray(Image.open(oldfilm_path).convert('RGB').crop(rect))
        oldfast = np.asarray(Image.open(oldfast_path).convert('RGB').crop(rect))
        reference, fast = tone(film,stops), tone(candidate,stops)
        ar,br = rms(fast,reference),rms(native,reference)
        result[f'display_{stops:+}_codes'] = dict(interactive=ar,hrt1=br,passes=ar <= br,
            fresh_vs_recorded_film_rms=rms(reference,oldfilm),fresh_vs_recorded_film_differing_channels=int(np.count_nonzero(reference!=oldfilm)),
            fresh_vs_recorded_interactive_differing_channels=int(np.count_nonzero(fast!=oldfast)))
        inputs.extend([native_path,oldfilm_path,oldfast_path])
        panels=[('Current film',reference),('Current Interactive',fast),('Recorded HRT1',native),
                ('Interactive absolute error x4',np.clip(np.abs(fast.astype(float)-reference)*4,0,255).astype('u1')),
                ('HRT1 absolute error x4',np.clip(np.abs(native.astype(float)-reference)*4,0,255).astype('u1'))]
        sheet=Image.new('RGB',(512*3,312*2),(20,27,35));draw=ImageDraw.Draw(sheet)
        for i,(name,pixels) in enumerate(panels):
            x,y=i%3*512,i//3*312
            draw.text((x+5,y+4),name,fill='white');sheet.paste(Image.fromarray(pixels),(x,y+24))
        sheet.save(a.directory/f'witness{suffix}.png')
    result['inputs']={str(path):hashlib.sha256(path.read_bytes()).hexdigest() for path in inputs}
    (a.directory/'witness-score.json').write_text(json.dumps(result,indent=2))
    print(json.dumps(result,indent=2))


if __name__ == '__main__':
    main()
