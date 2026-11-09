"""Resource-only tails for short processes missed by the WDDM sampler.

The first 88 frames and all settings match the timing case exactly. An additional
2,048 stationary frames keep resources alive for counter sampling. These runs do
not replace timing observations, startup values or quality captures.
"""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys

from capture import REPO, digest
from matrix import CONTENTION
from prepare import ROOT, save


def main():
    cases=[('game-base','rt'),('ara-pbr-oriented','rt'),('ara-pbr-oriented','raster'),
           ('game-no-gi','rt'),('game-no-gi','raster'),('game-no-reflections','rt'),('game-post','rt')]
    for case,mode in cases:
        original=ROOT/'phase2-scenes'/f'{case}-timing.json'
        existing=[p for p in (ROOT/'memory-supplements').glob('*/qualification.json') if json.loads(p.read_text())['original_scene_sha256']==digest(original)
                  and json.loads(Path(json.loads(p.read_text())['process']).read_text())['mode']==mode]
        if existing:
            print('reusing',existing[0],flush=True);continue
        spec=json.loads(original.read_text())
        extra=copy.deepcopy(spec['frames'][-1]);extra.update(phase='memory-tail',cut=False,capture=False)
        spec['frames'] += [copy.deepcopy(extra) for _ in range(2048)]
        spec['notes'].append('Resource-only 2048-frame stationary tail; original first 88 poses/settings unchanged; not a timing or quality replacement')
        folder=ROOT/'memory-supplements'/f'{case}--{mode}';folder.mkdir(parents=True,exist_ok=True)
        path=folder/'scene.json';save(path,spec)
        engine='renderlab' if mode=='rt' else 'ara'
        command=[sys.executable,'-B',str(REPO/f'tools/ara1/run-{engine}.py'),
            '--binary',str(ROOT/'bin/final'/('rl-capture.exe' if engine=='renderlab' else 'ara.exe')),
            '--source',str(REPO if engine=='renderlab' else ROOT/'ara-main'),'--scene',str(path),
            '--mode',mode,'--out',str(folder/mode),'--profile','release','--contention',CONTENTION+'; resource-only retention tail']
        if engine=='ara':command += ['--bake-cache',str(ROOT/'bakes'/('p2-ara-pbr-oriented-normal' if case.startswith('ara-pbr') else 'p2-game-normal'))]
        subprocess.run(command,env=os.environ|{'RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK':'I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK'},check=True)
        record=json.loads((folder/mode/'process.json').read_text())
        if not record.get('peak_process_dedicated_bytes'):raise RuntimeError('WDDM still unavailable; keep the unavailable result')
        save(folder/'qualification.json',dict(original_scene=str(original),original_scene_sha256=__import__('capture').digest(original),
            process=str(folder/mode/'process.json'),dedicated_bytes=record['peak_process_dedicated_bytes'],
            scope='separate same-configuration run, first 88 poses/settings identical, with a stationary 2048-frame tail; sampled peak is not an allocator/residency certificate'))
        print(case,record['peak_process_dedicated_bytes'],flush=True)


if __name__=='__main__':main()
