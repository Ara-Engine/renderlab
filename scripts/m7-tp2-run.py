"""TP2 CPU builds and n8-leased test batches; all bulk output stays on D:."""
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[1]
OUT = Path("D:/Coding/renderlab-work/m7-tp2")

def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), REPO / 'scripts' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

def batch(label, binary, steps):
    import json
    plan = OUT / ('batch-' + label + '.json')
    rows = json.loads(plan.read_text()) if plan.exists() else [dict(binary=binary, step=step) for step in steps]
    for index, row in enumerate(rows):
        binary, step = row['binary'], row['step']
        path, section, profile = step.split(':')
        out = OUT / 'runs' / label / f'{index}-{path}-{profile or "final"}'
        out.mkdir(parents=True, exist_ok=True)
        env = dict(os.environ, RENDERLAB_TP1_PATH=path, RENDERLAB_TP1_SECTIONS=section,
                   RENDERLAB_TP1_QUARRY=str(OUT / 'quarry-manifest.json'), RENDERLAB_TP1_OUT=str(out),
                   RENDERLAB_TP2_PROFILE=profile)
        test = 'gpu_tp2_fixture' if section == 'fixture' else 'gpu_tp1_scene'
        if section == 'example':
            command = [OUT / 'bin/example.exe','--path',path,'--out',out,'--frames','48','--every','16','--size','1280x720']
        else:
            command = [binary, '--test-threads=2'] if section == 'lib' else [binary,
                'renderer::transparency_gpu::tp1_tests::' + test, '--ignored', '--exact', '--nocapture', '--test-threads=1']
        with (out / 'test.log').open('w') as log:
            result = subprocess.run(command, cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((out / 'test.log').read_text(errors='replace')[-6000:], flush=True)
            raise SystemExit(result.returncode)
        print(f'passed {step}', flush=True)

def turn_ready(g, tp1):
    lines = tp1.schedule_read_retry(lambda: g.SCHEDULE.read_text(encoding='utf-8')).splitlines()
    after = OUT / 'gpu-after.json'
    if after.exists():
        import json
        gate = json.loads(after.read_text())
        anchors = [i for i, line in enumerate(lines) if gate['after'] in line]
        if not anchors:
            return False
        releases = [line for line in lines[anchors[-1] + 1:] if 'window released' in line and not line.startswith('[')]
        if not all(any(token in line for line in releases) for token in gate['requires']):
            return False
    orders = [i for i, line in enumerate(lines) if '[root order]' in line and 'tp2-p5' in line]
    if not orders:
        return True
    later = lines[orders[-1] + 1:]
    released = [line for line in later if 'window released' in line and not line.startswith('[')]
    return (any('root-gate-b4' in line for line in released)
            and any(line.startswith('m7-lc2 ') for line in released)
            and any('r1n-g1' in line for line in released))

if __name__ == '__main__':
    command, *args = sys.argv[1:]
    OUT.mkdir(parents=True, exist_ok=True)
    if command in ('build', 'check', 'build-example'):
        tp1 = load('m7-tp1-run')
        tp1.OUT, tp1.TARGET = OUT, OUT / 'target'
        original_guard = tp1.guard
        def guard():
            g = original_guard()
            g.OWNER, g.OUT = 'm7-tp2', OUT
            return g
        tp1.guard = guard
        # Serial Cargo use per target; each invocation also gets its own log files.
        if command in ('build', 'build-example'):
            import json
            import shutil
            import hashlib
            import datetime
            label = args[0] if command == 'build' else 'example'
            started = datetime.datetime.now(datetime.timezone.utc).isoformat()
            head = subprocess.check_output(['git','rev-parse','HEAD'], cwd=REPO, text=True).strip()
            diff = subprocess.check_output(['git','diff','HEAD','--','src','shaders','examples','Cargo.toml','Cargo.lock'], cwd=REPO)
            cargo_command = ['cargo','test','-p','renderlab','--lib','--no-run'] if command == 'build' else ['cargo','build','-p','renderlab','--example','game_scene']
            code = tp1.cargo(None, [*cargo_command,'-j','2','--target-dir',str(tp1.TARGET),'--message-format=json'], 'build-' + label)
            if code:
                raise SystemExit((OUT / ('build-' + label + '.err')).read_text(errors='replace')[-4000:])
            records = [json.loads(l) for l in (OUT / ('build-' + label + '.out')).read_text().splitlines() if l.startswith('{')]
            binary = next(r['executable'] for r in reversed(records) if r.get('reason') == 'compiler-artifact' and r.get('executable'))
            (OUT / 'bin').mkdir(exist_ok=True)
            shutil.copy2(binary, OUT / 'bin' / (label + '.exe'))
            (OUT / ('build-' + label + '.json')).write_text(json.dumps(dict(
                started=started, finished=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                head=head, source_diff_sha256=hashlib.sha256(diff).hexdigest(),
                binary_sha256=hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
                command=cargo_command, profile='test/dev opt-level-1 renderlab, two Cargo jobs'), indent=2))
            print('binary', OUT / 'bin' / (label + '.exe'))
        else:
            sys.argv = [sys.argv[0], command, *args]
            tp1.main()
    elif command == 'batch':
        batch(*args[:2], args[2:])
    elif command == 'gpu':
        label, binary, *steps = args
        runner, tp1 = load('m7-g1-run'), load('m7-tp1-run')
        g = tp1.guard()
        g.OWNER = 'm7-tp2'
        # Preload n8's unchanged lease/guard. Starting a new Python interpreter
        # after seeing a free lock loses every race to the in-process queues.
        os.environ.update(N8_OWNER='m7-tp2', N8_OUT=str(OUT))
        n8 = load('n8-run')
        deadline = time.monotonic() + 45 * 60
        g.schedule_line(f'[m7-tp2 request] owner=m7-tp2 label=tp2-{label} functional headless batch {g.utc().isoformat()}')
        while True:
            if ((OUT / 'bin' / (binary + '.exe')).exists() and turn_ready(g, tp1)
                    and not tp1.schedule_read_retry(runner.quiet_active, g)
                    and tp1.schedule_read_retry(runner.hold_state, g) != 'hold' and not tp1.LOCK.exists()):
                sys.argv = [str(REPO / 'scripts/n8-run.py'), '--label', 'tp2-' + label,
                    '--minutes', '15', '--fast-check', '--', sys.executable, '-B', __file__, 'batch', label,
                    str(OUT / 'bin' / (binary + '.exe')), *steps]
                n8.s1.claimed = False
                try:
                    code = n8.s1.runner.main()
                finally:
                    if n8.s1.claimed:
                        g.schedule_line(f'm7-tp2 tp2-{label} window released {g.utc().isoformat()} (no owned GPU/build job remains).')
                if code != 3:
                    raise SystemExit(code)
            if time.monotonic() > deadline:
                g.schedule_line(f'm7-tp2 request tp2-{label} withdrawn {g.utc().isoformat()} (no job remains).')
                raise SystemExit('lease wait timed out')
            # Match the other functional queues' atomic-lock cadence.
            time.sleep(0.05)
    else:
        raise SystemExit('expected build/check/gpu/batch')
