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
    for index, step in enumerate(steps):
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
            label = args[0] if command == 'build' else 'example'
            cargo_command = ['cargo','test','-p','renderlab','--lib','--no-run'] if command == 'build' else ['cargo','build','-p','renderlab','--example','game_scene']
            code = tp1.cargo(None, [*cargo_command,'-j','2','--target-dir',str(tp1.TARGET),'--message-format=json'], 'build-' + label)
            if code:
                raise SystemExit((OUT / ('build-' + label + '.err')).read_text(errors='replace')[-4000:])
            records = [json.loads(l) for l in (OUT / ('build-' + label + '.out')).read_text().splitlines() if l.startswith('{')]
            binary = next(r['executable'] for r in reversed(records) if r.get('reason') == 'compiler-artifact' and r.get('executable'))
            (OUT / 'bin').mkdir(exist_ok=True)
            shutil.copy2(binary, OUT / 'bin' / (label + '.exe'))
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
        deadline = time.monotonic() + 45 * 60
        g.schedule_line(f'[m7-tp2 request] owner=m7-tp2 label=tp2-{label} functional headless batch {g.utc().isoformat()}')
        while True:
            if not runner.quiet_active(g) and runner.hold_state(g) != 'hold' and not tp1.LOCK.exists():
                env = dict(os.environ, N8_OWNER='m7-tp2', N8_OUT=str(OUT))
                result = subprocess.run([sys.executable, '-B', REPO / 'scripts/n8-run.py', '--label', 'tp2-' + label,
                    '--minutes', '30', '--fast-check', '--', sys.executable, '-B', __file__, 'batch', label,
                    str(OUT / 'bin' / (binary + '.exe')), *steps], cwd=REPO, env=env)
                if result.returncode != 3:
                    raise SystemExit(result.returncode)
            if time.monotonic() > deadline:
                g.schedule_line(f'm7-tp2 request tp2-{label} withdrawn {g.utc().isoformat()} (no job remains).')
                raise SystemExit('lease wait timed out')
            time.sleep(3)
    else:
        raise SystemExit('expected build/check/gpu/batch')
