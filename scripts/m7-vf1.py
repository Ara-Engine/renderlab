"""VF1: guarded two-job compilation and one n8-run lease for lib tests and images.

  py -3 -B scripts/m7-vf1.py build
  py -3 -B scripts/m7-vf1.py gpu LABEL
All targets, staged binaries and evidence belong to D:/Coding/renderlab-work/m7-vf1.
"""
import argparse
import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[1]
OUT = Path('D:/Coding/renderlab-work/m7-vf1')
OWNER = 'm7-vf1'
BIN = OUT / 'bin' / os.environ.get('VF1_BIN_TAG', '')


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), REPO / 'scripts' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def schedule():
    checks = load('m7-g1-run')
    checks.UNIT = OWNER
    checks.OUT = OUT
    return checks, checks.ue()


def build():
    runner = load('m7-g2-run')
    runner.UNIT = OWNER
    runner.OUT = OUT
    target = OUT / 'target'
    OUT.mkdir(parents=True, exist_ok=True)
    for label, args in [('lib', ['test', '--lib', '--no-run']), ('example', ['build', '--example', 'game_scene'])]:
        command = ['cargo', *args, '--release', '-j', '2', '--target-dir', str(target), '--message-format=json']
        start = time.monotonic()
        code = runner.cargo(None, command, 'build-' + label)
        print(label, 'compile seconds', round(time.monotonic() - start, 2), flush=True)
        if code:
            print((OUT / ('build-' + label + '.err')).read_text(errors='replace')[-6000:])
            raise SystemExit(code)
        rows = [json.loads(l) for l in (OUT / ('build-' + label + '.out')).read_text().splitlines() if l.startswith('{')]
        executable = next(r['executable'] for r in reversed(rows) if r.get('executable'))
        BIN.mkdir(parents=True, exist_ok=True)
        shutil.copy2(executable, BIN / (label + '.exe'))
    print('staged', BIN, flush=True)


def batch(label):
    # A queued batch may pin a completed staged build before taking the lease;
    # existing executables stay immutable. Verify the pins before any test runs.
    pins = OUT / (label + '-binaries.json')
    directory = BIN
    if pins.exists():
        plan = json.loads(pins.read_text())
        directory = Path(plan['directory'])
        for name in ['lib', 'example']:
            actual = hashlib.sha256((directory / (name + '.exe')).read_bytes()).hexdigest()
            assert actual == plan[name + '_sha256'], name + ' executable pin changed'
        print('VF1 pinned binaries', directory, flush=True)
    tests = str(directory / 'lib.exe')
    example = str(directory / 'example.exe')
    root = OUT / 'images' / label
    root.mkdir(parents=True, exist_ok=True)
    manifest = 'E:/renderlab-work/m7-g1c/quarry/manifest.json'
    # Discover AT1's already-registered quarry manifest instead of duplicating assets.
    for p in Path('D:/Coding/renderlab-work/at1/plans').glob('*.json'):
        try:
            v = json.loads(p.read_text())
            if isinstance(v, list):
                for step in v:
                    manifest = step.get('env', {}).get('AT1_QUARRY', manifest)
        except (ValueError, OSError):
            pass
    def run(name, cmd, extra=None):
        env = os.environ.copy()
        env.update(extra or {})
        with (root / (name + '.log')).open('w') as log:
            result = subprocess.run(cmd, cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT)
        print(name, 'exit', result.returncode, flush=True)
        if result.returncode:
            print((root / (name + '.log')).read_text(errors='replace')[-5000:], flush=True)
            raise SystemExit(result.returncode)
    run('lib', [tests, '--test-threads=2'])
    run('reader', [tests, 'atmosphere::tests::gpu_vf1_clip_reader', '--ignored', '--exact', '--nocapture'])
    for path in ['nonrt', 'traced', 'raster']:
        output = root / path
        run('quarry-' + path, [tests, 'atmosphere::tests::gpu_at1_quarry', '--ignored', '--exact', '--nocapture'],
            dict(AT1_QUARRY=manifest, AT1_OUT=str(output), AT1_PATH=path, AT1_FRAMES='32'))
    for path in ['nonrt', 'rt']:
        for setting, flag in [('off', '--no-volumetric-fog'), ('default', ''), ('shafts', '--volumetric-fog')]:
            output = root / ('game-' + path + '-' + setting)
            run(output.name, [example, '--path', path, '--out', str(output), '--frames', '32', '--every', '16',
                '--size', '1280x720', *([flag] if flag else [])])


def gpu(label):
    checks, host = schedule()
    with host.SCHEDULE.open('a', encoding='utf-8') as stream:
        stream.write(f'\n[{OWNER} request] owner={OWNER} asks for <=20 min lease {label}: VF1 lib, quarry and game-scene images on both paths; headless, diagnostic timings. {dt.datetime.now(dt.timezone.utc).isoformat()}\n')
    deadline = time.monotonic() + 5400
    attempt = 0
    while time.monotonic() < deadline:
        if host.LOCK.exists() or checks.hold_state(host) == 'hold' or checks.quiet_active(host, any_lease=False):
            time.sleep(1)
            continue
        attempt += 1
        os.environ.update(N8_OWNER=OWNER, N8_OUT=str(OUT))
        guard = load('n8-run')
        original = guard.s1.runner.state
        def state(owned=()):
            if checks.hold_state(host) == 'hold' or checks.quiet_active(host, any_lease=False):
                raise RuntimeError('game hold or foreign QUIET window')
            return original(owned)
        guard.s1.runner.state = state
        sys.argv = ['n8-run', '--label', label + '-' + str(attempt), '--minutes', '20', '--fast-check', '--',
                    sys.executable, '-B', str(Path(__file__).resolve()), 'batch', label]
        try:
            code = guard.s1.runner.main()
        finally:
            if guard.s1.claimed:
                with host.SCHEDULE.open('a', encoding='utf-8') as stream:
                    stream.write(f'\n{OWNER} {label}-{attempt} window released {guard.s1.runner.utc()} (no owned GPU/build job remains).\n')
        if code != 3:
            raise SystemExit(code)
        time.sleep(2)
    raise SystemExit('GPU wait deadline reached; no job remains')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['build', 'gpu', 'batch'])
    parser.add_argument('label', nargs='?', default='a')
    args = parser.parse_args()
    if args.action == 'build': build()
    elif args.action == 'gpu': gpu(args.label)
    else: batch(args.label)
