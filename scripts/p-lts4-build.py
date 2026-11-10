"""One Cargo command per atomic build slot and outer stillkit mutex."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess as sp
import sys
import time
import uuid

REPO = Path(__file__).resolve().parents[1]
OUT = Path('D:/Coding/renderlab-work/p-lts4')
spec = importlib.util.spec_from_file_location('retime', REPO / 'scripts/p-retime.py')
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)
_, runner = r.load_runner()
label = sys.argv[1]
assert label.replace('-', '').isalnum()
command = ['cargo', 'build', '--release', '--locked', '-j', '2', '--example', 'ara1_capture']
if label == 'tests':
    command = ['cargo', 'test', '--release', '--locked', '-j', '2', '--lib', '--no-run', '--message-format=json']
r.post(f'[p-lts4 request] owner=p-lts4 {label}: one Cargo command, atomic build slot + stillkit mutex; >=8 GiB/5 GB; no foreign compiler')
deadline = time.monotonic() + 900
while True:
    directive = r.order()
    assert 'p-lts4' in directive, directive
    sample = runner.resource_sample()
    if (runner.LOCK.exists() or sample['compilers'] or sample['available_mbytes'] < 8192
            or not r.owner_gate('p-s4b')):
        assert time.monotonic() < deadline, 'bounded build admission expired'
        time.sleep(1)
        continue
    assert shutil.disk_usage('D:/').free >= 5_000_000_000
    try:
        r.BUILD_SLOT.mkdir()
    except FileExistsError:
        time.sleep(1)
        continue
    break
token = uuid.uuid4().hex
r.save(r.BUILD_SLOT / 'owner.json', dict(owner='p-lts4', token=token, pid=os.getpid(), command=command))
start = time.monotonic()
try:
    assert not runner.LOCK.exists() and not runner.resource_sample()['compilers']
    with (OUT / f'{label}-build.log').open('w') as log:
        result = sp.run(command, cwd=REPO, env=os.environ | {'CARGO_TARGET_DIR': str(OUT / 'target'), 'CARGO_BUILD_JOBS':'2'}, stdout=log, stderr=sp.STDOUT)
    if result.returncode == 0:
        binary = OUT / 'target/release/examples/ara1_capture.exe'
        if label == 'tests':
            rows = [json.loads(line) for line in (OUT / f'{label}-build.log').read_text().splitlines() if line.startswith('{')]
            binary = Path(next(row['executable'] for row in rows if row.get('executable') and row.get('profile', {}).get('test')))
        shutil.copy2(binary, OUT / f'{label}.exe')
        r.save(OUT / f'{label}-identity.json', dict(commit=sp.check_output(['git','rev-parse','HEAD'],text=True).strip(), sha256=r.sha(OUT/f'{label}.exe'), seconds=time.monotonic()-start, command=command, admission=sample, root_order=directive))
finally:
    assert json.loads((r.BUILD_SLOT/'owner.json').read_text())['token'] == token
    (r.BUILD_SLOT/'owner.json').unlink()
    r.BUILD_SLOT.rmdir()
    r.post(f'p-lts4 {label} window released {runner.utc()} (build ended; no owned build remains).')
raise SystemExit(result.returncode)
