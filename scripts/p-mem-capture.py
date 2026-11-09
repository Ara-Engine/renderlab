"""Frozen-binary 1080p ARA1 inventory and HDR/PNG identity captures, inside a GPU lease.

Usage: p-mem-capture.py BINARY_TAG SCENE... (binaries in P_MEM_OUT/bin).
P_MEM_CAPTURE_ONLY=1 skips RF/post suites; use p-mem-fixtures.py for explicit suites.
"""
import json,os,sys,subprocess,time,hashlib
from pathlib import Path
from p_mem_schedule import latest_order

root=Path(os.environ.get('P_MEM_OUT','D:/Coding/renderlab-work/p-mem'))
source=Path(os.environ.get('P_MEM_SCENES','D:/Coding/renderlab-work/ara1/phase2-scenes'))
schedule=Path('E:/renderlab-work/gpu-schedule.md')
order=latest_order(schedule)
tag=sys.argv[1]
binary=root/'bin'/f'{tag}.exe'
cases=sys.argv[2:] or ['game-base','ara-pbr-oriented','game-lights','quarry-daylight']
for case in cases:
    for mode in ['rt','nonrt']:
        latest=latest_order(schedule)
        if latest!=order:
            if 'Running leases are not stopped' not in latest and 'p-mem baseline' not in latest: raise RuntimeError('new root hold; release and re-read')
            order=latest
        out=root/tag/f'{case}-{mode}';out.mkdir(parents=True,exist_ok=True)
        original='game-lights' if case=='game-spots' else case
        spec=json.loads((source/f'{original}-quality.json').read_text())
        timing=json.loads((source/f'{original}-timing.json').read_text())
        assert [{k:v for k,v in f.items() if k!='capture'} for f in spec['frames']]==[{k:v for k,v in f.items() if k!='capture'} for f in timing['frames']]
        spec['size']=timing['size']
        if case=='game-spots':
            spec['name']='P-MEM all-spot allocation witness'
            for light in spec['lights']:light['spot']={'travel':[0.0,-1.0,0.0],'inner':0.4,'outer':0.8}
        scene=out/'scene.json';scene.write_text(json.dumps(spec,indent=2))
        env=os.environ|{'ARA1_HDR_HASH':'1','ARA1_MEMORY':'1','VK_INSTANCE_LAYERS':''}
        start=time.monotonic()
        with (out/'stdout.log').open('w') as f:
            p=subprocess.run([str(binary),str(scene),mode,str(out)],env=env,stdout=f,stderr=subprocess.STDOUT,creationflags=subprocess.CREATE_NO_WINDOW)
        if p.returncode:raise RuntimeError(f'{case}-{mode} exit {p.returncode}')
        m=json.loads((out/'memory.json').read_text())
        e=json.loads((out/'engine.json').read_text())
        hashes={p.name:hashlib.file_digest(p.open('rb'),'sha256').hexdigest() for p in out.glob('*.png')}
        hashes['hdr']={str(f['index']):f['hdr_sha256'] for f in e['frames'] if f['hdr_sha256']}
        (out/'hashes.json').write_text(json.dumps(hashes,indent=2))
        reference=root/'baseline1080'/f'{case}-{mode}'/'hashes.json'
        if tag.startswith('candidate') and reference.exists():
            expected=json.loads(reference.read_text())
            if hashes!=expected:raise RuntimeError(f'IDENTITY FAILED {case}-{mode}; inspect hashes before proceeding')
        print(tag,case,mode,'seconds',round(time.monotonic()-start,2),'allocated/reserved MiB',m['allocated_bytes']/2**20,m['reserved_bytes']/2**20,'dxgi',m['dxgi'],flush=True)
if 'quarry-base' not in cases and os.environ.get('P_MEM_CAPTURE_ONLY')!='1':
    for kind in ['rf','post']:
        latest=latest_order(schedule)
        if latest!=order:
            if 'Running leases are not stopped' not in latest and 'p-mem baseline' not in latest:raise RuntimeError('new root hold')
            order=latest
        subprocess.run([sys.executable,'-B',str(Path(__file__).with_name('p-mem-fixtures.py')),tag,kind],env=os.environ,check=True,creationflags=subprocess.CREATE_NO_WINDOW)
