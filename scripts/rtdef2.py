"""RTDEF2 paired captures. GPU/test commands always use n8's shared, guarded lease.

prepare/build are CPU-only; run waits for its own lease and blocks until it finishes.
All new evidence and the two-job Cargo target belong on D:.
"""
import argparse
import datetime
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path('D:/Coding/renderlab-work/rtdef2')
# Reuse this checkout's initial RTDEF2 compile; this unit owns and cleans the target.
TARGET = Path('D:/Coding/renderlab-work/rtdef/target')
REPO = Path(__file__).resolve().parents[1]
OWNER = 'rtdef2'
TR1 = 'renderer::fresh_shadow_tests::primary_ray_gpu_tests::visibility_buffer_tests::temporal_reconstruction_tests::gpu_tr1_sequence'


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), REPO / 'scripts' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def save(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, allow_nan=False) + '\n', encoding='utf-8')


def schedule():
    g = load('m7-g1-run')
    u = g.ue()
    u.OWNER = OWNER
    u.OUT = ROOT
    return g, u


def prepare():
    base = json.loads(Path('D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry-manifest.json').read_text())
    cameras = json.loads(Path(base['cameras']).read_text())['frames']
    variants = {'raster-exact': {'enabled': False, 'primary': 'raster-exact'}}
    variants.update({f'tr1-{n}': {'primary': 'traced-interactive', 'active_rays': n}
                     for n in (8, 16, 32, 64, 128, 256)})
    steps = []
    for pose, camera in enumerate(cameras):
        frames = []
        for f in range(64):
            # A 32-frame still warmup, 16 outward and 16 return frames, ending at the
            # registered pose. Both endpoint film references are reused read-only.
            dx = 0 if f < 32 else (f - 31 if f < 48 else 63 - f) * .001
            c = dict(camera, frame=f, phase='still' if f < 32 else 'out' if f < 48 else 'return')
            c['eye'] = [camera['eye'][0] + dx, *camera['eye'][1:]]
            c['target'] = [camera['target'][0] + dx, *camera['target'][1:]]
            frames.append(c)
        seq = ROOT / f'quarry-p{pose}-sequence.json'
        save(seq, dict(frames=frames))
        manifest = dict(base, path='rt', output=str(ROOT / f'quarry-p{pose}'), sequence=str(seq),
                        variants=variants, frames=[0, 7, 15, 31, 32, 39, 47, 48, 55, 63],
                        replay=False, visits=1, validate=False)
        path = ROOT / f'quarry-p{pose}.json'
        save(path, manifest)
        steps.append(dict(name=f'quarry-p{pose}', test=TR1, manifest=str(path)))
    scene = 'E:/renderlab-work/motion-benches/S2/fixture/scene.json'
    save(ROOT / 's2.json', dict(scene=scene, output=str(ROOT / 'S2'),
                              modes=['Raster-Exact', 'TR1', 'TR1-16', 'TR1-32', 'TR1-64', 'TR1-128', 'TR1-256'],
                              cases=['camera-0.25px-r0.04', 'shadow-step-r0.04'],
                              capture_frames=[0, 1, 2, 7, 15, 16, 19, 23, 31, 32, 63, 64, 95, 96, 127, 159],
                              no_full_captures=True, repeats=0, skip_reference=True))
    steps.append(dict(name='s2', test='gpu_motion_bench_batch', manifest=str(ROOT / 's2.json')))
    timing_frames = [dict(camera, frame=f + pose * 32)
                     for pose, camera in enumerate(cameras) for f in range(32)]
    save(ROOT / 'timing-sequence.json', dict(frames=timing_frames))
    save(ROOT / 'timing.json', dict(base, path='rt', output=str(ROOT / 'timing'),
                                   sequence=str(ROOT / 'timing-sequence.json'), variants=variants,
                                   frames=[], replay=False, visits=8, validate=False))
    save(ROOT / 'steps.json', steps)
    print(ROOT / 'steps.json', flush=True)


def build():
    g, _ = schedule()
    g.OUT = ROOT
    g.TARGET = TARGET
    g.BINARY = ROOT / 'bin/rtdef2-tests.exe'
    ROOT.mkdir(parents=True, exist_ok=True)
    g.build(argparse.Namespace(release=True, beside_leases=True, name='rtdef2-tests'))


