"""One foreground Cargo invocation after the preceding units release the queue."""
import ctypes
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess as sp
import time
import uuid

REPO = Path(__file__).resolve().parents[2]
OUT = Path('D:/Coding/renderlab-work/p-rts2')
spec = importlib.util.spec_from_file_location('retime', REPO / 'scripts/p-retime.py')
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)
_, runner = r.load_runner()
r.post('[p-rts2 request] owner=p-rts2 screen-build-2: historical S2 variant screen only; one cargo command, >=8GiB/5GB, build slot + stillkit mutex; yield until p-gather GPU release; no port; stop admission at 01:35 UTC. Previous unadmitted waiter stopped without building.')
def preceding_gpu_released():
    released = False
    with r.SCHEDULE.open(encoding='utf-8') as lines:
        for line in lines:
            if line.startswith('[root order]'): released = False
            elif line.startswith('p-gather unit complete ') or line.startswith('[p-gather handoff]'):
                released = True
    return released
deadline = '2026-11-10T01:35:00'
while True:
    assert time.strftime('%Y-%m-%dT%H:%M:%S', time.gmtime()) < deadline, 'build admission deadline'
    directive = r.order()
    assert 'p-rts2' in directive, directive
    sample = runner.resource_sample()
    if (runner.LOCK.exists() or sample['compilers'] or sample['available_mbytes'] < 8192
            or not preceding_gpu_released() or r.BUILD_SLOT.exists()):
        time.sleep(1)
        continue
    assert shutil.disk_usage('D:/').free >= 5_000_000_000
    break
kernel = ctypes.WinDLL('kernel32', use_last_error=True)
kernel.CreateMutexW.restype = ctypes.c_void_p
kernel.CreateMutexW.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_wchar_p]
kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
kernel.ReleaseMutex.argtypes = [ctypes.c_void_p]
kernel.CloseHandle.argtypes = [ctypes.c_void_p]
mutex = kernel.CreateMutexW(None, False, 'Global\\stillkit-cargo-one-at-a-time')
assert mutex
assert kernel.WaitForSingleObject(mutex, 1000) in (0, 128), 'mutex busy; retry separately'
token = uuid.uuid4().hex
claimed = False
command = ['cargo', 'test', '--release', '--locked', '-j', '2', '--lib', '--no-run', '--message-format=json']
try:
    assert not runner.resource_sample()['compilers'] and not runner.LOCK.exists()
    r.BUILD_SLOT.mkdir()
    claimed = True
    r.save(r.BUILD_SLOT/'owner.json', dict(owner='p-rts2', token=token, pid=os.getpid(), command=command))
    start = time.monotonic()
    with (OUT/'screen-build.log').open('w') as log:
        result = sp.run(command, cwd=OUT/'source', env=os.environ | {'CARGO_TARGET_DIR':str(OUT/'target'), 'CARGO_BUILD_JOBS':'2'}, stdout=log, stderr=sp.STDOUT)
    identity = dict(source='db326de', main=sp.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip(), command=command, seconds=time.monotonic()-start, exit=result.returncode, admission=sample, root_order=directive)
    if result.returncode == 0:
        rows = [json.loads(line) for line in (OUT/'screen-build.log').read_text().splitlines() if line.startswith('{')]
        binary = Path(next(row['executable'] for row in rows if row.get('executable') and row.get('profile',{}).get('test')))
        shutil.copy2(binary,OUT/'screen.exe')
        identity['sha256'] = r.sha(OUT/'screen.exe')
    r.save(OUT/'build-identity.json', identity)
finally:
    if claimed:
        assert json.loads((r.BUILD_SLOT/'owner.json').read_text())['token'] == token
        (r.BUILD_SLOT/'owner.json').unlink()
        r.BUILD_SLOT.rmdir()
    kernel.ReleaseMutex(mutex)
    kernel.CloseHandle(mutex)
    r.post(f'p-rts2 screen-build-2 window released {runner.utc()} (no owned build remains).')
raise SystemExit(result.returncode)
