"""Frozen P-RETIME builds and compiler-audited, bounded ABBA GPU leases.

Run `py -3 -B scripts/p-retime.py build`, then `... measure`.
Evidence is local/ignored; no production source in this checkout is modified.
"""
import hashlib
import importlib.util
import inspect
import json
import os
from pathlib import Path
import statistics as st
import subprocess as sp
import sys
import threading
import time
import zipfile

REPO = Path(__file__).resolve().parents[1]
OUT = Path('D:/Coding/renderlab-work/p-retime')
TARGET = Path('D:/Coding/pretime-target')
SCHEDULE = Path('E:/renderlab-work/gpu-schedule.md')
SCENES = Path('D:/Coding/renderlab-work/ara1/phase2-scenes')
FLAGS = sp.CREATE_NO_WINDOW
OUT.mkdir(parents=True, exist_ok=True)


def save(path, value):
    path.write_text(json.dumps(value, indent=2))


def sha(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def post(line):
    with SCHEDULE.open('a', encoding='utf-8') as f:
        f.write('\n' + line + '\n')
    print(line, flush=True)


def order():
    latest = None
    with SCHEDULE.open(encoding='utf-8', errors='replace') as lines:
        for line in lines:
            if line.startswith('[root order]'):
                latest = line.strip()
    if latest is None:
        raise RuntimeError('no root order')
    return latest


def load_runner():
    os.environ.update(N8_OWNER='p-retime', N8_OUT=str(OUT / 'lease'))
    spec = importlib.util.spec_from_file_location('retime_n8', REPO / 'scripts/n8-run.py')
    n8 = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(n8)
    runner = n8.s1.runner
    # Extend the existing cheap Toolhelp sampler to audit every compiler, not rustc only.
    source = inspect.getsource(runner.resource_sample)
    source = source.replace('count = 0', 'count = 0; compilers = []')
    source = source.replace('count += entry.name.lower() == "rustc.exe"',
        'count += entry.name.lower() == "rustc.exe"\n            if entry.name.lower() in '
        '{"cargo.exe","rustc.exe","link.exe","lld-link.exe","cl.exe","clang-cl.exe","clippy-driver.exe"}: '
        'compilers.append({"pid": entry.pid, "name": entry.name})')
    source = source.replace('rustc_count=count)', 'rustc_count=count, compilers=compilers)')
    scope = dict(runner.__dict__)
    exec(source, scope)
    runner.resource_sample = scope['resource_sample']
    runner.PREFLIGHT_MEMORY_MBYTES = 8192
    runner.RUNTIME_MEMORY_MBYTES = 2048
    return n8, runner


def build():
    tree = OUT / 'source'
    tree.mkdir(exist_ok=True)
    binaries = OUT / 'bin'
    binaries.mkdir(exist_ok=True)
    records = []
    # Clean A merge, then the actual P-MEM merge pair. Both are first-parent main pairs.
    for name, commit in [('a-before','71f7228'),('a-after','6112205'),
                         ('mem-before','dc0d4ea'),('mem-after','241b87b')]:
        directive = order()
        # Compiler holds use owner lines only; a quoted phrase in a root order is not a hold.
        active = False
        with SCHEDULE.open(encoding='utf-8', errors='replace') as lines:
            for line in lines:
                if line.startswith('p-retime retime admitted'): active = True
                elif line.startswith('p-retime ') and 'window released' in line: active = False
        if active:
            raise RuntimeError('compiler hold active')
        archive = OUT / 'source.zip'
        sp.run(['git','archive','--format=zip','-o',str(archive),commit], cwd=REPO, check=True)
        with zipfile.ZipFile(archive) as z:
            z.extractall(tree)
        # Extraction refreshes timestamps, so shared-target fingerprints cannot reuse old code.
        for p in tree.rglob('*'):
            if p.is_file(): os.utime(p, None)
        env = os.environ | {'CARGO_TARGET_DIR':str(TARGET),'CARGO_BUILD_JOBS':'2'}
        start = time.monotonic()
        commands = [['cargo','build','--release','-j','2','--locked','--example','ara1_capture']]
        if name.startswith('a-'):
            commands.append(['cargo','test','--release','-j','2','--locked','--lib','--no-run','--message-format=json'])
        for k, command in enumerate(commands):
            log = OUT / f'build-{name}-{k}.log'
            print('BUILD',name,command,flush=True)
            with log.open('w') as f:
                sp.run(command,cwd=tree,env=env,stdout=f,stderr=sp.STDOUT,check=True,creationflags=FLAGS)
            if k == 0:
                import shutil
                shutil.copy2(TARGET / 'release/examples/ara1_capture.exe',binaries / f'{name}.exe')
            else:
                artifacts = [json.loads(s) for s in log.read_text().splitlines() if s.startswith('{')]
                exe = next(a['executable'] for a in artifacts if a.get('executable') and a.get('profile',{}).get('test'))
                shutil.copy2(exe,binaries / f'{name}-tests.exe')
        records.append(dict(name=name,commit=sp.check_output(['git','rev-parse',commit],cwd=REPO,text=True).strip(),
            seconds=time.monotonic()-start,sha256=sha(binaries / f'{name}.exe'),root_order=directive))
        save(OUT / 'builds.json',records)
        print('BUILT',records[-1],flush=True)


def build_bvh():
    """Isolate the software runtime packing from all other P-MEM changes."""
    import shutil
    tree = OUT / 'source'
    archive = OUT / 'bvh-source.zip'
    sp.run(['git','archive','--format=zip','-o',str(archive),'dc0d4ea'],cwd=REPO,check=True)
    with zipfile.ZipFile(archive) as z: z.extractall(tree)
    paths = ['src/gi_bvh.rs','shaders/gi_rays_software.wgsl']
    for path in paths:
        (tree / path).write_bytes(sp.check_output(['git','show',f'241b87b:{path}'],cwd=REPO))
    for path in tree.rglob('*'):
        if path.is_file(): os.utime(path,None)
    start = time.monotonic()
    with (OUT / 'build-mem-packed-only.log').open('w') as log:
        sp.run(['cargo','build','--release','-j','2','--locked','--example','ara1_capture'],cwd=tree,
            env=os.environ | {'CARGO_TARGET_DIR':str(TARGET),'CARGO_BUILD_JOBS':'2'},stdout=log,stderr=sp.STDOUT,
            creationflags=FLAGS,check=True)
    shutil.copy2(OUT / 'bin/mem-after.exe',OUT / 'bin/mem-merged.exe')
    shutil.copy2(TARGET / 'release/examples/ara1_capture.exe',OUT / 'bin/mem-after.exe')
    records = json.loads((OUT / 'builds.json').read_text())
    records.append(dict(name='mem-packed-only (measured as mem-after)',base='dc0d4ea',overlay_commit='241b87b',
        overlay_paths=paths,source_sha256={p:sha(tree / p) for p in paths},
        seconds=time.monotonic()-start,sha256=sha(OUT / 'bin/mem-after.exe')))
    save(OUT / 'builds.json',records)
    print('BUILT',records[-1],flush=True)


def capture(case, mode, index, variant):
    owner = json.loads(Path('D:/Coding/renderlab-work/gpu-slot.lock/owner.json').read_text())
    assert owner['owner'] == 'p-retime'
    n8, runner = load_runner()
    folder = OUT / 'runs' / case / mode / f'{index}-{variant}'
    folder.mkdir(parents=True,exist_ok=False)
    samples = []
    stop = threading.Event()
    def audit():
        while not stop.is_set():
            samples.append(runner.resource_sample())
            stop.wait(.1)
    thread = threading.Thread(target=audit)
    thread.start()
    start = time.time()
    env = os.environ | {'ARA1_HDR_HASH':'1','VK_LOADER_LAYERS_DISABLE':'VK_LAYER_KHRONOS_validation'}
    for key in ['ARA1_MEMORY','ARA1_STANDARD_PROGRAM','VK_INSTANCE_LAYERS']:
        env.pop(key,None)
    if case == 'soft-quarry':
        env.update(LC2='1',LC3='1',LC1_PATH='traced' if mode=='rt' else mode,
            LC1_OUT=str(folder),LC1_QUARRY='E:/renderlab-work/m7-g1c/quarry/manifest.json',LC1_FRAMES='8',
            LC2_NEAR='E:/renderlab-work/m7-g1c/near-cache/near-field-v1/rla1-rvg1/packages/743a2fce42fdf444407e23c2b00c0586ff0438806eeb023bfc19d91979c502f3')
        command = [str(OUT / 'bin' / f'{variant}-tests.exe'),
            'renderer::light_cluster_tests::gpu_lc1_quarry_night','--ignored','--exact','--nocapture','--test-threads=1']
    else:
        command = [str(OUT / 'bin' / f'{variant}.exe'),str(OUT / 'scenes' / f'{case}.json'),mode,str(folder)]
    try:
        with (folder / 'stdout.log').open('w') as f:
            sp.run(command,
                env=env,cwd=REPO,stdout=f,stderr=sp.STDOUT,check=True,creationflags=FLAGS,timeout=650)
    finally:
        stop.set(); thread.join()
        save(folder / 'audit.json',dict(start=start,end=time.time(),samples=samples))
    assert samples and not any(s['compilers'] for s in samples), 'compiler contamination: reject sample'
    if case == 'soft-quarry':
        reports = [p for p in folder.glob('*.json') if p.name != 'audit.json']
        assert len(reports)==1
        report = json.loads(reports[0].read_text())
        hashes = {p.name:sha(p) for p in folder.iterdir() if p.suffix in {'.png','.bin','.f32','.hdr'}}
        save(folder / 'result.json',dict(case=case,mode=mode,index=index,variant=variant,report=report,
            hashes=hashes,audit_samples=len(samples),compiler_samples=0,
            min_available_mib=min(s['available_mbytes'] for s in samples)))
        print('RESULT',case,mode,index,variant,{k:v for k,v in report.items() if k.startswith('time_')},flush=True)
        return
    e = json.loads((folder / 'engine.json').read_text())
    assert len(e['frames']) == 352
    gpu = {s['frame_id']:s for s in e['gpu']}
    phases = {'still':range(32,40),'moving':range(64,72),'cut':[72],'recovered':range(80,88)}
    row = dict(case=case,mode=mode,index=index,variant=variant,binary_sha256=sha(OUT / 'bin' / f'{variant}.exe'),
        audit_samples=len(samples),compiler_samples=0,min_available_mib=min(s['available_mbytes'] for s in samples),
        phases={phase:{metric:[st.mean((gpu[e['frames'][rep*88+i]['frame_id']]['span_ms'] if metric=='gpu' else
            e['frames'][rep*88+i]['cpu_service_ms']) for i in indices) for rep in range(4)]
            for metric in ['gpu','service']} for phase,indices in phases.items()},
        hdr={str(f['index']):f['hdr_sha256'] for f in e['frames'] if f.get('hdr_sha256')},
        png={p.name:sha(p) for p in folder.glob('frame-*.png')})
    spec = json.loads((OUT / 'scenes' / f'{case}.json').read_text())
    expected = {i for i,f in enumerate(spec['frames']) if f['capture']}
    assert set(row['hdr']) == {str(i) for i in expected | {0}}
    assert set(row['png']) == {f'frame-{i:04}.png' for i in expected}
    row.update(scene_sha256=sha(OUT / 'scenes' / f'{case}.json'),asset_sha256=sha(Path(spec['asset'])))
    save(folder / 'result.json',row)
    print('RESULT',case,mode,index,variant,row['phases'],flush=True)


def measure():
    wait_seconds = min(10800,float(os.environ.get('P_RETIME_DEADLINE_UNIX',time.time()+10800))-time.time())
    deadline = time.monotonic()+max(0,wait_seconds)
    scenes = OUT / 'scenes'
    scenes.mkdir(exist_ok=True)
    for case, source in [('lights','game-lights-unshadowed'),('quarry','quarry-daylight'),('game','game-base')]:
        timing = json.loads((SCENES / f'{source}-timing.json').read_text())
        quality = json.loads((SCENES / f'{source}-quality.json').read_text())
        assert len(timing['frames']) == 88
        for a,b in zip(timing['frames'],quality['frames']):
            assert {k:v for k,v in a.items() if k!='capture'} == {k:v for k,v in b.items() if k!='capture'}
        asset = Path(timing['asset'])
        timing['asset'] = str(asset if asset.is_absolute() else (SCENES / asset).resolve())
        timing['frames'] = [dict(f,capture=quality['frames'][i]['capture']) for _ in range(4) for i,f in enumerate(timing['frames'])]
        save(scenes / f'{case}.json',timing)
    for case,mode,prefix in [('soft-quarry','rt','a'),('soft-quarry','nonrt','a'),('quarry','rt','a'),('quarry','nonrt','a'),('lights','rt','a'),
                             ('lights','nonrt','a'),('game','nonrt','mem'),('quarry','nonrt','mem')]:
        while True:
            try:
                lease(case,mode,prefix,deadline)
                break
            except RuntimeError as error:
                if 'lease failed' not in str(error): raise
                print('REJECTED; retrying quiet admission:',error,flush=True)
                rejected = OUT / 'rejected' / f'{prefix}-{case}-{mode}-{time.time_ns()}'
                rejected.mkdir(parents=True)
                for i,side in enumerate(['before','after','after','before']):
                    path = OUT / 'runs' / case / mode / f'{i}-{prefix}-{side}'
                    assert path.resolve().is_relative_to(OUT.resolve())
                    if path.exists():
                        for attempt in range(100):
                            try:
                                path.rename(rejected / path.name)
                                break
                            except PermissionError:
                                if attempt==99: raise
                                time.sleep(.1)


def lease(case,mode,prefix,deadline):
    n8, runner = load_runner()
    label = f'{prefix}-{case}-{mode}-{int(time.time())}'
    if (OUT / 'runs' / case / mode / f'3-{prefix}-before/result.json').exists():
        return
    post(f'[p-retime request] owner=p-retime label={label}; <=15 min; >=8192 MiB and no cargo/rustc/link from ANY project; ABBA frozen binaries, no builds. {runner.utc()}')
    checked = 0
    while time.monotonic()<deadline:
        if time.monotonic()-checked >= 1:
            directive = order()
            sample = runner.resource_sample()
            checked = time.monotonic()
        if not runner.LOCK.exists() and sample['available_mbytes']>=8192 and not sample['compilers']:
            directive = order()
            sample = runner.resource_sample()
            if sample['available_mbytes']<8192 or sample['compilers']:
                continue
            break
        time.sleep(.05)
    else: raise RuntimeError('three-hour quiet window wait expired')
    print(directive,flush=True)
    original_save = runner.save
    def guarded_save(path,value):
        if path == runner.LOCK / 'owner.json':
            if order()!=directive:
                runner.LOCK.rmdir(); raise RuntimeError('root order changed before claim')
            value['root_order'] = directive
        if path.name.endswith('-preflight.json') and not value['games'] and not value['gpu_processes'] and not value['builds'] and value['available_mbytes']>=8192:
            post('p-retime retime admitted')
        original_save(path,value)
    runner.save = guarded_save
    command = [sys.executable,'-B',str(Path(__file__).resolve()),'abba',case,mode,prefix]
    sys.argv = ['n8-run.py','--label',label,'--minutes','15','--no-build-contention','--record-resources','--',*command]
    try:
        result = runner.main()
        if result: raise RuntimeError(f'lease failed {result}')
    finally:
        if n8.s1.claimed: post(f'p-retime {label} window released {runner.utc()} (no owned GPU/build job remains).')


if __name__=='__main__':
    if sys.argv[1]=='build': build()
    elif sys.argv[1]=='build-bvh': build_bvh()
    elif sys.argv[1]=='measure': measure()
    elif sys.argv[1]=='abba':
        case,mode,prefix=sys.argv[2:]
        post('[p-retime QUIET active] owner=p-retime COMPILER-QUIET; no foreign builds until p-retime window released.')
        for i,tag in enumerate(['before','after','after','before']): capture(case,mode,i,f'{prefix}-{tag}')