def run_child(binary, test, manifest=None):
    owner = json.loads(Path('D:/Coding/renderlab-work/gpu-slot.lock/owner.json').read_text())
    assert owner['owner'] == OWNER, owner
    g, u = schedule()
    assert g.hold_state(u) != 'hold', 'game hold'
    assert not g.quiet_active(u, any_lease=False), 'foreign QUIET window'
    env = dict(os.environ)
    if manifest:
        env['RENDERLAB_PRIMARY_MANIFEST'] = str(manifest)
    cmd = [str(binary)]
    if test:
        cmd += [test, '--ignored']
    cmd += ['--nocapture', '--test-threads=1']
    subprocess.run(cmd, cwd=REPO, env=env, stdout=sys.stdout, stderr=subprocess.STDOUT,
                   creationflags=subprocess.CREATE_NO_WINDOW, check=True)


def batch(args):
    binary = ROOT / 'bin/rtdef2-tests.exe'
    if args.only == 'pilot':
        run_child(binary, None)
        run_child(binary, 'gpu_motion_bench_batch', ROOT / 's2.json')
        return
    if args.only == 'lib':
        run_child(binary, None)
        return
    if args.only == 'timing':
        run_child(binary, TR1, ROOT / 'timing.json')
        return
    for step in json.loads((ROOT / 'steps.json').read_text()):
        if args.only and step['name'] != args.only:
            continue
        print('RTDEF2', step['name'], flush=True)
        run_child(binary, step['test'], step['manifest'])


def run(args):
    g, u = schedule()
    label = args.only or 'quality'
    with u.SCHEDULE.open('a', encoding='utf-8') as f:
        f.write(f'\n[rtdef2 request] owner=rtdef2 asks for <=25 min {label}: paired RT quarry/S2 captures and tests; headless, functional timings. {datetime.datetime.now(datetime.timezone.utc).isoformat()}\n')
    deadline = time.monotonic() + 3600
    attempt = 0
    while time.monotonic() < deadline:
        if Path('D:/Coding/renderlab-work/gpu-slot.lock').exists():
            time.sleep(.5)
            continue
        if g.hold_state(u) == 'hold' or g.quiet_active(u, any_lease=False):
            print('DEFERRED: game hold or QUIET', flush=True)
            time.sleep(10)
            continue
        attempt += 1
        os.environ.update(N8_OWNER=OWNER, N8_OUT=str(ROOT))
        guard = load('n8-run')
        original = guard.s1.runner.state
        quiet = args.only == 'timing'
        announced = False

        def state(owned=()):
            nonlocal announced
            if g.hold_state(u) == 'hold' or g.quiet_active(u, any_lease=False):
                raise RuntimeError('game hold or foreign QUIET window')
            if quiet and not announced:
                announced = True
                end = datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(minutes=3)
                with u.SCHEDULE.open('a', encoding='utf-8') as f:
                    f.write(f'\n[rtdef2 QUIET active] owner=rtdef2 label=timing end={end.isoformat()}; no foreign GPU/compiler/heavy CPU until release.\n')
                # Announce under the acquired lock, then let existing compilers drain.
                settle = time.monotonic() + 50
                while True:
                    value = original(owned)
                    if not value['builds'] or time.monotonic() >= settle:
                        return value
                    time.sleep(1)
            return original(owned)

        guard.s1.runner.state = state
        sys.argv = ['n8-run.py', '--label', f'{label}-{int(time.time())}-{attempt}',
                    '--minutes', '25', '--fast-check', '--', sys.executable, '-B', str(Path(__file__).resolve()),
                    'batch'] + (['--only', args.only] if args.only else [])
        if quiet:
            sys.argv.insert(1, '--no-build-contention')
        try:
            code = guard.s1.runner.main()
        finally:
            if guard.s1.claimed:
                with u.SCHEDULE.open('a', encoding='utf-8') as f:
                    f.write(f'\nrtdef2 {sys.argv[sys.argv.index("--label")+1]} window released {guard.s1.runner.utc()} (no owned GPU/build job remains).\n')
        if code != 3:
            raise SystemExit(code)
        print('RTDEF2 waiting for its GPU lease', flush=True)
        time.sleep(.5)
    raise SystemExit('RTDEF2 resource queue exceeded one hour')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('action', choices=('prepare', 'build', 'run', 'batch'))
    p.add_argument('--only')
    a = p.parse_args()
    if a.action == 'prepare':
        prepare()
    elif a.action == 'build':
        build()
    elif a.action == 'batch':
        batch(a)
    else:
        run(a)


if __name__ == '__main__':
    main()
