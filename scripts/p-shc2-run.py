"""P-SHC2 CPU build / bounded n8 lease. Schedule reads are streaming and owner-exact."""
import argparse
import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[1]
OUT = Path(os.environ.get('P_SHC2_OUT','D:/Coding/renderlab-work/p-shc2'))
TARGET = Path('D:/Coding/pshc2-target')
SCHEDULE = Path('E:/renderlab-work/gpu-schedule.md')
FLAGS = subprocess.CREATE_NO_WINDOW

def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

def lines():
    with SCHEDULE.open(encoding='utf-8') as f:
        return [line.rstrip('\n') for line in f]

def order():
    value = [s for s in lines() if s.startswith('[root order]')][-1]
    return dict(line=value, sha256=hashlib.sha256(value.encode()).hexdigest())

def quiet():
    history = lines()
    for i, line in enumerate(history):
        admitted = re.match(r'^(\S+) (?:\S+ )?admitted\b',line)
        explicit = re.match(r'^\[(\S+) quiet[^\]]*\] owner=(\S+)',line)
        # Root defines this exact owner's admission signal as COMPILER-QUIET,
        # including its deliberately short "p-retime retime admitted" line.
        if not ((admitted and ('COMPILER-QUIET' in line or admitted[1]=='p-retime')) or explicit):
            continue
        owner = explicit[2] if explicit else admitted[1]
        if explicit:
            end = re.search(r'\bend=(\S+)',line)
            if end and dt.datetime.fromisoformat(end[1].rstrip(';,').replace('Z','+00:00'))<=dt.datetime.now(dt.timezone.utc):
                continue
        if not any(re.match(re.escape(owner)+r' .+ window released \d{4}-', s) for s in history[i+1:]):
            return line
    return None

def note(text):
    with SCHEDULE.open('a', encoding='utf-8') as f:
        f.write('\np-shc2 '+text+' '+dt.datetime.now(dt.timezone.utc).isoformat()+'\n')

def build():
    n8 = load(REPO/'scripts/n8-run.py', 'shc2_build_n8')
    if quiet() or n8.s1.runner.resource_sample()['available_mbytes'] < 7168:
        raise RuntimeError('build deferred: compiler quiet or <7 GiB')
    OUT.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(TARGET), CARGO_BUILD_JOBS='2')
    for label, command in [
        ('tests', ['cargo','test','--release','-p','renderlab','--lib','--no-run','-j','2']),
        ('examples', ['cargo','build','--release','-p','renderlab','--example','game_scene','--example','ara1_capture','-j','2'])]:
        note('CPU-only build '+label+' (two jobs; '+str(TARGET)+'; stops on compiler quiet)')
        start=time.monotonic()
        with (OUT/('build-'+label+'.log')).open('w') as log:
            child=subprocess.Popen(command,cwd=REPO,env=env,stdout=log,stderr=subprocess.STDOUT,creationflags=FLAGS)
            while child.poll() is None:
                time.sleep(1)
                if quiet() or n8.s1.runner.resource_sample()['available_mbytes'] < 2048:
                    subprocess.run(['taskkill','/PID',str(child.pid),'/T','/F'],capture_output=True,creationflags=FLAGS)
                    child.wait()
                    raise RuntimeError('build stopped: compiler quiet / 2 GiB runtime guard')
        print(label,child.returncode,round(time.monotonic()-start,3),flush=True)
        if child.returncode: raise RuntimeError('build failed: '+label)
        if label=='tests':
            binary=max((TARGET/'release/deps').glob('renderlab-*.exe'),key=lambda p:p.stat().st_mtime)
            shutil.copy2(binary,OUT/'tests.exe')
        else:
            for name in ('game_scene','ara1_capture'):shutil.copy2(TARGET/('release/examples/'+name+'.exe'),OUT/(name+'.exe'))
    note('CPU builds finished; no owned compiler remains.')
    manifest = dict(head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip(),
        binaries={p.name:hashlib.file_digest(p.open('rb'),'sha256').hexdigest() for p in OUT.glob('*.exe')},
        changed_files=subprocess.check_output(['git','diff','--name-only','HEAD'],cwd=REPO,text=True).splitlines())
    (OUT/'manifest.json').write_text(json.dumps(manifest,indent=2))

