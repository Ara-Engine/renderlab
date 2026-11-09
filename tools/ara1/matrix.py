"""Frozen phase-two cases and sequential, resumable leased execution.

CPU-only `prepare` reuses the already converted, identical assets. `run` must be
inside lease.py; completed captures are immutable and only successful ones skip.
"""
import argparse
import copy
import json
import math
import os
from pathlib import Path
import subprocess
import sys

from capture import REPO, digest
from prepare import ROOT, frames, save
import numpy as np

ARA = ROOT / 'ara-main'
V3 = Path('C:/Users/snowy/.t3/worktrees/snengine/t3-9c4a1f92')
CONTENTION = 'shared machine; one owned headless renderer; lease resource log records foreign CPU builds, available RAM and GPU utilization; no quiet-window performance certification'
WITNESSES = {0, 31, 38, 39, 40, 41, 42, 43, 47, 63, 70, 71, 72, 73, 74, 75, 76, 80, 87}


def projected_box(spec, center, radius):
    f=spec['frames'][39];eye=np.asarray(f['eye']);forward=np.asarray(f['target'])-eye
    forward=forward/np.linalg.norm(forward);right=np.cross(forward,[0,1,0]);right/=np.linalg.norm(right)
    up=np.cross(right,forward);tan=math.tan(f['fov_y']/2);aspect=spec['size'][0]/spec['size'][1]
    delta=np.asarray(center)-eye;depth=np.dot(delta,forward)
    x=.5+np.dot(delta,right)/depth/tan/aspect/2;y=.5-np.dot(delta,up)/depth/tan/2
    dx=radius/depth/tan/aspect/2;dy=radius/depth/tan/2
    return [max(0,x-dx),max(0,y-dy),min(1,x+dx),min(1,y+dy)]


