"""P-VERIFY: compiler-free, bounded leases for prebuilt verification plans."""
import importlib.util
import inspect
import json
import os
from pathlib import Path
import subprocess as sp
import sys
import threading
import time

REPO = Path(__file__).resolve().parents[1]
OUT = Path('D:/Coding/renderlab-work/p-verify')
SCHEDULE = Path('E:/renderlab-work/gpu-schedule.md')
OUT.mkdir(parents=True, exist_ok=True)


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def post(line):
    for attempt in range(50):
        try:
            with SCHEDULE.open('a', encoding='utf-8') as f:
                f.write('\n' + line + '\n')
            break
        except PermissionError:
            if attempt == 49:
                raise
            time.sleep(.1)
    print(line, flush=True)


def lease(label, command):
    retime = load('retime', REPO / 'scripts/p-retime.py')
    n8, runner = retime.load_runner()
    runner.OWNER = 'p-verify'
    runner.OUT = OUT / 'lease'
    state = runner.state
    def quiet_state(owned=()):
        value = state(owned)
        value['builds'] += runner.resource_sample()['compilers']
        return value
    runner.state = quiet_state
    # Stream the complete shared schedule; retain the latest directive and tail.
    tail = []
    with SCHEDULE.open(encoding='utf-8', errors='replace') as lines:
        for line in lines:
            if line.startswith('[root order]'):
                tail = []
            tail.append(line.rstrip())
    (OUT / (label + '-schedule.json')).write_text(json.dumps(tail, indent=2))
    post(f'[p-verify request] owner=p-verify label={label}; <=15 min; >=8 GiB; no cargo/rustc; prebuilt only. {runner.utc()}')
    sample = runner.resource_sample()
    if sample['compilers'] or sample['available_mbytes'] < 8192 or Path('D:/Coding/renderlab-work/build-slot.lock').exists():
        print('DEFERRED', sample, flush=True)
        return 3
    original_save = runner.save
    def save(path, value):
        if path == runner.LOCK / 'owner.json':
            sample = runner.resource_sample()
            if sample['compilers'] or sample['available_mbytes'] < 8192:
                runner.LOCK.rmdir()
                raise RuntimeError('admission changed')
        if path.name.endswith('-preflight.json') and not value['games'] and not value['gpu_processes'] and value['available_mbytes'] >= 8192:
            post(f'p-verify {label} admitted {runner.utc()}')
        original_save(path, value)
    runner.save = save
    quiet = ['--no-build-contention'] if os.environ.get('PVERIFY_QUIET') == '1' else []
    sys.argv = ['n8-run.py', '--label', label, '--minutes', '14', *quiet, '--record-resources', '--', *command]
    try:
        return runner.main()
    finally:
        if n8.s1.claimed:
            post(f'p-verify {label} window released {runner.utc()} (no owned GPU/build job remains).')


def plan(path):
    module = load('plan', REPO / 'tools/p-nrt/plan.py')
    # Preserve capture switches (the main helper predates per-step env support).
    capture = module.capture
    def env_capture(step):
        previous = os.environ.copy()
        os.environ.update(step.get('env', {}))
        try:
            return capture(step)
        finally:
            os.environ.clear(); os.environ.update(previous)
    module.capture = env_capture
    # Trace targets are stopped by their exact executable path/PID, never /IM.
    def kill(name):
        binary = next(s['binary'] for s in json.loads(Path(path).read_text()) if Path(s.get('binary', '')).name == name)
        query = "Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq '" + binary.replace("'", "''") + "' } | Select-Object -ExpandProperty ProcessId"
        pids = sp.check_output(['powershell', '-NoProfile', '-Command', query], text=True).split()
        for pid in pids:
            sp.run(['taskkill', '/PID', pid, '/T', '/F'], creationflags=sp.CREATE_NO_WINDOW)
    module.kill = kill
    trace = module.trace
    def safe_trace(step):
        previous = os.environ.copy()
        os.environ.update(step.get('env', {}))
        try:
            return trace(step)
        finally:
            kill(Path(step['binary']).name)
            os.environ.clear(); os.environ.update(previous)
    module.trace = safe_trace
    sys.argv = ['plan.py', path]
    return module.main()


