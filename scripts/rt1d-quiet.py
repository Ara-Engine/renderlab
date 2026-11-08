"""RT1d final quiet lease; n8's ordinary lock, memory/game and compiler guards.

Publish the quiet interval at lock acquisition, then give already running
compilers at most 45 seconds to finish before n8's preflight. New compilers
during the measured child still trigger n8's --no-build-contention abort.
"""
import datetime
import importlib.util
import os
from pathlib import Path
import sys
import time

os.environ['N8_OWNER'] = 'rt1d-shadow-filter'
os.environ['N8_OUT'] = 'D:/Coding/renderlab-work/rt1d'
spec = importlib.util.spec_from_file_location('rt1d_guard', Path(__file__).with_name('n8-run.py'))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)
original_state, original_save = guard.s1.runner.state, guard.s1.runner.save
waiting = True
schedule = guard.s1.SCHEDULE
label = sys.argv[sys.argv.index('--label')+1]


def save(path, value):
    original_save(path, value)
    if path == guard.s1.runner.LOCK/'owner.json':
        end = datetime.datetime.fromisoformat(value['utc'])+datetime.timedelta(minutes=value['expected_minutes'])
        with schedule.open('a') as stream:
            stream.write(f'\n[rt1d QUIET active] owner=rt1d-shadow-filter label={label} start={value["utc"]} end={end.isoformat()}; no new GPU lease, compiler or heavy CPU job inside. Existing compilers drain before measurement.\n')


def state(owned=()):
    global waiting
    value = original_state(owned)
    if waiting and not owned:
        waiting = False
        start = time.monotonic()
        deadline = start+45
        clean_since = None
        samples = []
        while not value['games'] and not value['gpu_processes'] and value['available_mbytes']>=4096:
            now = time.monotonic()
            samples.append(dict(utc=value['utc'],compilers=[p['ProcessId'] for p in value['builds']]))
            clean_since = None if value['builds'] else clean_since if clean_since is not None else now
            if clean_since is not None and now-clean_since>=6:
                break
            if now>=deadline:
                break
            print('QUIET preflight: settling compiler processes', samples[-1]['compilers'], flush=True)
            time.sleep(1)
            value = original_state()
        value['quiet_preflight_wait_seconds'] = time.monotonic()-start
        value['quiet_preflight_compiler_samples'] = samples
    return value


guard.s1.runner.save, guard.s1.runner.state = save, state
code = 1
try:
    assert '--no-build-contention' in sys.argv and '--fast-check' in sys.argv
    code = guard.s1.runner.main()
finally:
    if guard.s1.claimed:
        with schedule.open('a') as stream:
            stream.write(f'\n[rt1d QUIET {"completed" if code == 0 else "stopped"}] label={label} {guard.s1.runner.utc()}; owned children settled and shared lock released.\n')
            stream.write(f'rt1d-shadow-filter {label} window released {guard.s1.runner.utc()} (no owned GPU/build job remains).\n')
raise SystemExit(code)