def prepare():
    folder = ROOT/'phase2-scenes'
    folder.mkdir(exist_ok=True)
    for name in ('game', 'ara-pbr', 'quarry'):
        original = ROOT/'scenes-dry'/f'{name}.json'
        spec = json.loads(original.read_text())
        asset = Path(spec['asset'])
        spec['asset'] = str(asset if asset.is_absolute() else original.parent/asset)
        f = spec['frames'][0]
        spec['frames'] = frames(f['eye'], f['target'], f['fov_y'], False)
        spec['synchronous'] = True
        variants = {'base': {}}
        if name == 'game':
            variants |= {'no-gi': {'gi': False}, 'no-reflections': {'reflections': False},
                         'alpha': {'alpha_panel': True}, 'atmosphere': {'atmosphere': True},
                         'fog': {'fog': True}, 'post': {'post': True},
                         'lights': {}, 'lights-unshadowed': {'local_shadows': False}}
        for variant, effects in variants.items():
            case = copy.deepcopy(spec)
            case['effects'] = effects
            case['name'] += '-'+variant
            if variant.startswith('lights'):
                case['sun_lux'] = [1000]*3
                case['environment'] = [30,35,45]
                case['exposure_stops'] = -8
                case['lights'] = [dict(position=[-5+(i%8)*1.5, 1+(i//8%2)*1.7, -2+(i//8)*1.25],
                    color=[[1,.25,.12],[.12,.4,1],[.2,1,.3],[1,.8,.3]][i%4], lumens=1800,
                    range=6, spot=None) for i in range(64)]
            for resolution, size in [('quality',[960,540]), ('timing',[1920,1080])]:
                item = copy.deepcopy(case)
                item['size'] = size
                for i, frame in enumerate(item['frames']):
                    frame['capture'] = i in (WITNESSES if resolution=='quality' else {39,71,87})
                path = folder/f'{name}-{variant}-{resolution}.json'
                if path.exists() and json.loads(path.read_text()) != item:
                    raise RuntimeError(f'refusing to change frozen scene {path}')
                save(path,item)
    save(folder/'identities.json', {p.name:digest(p) for p in folder.glob('*.json') if p.name!='identities.json'})
    game=json.loads((folder/'game-base-quality.json').read_text())
    save(folder/'regions.json',dict(registration='selected from world positions before candidate/reference scoring; fixed screen crops, no motion tracking',
        game={
            'gi_bounce':projected_box(game,[-4.8,.5,0],1.6),
            'sun_shadow':projected_box(game,[-.5,.05,-2],2.5),
            'mirror':projected_box(game,[-1.2,.8,5.6],1.0),
            'glossy':projected_box(game,[1.8,1,3.2],1.15),
            'silhouette':projected_box(game,[0,4.4,-3],2.7),
            'alpha_overlap':projected_box(game,[1.8,1.3,4.3],1.65),
            'sky_horizon':[0,0,1,.25], 'light_overlap':projected_box(game,[2,.7,1],3)},
        quarry={'surface_detail':[.2,.2,.8,.85],'shadow_contact':[.3,.5,.7,.9],'sky_horizon':[0,0,1,.2]},
        **{'ara-pbr':{'pbr_grid':[.1,.3,.85,.9],'texture_detail':[.32,.3,.65,.8],'sky_horizon':[0,0,1,.18]}}))
    print(folder)


def run(a):
    scene = ROOT/'phase2-scenes'/f'{a.case}-{a.resolution}.json'
    spec = json.loads(scene.read_text())
    asset_name = a.case.split('-base')[0] if '-base' in a.case else 'game'
    if a.case.startswith('ara-pbr'): asset_name = 'ara-pbr'
    if a.case.startswith('quarry'): asset_name = 'quarry'
    cache = ROOT/'bakes'/f'p2-{asset_name}{"-alpha" if spec.get("effects",{}).get("alpha_panel") else ""}-normal'
    group = ROOT/'phase2-runs'/a.tag/f'{a.case}-{a.resolution}'
    for mode in a.engines.split(','):
        out = group/mode
        if out.exists():
            record = json.loads((out/'process.json').read_text()) if (out/'process.json').exists() else {}
            if record.get('exit_code') == 0 and record.get('scene_sha256') == digest(scene):
                print(f'reusing {out}',flush=True)
                continue
            raise RuntimeError(f'incomplete output needs a new tag: {out}')
        engine = 'renderlab' if mode in ('rt','nonrt','reference') else 'ara'
        binary = ROOT/'bin/a2c92b1'/('rl-reference.exe' if mode=='reference' else 'rl-capture.exe' if engine=='renderlab' else 'ara.exe')
        commands = []
        if engine=='ara' and not (cache/'bake.json').exists():
            commands.append(('bake',group/'bake'))
        commands.append((mode,out))
        for actual, target in commands:
            command=[sys.executable,'-B',str(REPO/f'tools/ara1/run-{engine}.py'),
                '--binary',str(binary),'--source',str(REPO if engine=='renderlab' else ARA),
                '--scene',str(scene),'--mode',actual,'--out',str(target),'--profile','release','--contention',CONTENTION]
            if engine=='ara':command += ['--bake-cache',str(cache)]
            environment=os.environ | {'RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK':'I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK'}
            if mode=='reference':
                environment |= {'ARA1_REFERENCE_SAMPLES':str(a.samples),'ARA1_REFERENCE_INDICES':a.indices}
            subprocess.run(command,env=environment,check=True)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    sub=p.add_subparsers(dest='command',required=True)
    sub.add_parser('prepare')
    r=sub.add_parser('run')
    r.add_argument('--case',required=True)
    r.add_argument('--resolution',choices=['quality','timing'],default='quality')
    r.add_argument('--engines',default='nonrt,rt,raster,hybrid')
    r.add_argument('--tag',required=True)
    r.add_argument('--samples',type=int,default=1024)
    r.add_argument('--indices',default='39')
    a=p.parse_args()
    if a.command=='prepare':
        prepare()
    else:
        for case in a.case.split(','):
            for resolution in a.resolution.split(','):
                item=copy.copy(a);item.case=case;item.resolution=resolution
                run(item)


if __name__=='__main__': main()