def batch(kind):
    lock=Path('D:/Coding/renderlab-work/gpu-slot.lock/owner.json')
    if not lock.exists() or json.loads(lock.read_text())['owner']!='p-shc2':
        raise RuntimeError('test binaries require an owned p-shc2 lease')
    if kind=='verify':
        for part in ('background','edit','startup','ara'):
            code=batch(part)
            if code:return code
        return 0
    os.environ.update(WGPU_DEBUG='0',WGPU_VALIDATION='0',VK_LOADER_LAYERS_DISABLE='VK_LAYER_KHRONOS_validation')
    os.environ.pop('VK_INSTANCE_LAYERS',None)
    for path in ('rt','nonrt'):
        out=OUT/kind/path
        out.mkdir(parents=True,exist_ok=True)
        env=dict(os.environ,RENDERLAB_ES1_PATH=path,RENDERLAB_SHC2_OUT=str(out))
        if kind=='screen':
            env.update(RENDERLAB_CC1_PROFILE='1',RENDERLAB_CC1_OUT=str(out),RENDERLAB_CC1_CASES='full,buffered-gi,buffered-engine')
            command=[str(OUT/'tests.exe'),'gpu_cc1_codegen','--ignored','--nocapture','--test-threads=1']
        elif kind=='background':
            command=[str(OUT/'tests.exe'),'gpu_shc_mixed_reload','--ignored','--nocapture','--test-threads=1']
        elif kind=='lib':
            command=[str(OUT/'tests.exe'),'--test-threads=2']
        elif kind=='edit':
            env['RENDERLAB_RELOAD_PROBE_TINT']=str(0.6+(time.time_ns()%2500000)/10000000)
            command=[str(OUT/'game_scene.exe'),'--path',path,'--out',str(out),'--frames','2','--every','1','--reload-probe','--no-gi','--no-temporal','--no-pipeline-cache']
        elif kind=='startup':
            if (out/'cache').exists():
                raise RuntimeError('startup requires a fresh evidence root; set P_SHC2_OUT and stage binaries there')
            tint=0.6+(time.time_ns()%2500000)/10000000
            (out/'tint.json').write_text(json.dumps(dict(tint=tint)))
            env.update(RENDERLAB_SHC_PROGRAM_TINT=str(tint),RENDERLAB_SHC_HDR_HASHES='1')
            for cache_state in ('empty','populated'):
                destination=out/cache_state
                command=[str(OUT/'game_scene.exe'),'--path',path,'--out',str(destination),'--frames','2','--every','1','--pipeline-cache',str(out/'cache'),'--timing-frames','32']
                with (out/(cache_state+'.log')).open('w') as log:
                    result=subprocess.run(command,cwd=REPO,env=env,stdout=log,stderr=subprocess.STDOUT,creationflags=FLAGS,timeout=240)
                if result.returncode:return result.returncode
            continue
        elif kind=='ara':
            for scene in ('game-base-timing','ara-pbr-base-timing','quarry-base-timing'):
                spec=Path('D:/Coding/renderlab-work/ara1/phase2-scenes')/(scene+'.json')
                if not spec.exists():raise RuntimeError('missing ARA1 scene '+str(spec))
                command=[str(OUT/'ara1_capture.exe'),str(spec),path,str(out/scene)]
                with (out/(scene+'.log')).open('w') as log:
                    result=subprocess.run(command,cwd=REPO,env=env,stdout=log,stderr=subprocess.STDOUT,creationflags=FLAGS,timeout=240)
                if result.returncode:return result.returncode
            continue
        else: raise ValueError(kind)
        with (out/'run.log').open('w') as log:
            result=subprocess.run(command,cwd=REPO,env=env,stdout=log,stderr=subprocess.STDOUT,creationflags=FLAGS,timeout=360)
        print(kind,path,result.returncode,flush=True)
        if result.returncode:return result.returncode
        if kind=='lib':break
    return batch('lib') if kind=='background' else 0

def gpu(args):
    state=order()
    print(json.dumps(state),flush=True)
    if args.expected!=state['sha256']:raise RuntimeError('root order changed')
    history=lines()
    anchor=max(i for i,s in enumerate(history) if s.startswith('[root order]'))
    for gate in args.after:
        owner, _, label = gate.partition(':')
        pattern = re.escape(owner)+' '+(re.escape(label) if label else '.+')+r' window released \d{4}-'
        if not any(re.match(pattern,s) for s in history[anchor+1:]):
            raise RuntimeError('waiting for exact owner/label '+gate)
    if quiet():raise RuntimeError('COMPILER-QUIET hold active')
    os.environ.update(N8_OWNER='p-shc2',N8_OUT=str(OUT))
    n8=load(REPO/'scripts/n8-run.py','shc2_n8')
    n8.s1.runner.PREFLIGHT_MEMORY_MBYTES=8192 if args.kind in ('ara','verify') else 7168
    n8.s1.runner.RUNTIME_MEMORY_MBYTES=2048
    original=n8.s1.runner.save
    def save(path,value):
        if path==n8.s1.runner.LOCK/'owner.json':
            if order()!=state or quiet():
                n8.s1.runner.LOCK.rmdir()
                raise RuntimeError('root order or compiler quiet changed before claim')
            value['root_order']=state['line']
        original(path,value)
    n8.s1.runner.save=save
    sys.argv=['n8-run.py','--label',args.label,'--minutes','12','--record-resources','--',sys.executable,'-B',str(Path(__file__).resolve()),'batch',args.kind]
    try:return n8.s1.runner.main()
    finally:
        if n8.s1.claimed:
            with SCHEDULE.open('a',encoding='utf-8') as f:
                f.write('\np-shc2 '+args.label+' window released '+dt.datetime.now(dt.timezone.utc).isoformat()+' (no owned GPU/build job remains).\n')

def main():
    p=argparse.ArgumentParser(description=__doc__)
    sub=p.add_subparsers(dest='mode',required=True)
    sub.add_parser('order');sub.add_parser('build')
    b=sub.add_parser('batch');b.add_argument('kind')
    g=sub.add_parser('gpu');g.add_argument('kind');g.add_argument('--label',required=True);g.add_argument('--expected',required=True);g.add_argument('--after',action='append',default=[])
    a=p.parse_args()
    if a.mode=='order':print(json.dumps(order(),indent=2));return 0
    if a.mode=='build':build();return 0
    if a.mode=='batch':return batch(a.kind)
    return gpu(a)

if __name__=='__main__':raise SystemExit(main())
