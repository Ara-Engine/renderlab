"""Short RT1c quality-only lease during Islam's explicit relaxed game hold.

Keeps the shared lock, hidden-process launcher, 4/2 GiB memory guard, foreign
renderer checks and release record. Only the named CS2 process is authorized;
all timings from this launcher are contended and cannot certify performance.
"""
import importlib.util
import os
from pathlib import Path
import sys


def main():
    os.environ['N8_OWNER'] = 'rt1c'
    os.environ['N8_OUT'] = 'E:/renderlab-work/rt1c'
    spec = importlib.util.spec_from_file_location('rt1c_guard', Path(__file__).with_name('n8-run.py'))
    guard = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(guard)
    original = guard.s1.runner.state

    def relaxed_hold():
        directives = [line for line in guard.s1.SCHEDULE.read_text(encoding='utf-8').splitlines()
                      if line.startswith(('[GAME HOLD]', '[GAME HOLD RELAXED]', '[GAME HOLD RELEASED]'))]
        return bool(directives and directives[-1].startswith('[GAME HOLD RELAXED]'))

    if not relaxed_hold():
        raise SystemExit('This quality-only launcher requires an explicit [GAME HOLD RELAXED] directive')
    if '--minutes' not in sys.argv or not 0 < float(sys.argv[sys.argv.index('--minutes')+1]) <= 2:
        raise SystemExit('RT1c quality leases must be at most two minutes')
    if '--fast-check' not in sys.argv or '--' not in sys.argv:
        raise SystemExit('The 4/2 GiB guard and explicit test command are required')
    command = sys.argv[sys.argv.index('--')+1:]
    if len(command) != 5 or command[1:] != ['gpu_motion_bench_batch', '--ignored', '--nocapture', '--test-threads=1']:
        raise SystemExit('Only the prepared RT1c motion witness is authorized by this launcher')

    def state(owned=()):
        value = original(owned)
        # Re-read the directive on each conflict sample, so a renewed hard hold
        # restores the ordinary game rejection while this job is running.
        relaxed = relaxed_hold()
        permitted = [process for process in value['games'] if relaxed
                     and process['Name'].lower() == 'cs2.exe'
                     and (process.get('ExecutablePath') or '').lower() ==
                     'e:\\steamlibrary\\steamapps\\common\\counter-strike global offensive\\game\\bin\\win64\\cs2.exe']
        value['authorized_game_during_quality'] = permitted
        value['games'] = [process for process in value['games'] if process not in permitted]
        value['quality_only_contended'] = True
        return value

    guard.s1.runner.state = state
    if '--contended-screen' not in sys.argv:
        sys.argv.insert(1, '--contended-screen')
    try:
        return guard.s1.runner.main()
    finally:
        if guard.s1.claimed:
            label = sys.argv[sys.argv.index('--label')+1]
            with guard.s1.SCHEDULE.open('a', encoding='utf-8') as stream:
                stream.write(f'\nrt1c {label} window released {guard.s1.runner.utc()} (no owned GPU/build job remains; quality only, timings contended).\n')


if __name__ == '__main__':
    raise SystemExit(main())
