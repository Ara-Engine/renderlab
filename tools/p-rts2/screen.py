"""Cheap first screen: one contiguous S2 track, then its registered film reference.

Run only within an n8 lease. A failing first track rejects a variant without a port.
"""
import json
import os
from pathlib import Path
import subprocess as sp
import time

OUT = Path('D:/Coding/renderlab-work/p-rts2')
VARIANTS = [('base',8,2,1), ('disk8-r2',8,2,1), ('disk8-r3',8,3,1),
            ('disk8-r4',8,4,1), ('disk16-r2',16,2,1), ('disk16-r3',16,3,1),
            ('cone15-r2',8,2,1.5)]
for name,count,radius,cone in VARIANTS:
    assert time.strftime('%Y-%m-%dT%H:%M:%S',time.gmtime()) < '2026-11-10T01:51:00', 'screen deadline'
    manifest = dict(scene='E:/renderlab-work/motion-benches/S2/fixture/scene.json',
                    output=str(OUT/'runs'/name), modes=['TR1-Auto32'],
                    cases=['camera-1px-r0.04'], no_full_captures=True,
                    registered_direct=True, repeats=0, skip_reference=name!='base')
    path=OUT/f'{name}.json'
    path.write_text(json.dumps(manifest,indent=2))
    env = os.environ | dict(RENDERLAB_PRIMARY_MANIFEST=str(path), P_RTS2_VARIANT=name,
                           P_RTS2_COUNT=str(count), P_RTS2_RADIUS=str(radius), P_RTS2_CONE=str(cone))
    begin=time.monotonic()
    with (OUT/f'{name}.log').open('w') as log:
        result=sp.run([str(OUT/'screen.exe'),'gpu_motion_bench_batch','--ignored','--nocapture','--test-threads=1'],
                      cwd=OUT/'source',env=env,stdout=log,stderr=sp.STDOUT)
    print(json.dumps(dict(variant=name, exit=result.returncode, seconds=time.monotonic()-begin)),flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
