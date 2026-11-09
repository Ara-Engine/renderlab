"""Frozen test-binary suites, inside a GPU lease: BINARY_TAG rf|qwalk|post|lc2.

Test executables are P_MEM_OUT/bin/<tag>-tests.exe. Registered fixture/camera inputs
are reused; output is redirected to a per-suite log and full-image hashes.
"""
import json,os,subprocess,sys,hashlib
from pathlib import Path

root=Path(os.environ.get('P_MEM_OUT','D:/Coding/renderlab-work/p-mem'))
tag,kind=sys.argv[1:]
binary=root/'bin'/f'{tag}-tests.exe'
schedule=Path('E:/renderlab-work/gpu-schedule.md')
order=next(x for x in reversed(schedule.read_text().splitlines()) if x.startswith('[root order]'))
for mode in ['rt','nonrt']:
    latest=next(x for x in reversed(schedule.read_text().splitlines()) if x.startswith('[root order]'))
    if latest!=order:
        if 'Running leases are not stopped' not in latest and 'p-mem baseline' not in latest:raise RuntimeError('new root hold')
        order=latest
    out=root/tag/f'{kind}-{mode}';out.mkdir(parents=True,exist_ok=True)
    env=os.environ|{'VK_INSTANCE_LAYERS':''}
    if kind=='rf':
        env.update(RENDERLAB_M7_R0A_SCENES=';'.join(f'E:/renderlab-work/m7-r0a/scenes/RF{i}' for i in range(1,11)),
            RENDERLAB_M7_R0A_PATH=mode,RENDERLAB_M7_REFERENCE_OUT=str(out),RENDERLAB_R1_TR1='1')
        test='gpu_r0a_current'
    elif kind=='qwalk':
        m=json.loads(Path(f'D:/Coding/renderlab-work/m7-tr1/plans/pv-quarry/manifest-{mode}.json').read_text())
        m.update(output=str(out),variants={'native':{}},write_frames=True,write_color=True,
            frames=list(range(161)),current_defaults=True)
        manifest=out/'manifest.json';manifest.write_text(json.dumps(m,indent=2))
        env['RENDERLAB_PRIMARY_MANIFEST']=str(manifest)
        test='gpu_tr1_sequence'
    elif kind=='post':
        env['P_MEM_POST_RECORD']='1'
        manifest=out/'manifest.json';manifest.write_text(json.dumps({'path':mode,'output':str(out),'fixture_only':True}))
        env['RENDERLAB_PRIMARY_MANIFEST']=str(manifest)
        test='gpu_post_sweep'
    elif kind=='lc2':
        env.update(LC1_PATH='traced' if mode=='rt' else 'nonrt',LC1_OUT=str(out),LC3='1')
        test='gpu_lc2_penumbra'
    else:raise RuntimeError(kind)
    with (out/'stdout.log').open('w') as f:
        p=subprocess.run([str(binary),test,'--ignored','--nocapture','--test-threads=1'],env=env,stdout=f,stderr=subprocess.STDOUT,creationflags=subprocess.CREATE_NO_WINDOW)
    suffixes={'rf':{'.f32'},'qwalk':{'.rgba8'},'post':{'.png'},'lc2':{'.png'}}[kind]
    hashes={str(p.relative_to(out)):hashlib.file_digest(p.open('rb'),'sha256').hexdigest() for p in out.rglob('*') if p.suffix in suffixes}
    (out/'hashes.json').write_text(json.dumps(hashes,indent=2))
    reference=root/'baseline1080'/f'{kind}-{mode}'
    if tag.startswith('candidate'):
        if kind=='qwalk':
            actual=json.loads((out/'tr1.json').read_text())['variants']['native']['hashes']
            expected=json.loads((reference/'tr1.json').read_text())['variants']['native']['hashes']
        elif (reference/'hashes.json').exists():
            actual=hashes;expected=json.loads((reference/'hashes.json').read_text())
        else:actual=expected=None
        if actual is not None and actual!=expected:raise RuntimeError(f'IDENTITY FAILED {kind}-{mode}')
    if kind=='qwalk':
        from PIL import Image
        for frame in [0,64,160]:
            raw=out/'native'/f'f{frame:03}.rgba8'
            if raw.exists():Image.frombytes('RGBA',(1920,1080),raw.read_bytes()).save(out/f'frame-{frame:03}.png')
        for raw in out.rglob('*.rgba8'):
            if not raw.resolve().is_relative_to(out.resolve()):raise RuntimeError('capture outside owned output')
            if raw.stat().st_size>5_000_000:raw.unlink()
    print(tag,kind,mode,'exit',p.returncode,'hashes',len(hashes),flush=True)
    if p.returncode:raise RuntimeError(f'{kind} {mode} failed; inspect log')
