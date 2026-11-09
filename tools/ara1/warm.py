"""Three warm startup observations, plus visible live WGSL edits on both RL paths."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

from capture import REPO
from prepare import ROOT, save
from matrix import CONTENTION


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--tag',required=True);a=p.parse_args()
    original=ROOT/'phase2-scenes/game-base-quality.json'
    scene=json.loads(original.read_text());scene['frames']=scene['frames'][:1]
    scene['frames'][0]['capture']=True
    path=ROOT/'phase2-scenes/game-startup.json'
    if path.exists() and json.loads(path.read_text())!=scene:raise ValueError('startup scene changed')
    save(path,scene)
    base=ROOT/'phase2-runs/final/game-base-quality'
    for mode in ('nonrt','rt','raster','hybrid'):
        engine='renderlab' if mode in ('nonrt','rt') else 'ara'
        for repeat in range(3):
            # RL's application pipeline cache is a sibling of the output and is
            # shared with the completed quality run, without altering its images.
            out=base/f'{a.tag}-{mode}-{repeat}'
            binary=ROOT/'bin/final'/('rl-capture.exe' if engine=='renderlab' else 'ara.exe')
            cmd=[sys.executable,'-B',str(REPO/f'tools/ara1/run-{engine}.py'),
                '--binary',str(binary),'--source',str(REPO if engine=='renderlab' else ROOT/'ara-main'),
                '--scene',str(path),'--mode',mode,'--out',str(out),'--profile','release','--contention',CONTENTION]
            if engine=='ara':cmd += ['--bake-cache',str(ROOT/'bakes/p2-game-normal'),'--cache-from',str(base/mode/'gpu-cache')]
            environment=os.environ|{'RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK':'I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK'}
            if engine=='renderlab':environment['ARA1_SHADER_RELOAD']='1'
            subprocess.run(cmd,env=environment,check=True)


if __name__=='__main__':main()
