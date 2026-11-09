"""Quarry-only frozen-pair ABBA; all executable runs require an owned N8 lease."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys

REPO = Path(__file__).resolve().parents[1]
OUT = Path('D:/Coding/renderlab-work/p-bvht')
SCHEDULE = Path('E:/renderlab-work/gpu-schedule.md')
SLOT = Path('D:/Coding/renderlab-work/build-slot.lock')
OUT.mkdir(parents=True, exist_ok=True)


def load():
    spec = importlib.util.spec_from_file_location('retime', REPO / 'scripts/p-retime.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def order():
    latest = None
    with SCHEDULE.open(encoding='utf-8', errors='replace') as stream:
        for line in stream:
            if line.startswith('[root order]'):
                latest = line.strip()
    return latest


def post(line):
    with SCHEDULE.open('a', encoding='utf-8') as stream:
        stream.write('\n' + line + '\n')
    print(line, flush=True)


def batch():
    owner = json.loads(Path('D:/Coding/renderlab-work/gpu-slot.lock/owner.json').read_text())
    assert owner['owner'] == 'p-bvht'
    r = load()
    r.OUT = OUT
    # Keep the original four-track parser; eight ABBA processes give 16 tracks/side.
    import inspect
    exec(inspect.getsource(r.capture).replace("== 'p-retime'", "== 'p-bvht'"), r.__dict__)
    for index, side in enumerate(['before', 'after', 'after', 'before'] * 2):
        assert order() == owner['root_order'], 'new root order; release'
        r.capture('quarry', 'nonrt', index, 'mem-' + side)
        folder = OUT / 'runs/quarry/nonrt' / f'{index}-mem-{side}'
        # Hash before deleting large raw readbacks; retain JSON and PNG witnesses.
        removed = []
        for path in folder.iterdir():
            if path.stat().st_size > 5_000_000 and path.suffix not in {'.json', '.png', '.log'}:
                removed.append(dict(name=path.name, bytes=path.stat().st_size, sha256=r.sha(path)))
                path.unlink()
        r.save(folder / 'cleanup.json', removed)


def lease():
    r = load()
    n8, runner = r.load_runner()
    runner.OWNER = 'p-bvht'
    runner.OUT = OUT / 'lease'
    directive = order()
    print(directive, flush=True)
    sample = runner.resource_sample()
    if sample['compilers'] or sample['available_mbytes'] < 8192 or shutil.disk_usage('D:/').free < 5_000_000_000:
        print('DEFERRED', sample, flush=True)
        return 3
    # The user requires the atomic slot for leases as well as builds.
    SLOT.mkdir()
    token = f'p-bvht-{os.getpid()}'
    r.save(SLOT / 'owner.json', dict(owner='p-bvht', token=token, pid=os.getpid(), utc=runner.utc()))
    original_save = runner.save
    state = runner.state
    def quiet_state(owned=()):
        value = state(owned)
        value['builds'] += runner.resource_sample()['compilers']
        return value
    runner.state = quiet_state
    def save(path, value):
        if path == runner.LOCK / 'owner.json':
            assert order() == directive, 'root order changed before admission'
            value['root_order'] = directive
        original_save(path, value)
    runner.save = save
    sys.argv = ['n8-run.py', '--label', 'quarry-abba', '--minutes', '14', '--no-build-contention', '--record-resources', '--', sys.executable, '-B', str(Path(__file__).resolve()), 'batch']
    try:
        return runner.main()
    finally:
        if json.loads((SLOT / 'owner.json').read_text())['token'] == token:
            (SLOT / 'owner.json').unlink()
            SLOT.rmdir()
        if n8.s1.claimed:
            post(f'p-bvht quarry-abba window released {runner.utc()} (no owned GPU/build job remains).')


def prepare():
    r = load()
    for directory in ['bin', 'scenes']:
        target = OUT / directory
        target.mkdir(exist_ok=True)
        for source in (r.OUT / directory).iterdir():
            if source.is_file() and not (target / source.name).exists():
                os.link(source, target / source.name)
    identities = {side: r.sha(OUT / f'bin/mem-{side}.exe') for side in ['before', 'after']}
    assert identities == dict(before='6766d9e58aea4d30a63558ceca7beda6f231f01a475f81a7bbdc13166d3663de', after='fc8e651ea9993cf26657d26e9ac8dd90e0db6e07b98f4b752ac7fa551a723b05')
    r.save(OUT / 'identities.json', identities)
    post('[p-bvht request] owner=p-bvht quarry-abba: two ABBA blocks, 16 tracks per binary; prebuilt packed-only pair; <=14min, >=8GiB/5GB, atomic slot + stillkit mutex, 2GiB runtime guard, no compilers. Prior p-nrt3b finish released 23:31:40 UTC; yielding to any new p-nrt3b request ahead of admission.')


if __name__ == '__main__':
    if sys.argv[1] == 'prepare':
        prepare()
    elif sys.argv[1] == 'batch':
        batch()
    elif sys.argv[1] == 'lease':
        raise SystemExit(lease())