def build(unit):
    """Build an archived source snapshot; release the atomic slot immediately."""
    retime = load('retime', REPO / 'scripts/p-retime.py')
    _, runner = retime.load_runner()
    slot = Path('D:/Coding/renderlab-work/build-slot.lock')
    sample = runner.resource_sample()
    if sample['compilers'] or sample['available_mbytes'] < 8192 or runner.LOCK.exists():
        print('DEFERRED', sample, flush=True)
        return 3
    slot.mkdir()
    token = f'p-verify-{os.getpid()}'
    (slot / 'owner.json').write_text(json.dumps(dict(owner='p-verify', token=token, pid=os.getpid(), utc=runner.utc())))
    target = Path('D:/Coding/pverify-target')
    tree = OUT / f'{unit}-source'
    command = ['cargo', 'build', '--release', '-j', '2', '--locked']
    command += ['--example', 'p_nsq_screen'] if unit == 'nsq3' else ['--example', 'game_scene', '--example', 'skinned-mesh', '--tests', '--message-format=json']
    start = time.monotonic()
    try:
        sample = runner.resource_sample()
        if sample['compilers'] or sample['available_mbytes'] < 8192:
            return 3
        with (OUT / f'{unit}-build.log').open('w') as log:
            child = sp.Popen(command, cwd=tree, env=os.environ | dict(CARGO_TARGET_DIR=str(target), CARGO_BUILD_JOBS='2'), stdout=log, stderr=sp.STDOUT, creationflags=sp.CREATE_NO_WINDOW)
            stopped = threading.Event()
            resources = []
            def monitor_memory():
                while not stopped.wait(1):
                    value = runner.resource_sample()
                    resources.append(value)
                    if value['available_mbytes'] < 2048 and child.poll() is None:
                        sp.run(['taskkill', '/PID', str(child.pid), '/T', '/F'], creationflags=sp.CREATE_NO_WINDOW)
                        return
            monitor = threading.Thread(target=monitor_memory)
            monitor.start()
            try:
                return child.wait(timeout=810)
            except sp.TimeoutExpired:
                sp.run(['taskkill', '/PID', str(child.pid), '/T', '/F'], creationflags=sp.CREATE_NO_WINDOW)
                child.wait()
                return 1
            finally:
                stopped.set(); monitor.join()
                (OUT / f'{unit}-build-resources.json').write_text(json.dumps(resources))
    finally:
        (OUT / f'{unit}-build-audit.json').write_text(json.dumps(dict(command=command, seconds=time.monotonic()-start, admission=sample), indent=2))
        if json.loads((slot / 'owner.json').read_text())['token'] == token:
            (slot / 'owner.json').unlink(); slot.rmdir()


if __name__ == '__main__':
    if sys.argv[1] == 'lease':
        raise SystemExit(lease(sys.argv[2], sys.argv[3:]))
    if sys.argv[1] == 'plan':
        raise SystemExit(plan(sys.argv[2]))
    if sys.argv[1] == 'build':
        raise SystemExit(build(sys.argv[2]))
    if sys.argv[1] == 'retime':
        retime = load('retime', REPO / 'scripts/p-retime.py')
        retime.OUT = OUT / 'retime'
        for directory in ['bin', 'scenes']:
            destination = retime.OUT / directory
            destination.mkdir(parents=True, exist_ok=True)
            for source in Path('D:/Coding/renderlab-work/p-retime', directory).iterdir():
                if source.is_file() and not (destination / source.name).exists():
                    os.link(source, destination / source.name)
        source = inspect.getsource(retime.capture).replace("== 'p-retime'", "== 'p-verify'")
        exec(source, retime.__dict__)
        case, mode, prefix = sys.argv[2:]
        for index, side in enumerate(['before', 'after', 'after', 'before']):
            retime.capture(case, mode, index, prefix + '-' + side)
    if sys.argv[1] == 'wait':
        _, runner = load('retime', REPO / 'scripts/p-retime.py').load_runner()
        end = time.monotonic() + float(sys.argv[2])
        with (OUT / 'queue.jsonl').open('a') as audit:
            while True:
                sample = runner.resource_sample()
                audit.write(json.dumps(sample) + '\n'); audit.flush()
                if not sample['compilers'] and sample['available_mbytes'] >= 8192:
                    print('ADMISSIBLE', flush=True)
                    if len(sys.argv) > 3:
                        code = sp.call(sys.argv[3:], env=os.environ | {'ARA1_HDR_HASH': '1'})
                        if code == 3:
                            time.sleep(1)
                            continue
                        raise SystemExit(code)
                    break
                if time.monotonic() >= end:
                    print('WAIT EXPIRED', sample, flush=True)
                    raise SystemExit(3)
                time.sleep(1)
