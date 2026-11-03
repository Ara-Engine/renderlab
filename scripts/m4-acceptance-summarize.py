from pathlib import Path
import argparse
import csv
import hashlib
import json
import re
import subprocess
import sys
import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'measurements/m4-acceptance'
EVIDENCE = ROOT / '.refs/m4-acceptance'


def load(path):
    return json.loads(path.read_text(encoding='utf-8-sig'))


def rows(path):
    return list(csv.DictReader(path.open()))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def delta(rs, key):
    return np.diff([int(r[key]) for r in rs], prepend=0).astype(float)


def distribution(values):
    a = np.asarray(values)
    return dict(p50=float(np.percentile(a, 50)), p99=float(np.percentile(a, 99)),
                maximum=float(a.max()), total=float(a.sum()))


def evidence(run):
    replay = run / 'replay'
    proc = load(run / 'process.json')
    limits = load(replay / 'limits.json')['runtime']
    expected_limits = load(ROOT / '.refs/m4-limits/registered-defaults.json')['runtime'].copy()
    fixture = 'closed-sphere' if 'closed-sphere' in run.name else 'material-thin'
    if run.name.startswith('capacity-'):
        candidates, offset, delay = dict(a=(1, 0, 0), b=(64, 64, 100), c=(1024, 4096, 0),
                                          d=(64, 0, 100), e=(1024, 64, 0), f=(1, 4096, 100))[run.name[-1]]
        expected_limits.update(max_candidates=candidates, max_clusters=(1 if fixture == 'closed-sphere' else 13)+offset, io_delay_ms=delay)
    elif run.name.startswith('minimum-geometry-'):
        expected_limits.update(max_candidates=64, geometry_bytes=131072 if fixture == 'closed-sphere' else 196608)
    assert limits == expected_limits, (run.name, 'settings changed', limits, expected_limits)
    known_content = {'cameras.json': '0988e3cb375a0040a433b66edd1c445fae183a179a1c37cd5e34cedf92229bd1',
                     'closed-sphere.manifest': 'e2d7f42f642958123e87651466f2fb2cefe96ed0c69902d9659c52f2746f7220',
                     'material-thin.manifest': 'df600f16cbca53054bb12661c5fca986170d90af38d82007a4c105e674da6b6d',
                     '63e6f0d39d2d0b01c1a858c3ba4a5c5704f280268ba482cd28a6fffae349da1d': '63e6f0d39d2d0b01c1a858c3ba4a5c5704f280268ba482cd28a6fffae349da1d',
                     '7c9da3f0eaa6f5f477eeb690908eba639def21948fe9aeafcad7cc4cd0e13eea': '7c9da3f0eaa6f5f477eeb690908eba639def21948fe9aeafcad7cc4cd0e13eea'}
    content = load(run / 'content.json')
    recorded_content = {Path(record['Path']).name: record['Hash'].lower() for record in content}
    assert all(recorded_content.get(name) == value for name, value in known_content.items())
    rs = rows(replay / 'frames.csv')
    prep = rows(replay / 'preparation.csv')
    assert proc['exit'] == 0 and len(rs), run
    stderr = (run / 'stderr.log').read_text()
    match = re.search(r'gpu_validation_summary errors=(\d+) missing_layer=(\d+) active_layer=(\d+)', stderr)
    assert match and match[1] == match[2] == '0', run
    active_layers = int(match[3])
    checked = '--verify-cover' in proc['args']
    if run.name.startswith('timing-'):
        assert not any(flag in proc['args'] for flag in ['--capture', '--validate', '--verify-cover', '--page-log', '--warmup'])
        features = (replay / 'run.txt').read_text()
        assert 'profile=release oracle=false shadow=false profiling=false validation=false' in features
    if checked:
        assert int(match[3]) > 0, run
    for r in prep + rs:
        assert int(r['peak']) <= limits['geometry_bytes'], run
        assert int(r['geometry']) <= limits['geometry_bytes'], run
        assert int(r['metadata']) + int(r['scratch']) <= limits['metadata_bytes'], run
        assert int(r['cpu_ready']) <= (limits['read_slots'] + 1) * 65536 + 1, run
        assert int(r['payload']) <= int(r['owned_buffers']) <= int(r['geometry']), run
        assert int(r['candidates']) <= limits['max_candidates'], run
        assert int(r['cut_clusters']) <= limits['max_clusters'], run
        assert r['failures'] == r['retries'] == '0', run
    quiet_keys = ['reads', 'uploads', 'evictions', 'selection_runs', 'failures', 'retries',
                  'dropped_ready', 'cut_clusters', 'unresolved', 'max_error']
    last = rs[-1]
    quiet = 0
    for r in reversed(rs):
        if all(r[k] == last[k] for k in quiet_keys):
            quiet += 1
        else:
            break
    assert quiet >= 301, (run.name, quiet)
    if (replay / 'pages.csv').exists():
        events = rows(replay / 'pages.csv')
        final_tick = max(int(r['tick']) for r in events)
        assert final_tick < len(rs) + len(prep) - 300, (run.name, final_tick)
    summary = (replay / 'summary.txt').read_text()
    verified = int(re.search(r'verified_covers=(\d+)', summary)[1])
    if checked:
        assert verified >= len(rs), run
    counters = {}
    for key in ['uploaded_bytes', 'inflight_retirements', 'retiring_peak', 'staging_peak',
                'cpu_io_bound_bytes', 'texture_bytes', 'pipeline_creations', 'target_creations',
                'timing_failures', 'timed_frames', 'gpu_samples', 'peak_flights']:
        match = re.search(rf'\b{key}: (\d+)', summary)
        counters[key] = int(match[1]) if match else None
    assert counters['timing_failures'] == 0, run
    wall = rows(replay / 'wall.csv')
    section_keys = ['admission_total_us', 'completion_total_us', 'resources_total_us',
                    'protection_total_us', 'stale_ready_total_us', 'make_room_total_us',
                    'upload_admission_total_us', 'requests_total_us', 'snapshot_total_us']
    top_level = ['admission_total_us', 'completion_total_us', 'resources_total_us',
                 'protection_total_us', 'stale_ready_total_us', 'requests_total_us']
    sections = sum((delta(rs, k) for k in top_level), np.zeros(len(rs))) / 1000
    timings = {k.removesuffix('_total_us') + '_ms': distribution(delta(rs, k) / 1000)
               for k in ['projection_total_us', 'selection_total_us'] + section_keys}
    timings.update({k.removesuffix('_us') + '_ms': distribution([int(r[k]) / 1000 for r in rs])
                    for k in ['prep_us', 'encode_us', 'submit_us']})
    timings['advance_accounted_lower_bound_ms'] = distribution(sections)
    timings.update({k.removesuffix('_us') + '_ms': distribution([int(r[k]) / 1000 for r in wall])
                    for k in ['advance_draw_wall_us', 'backpressure_wait_us', 'work_wall_us',
                              'pacing_requested_us', 'pacing_actual_us']})
    gpu = rows(replay / 'gpu.csv')
    if run.name.startswith('timing-'):
        assert len(gpu) == len(rs), (run.name, 'missing timed GPU samples', len(gpu), len(rs))
    timings['gpu_pass_ms'] = distribution([float(r['mesh_pass_us']) / 1000 for r in gpu])
    if run.name.startswith('timing-'):
        assert all(sections[i] <= int(wall[i]['advance_draw_wall_us']) / 1000 + .010 for i in range(len(rs))), run
    peak_keys = ['geometry', 'peak', 'metadata', 'scratch', 'payload', 'owned_buffers',
                 'cpu_ready', 'retiring', 'staging', 'cut_clusters', 'candidates']
    maximums = {k: max(int(r[k]) for r in prep + rs) for k in peak_keys}
    moving = [{k: r[k] for k in ['frame', 'cut_clusters', 'unresolved', 'max_error',
                               'candidate_debt', 'output_debt', 'residency_debt']}
              for r in rs if int(r['frame']) in (210, 269, 270)]
    worst = int(np.argmax(sections))
    return dict(frames=len(rs), limits=limits, quiet_snapshots=quiet, quiet_transitions=quiet-1,
                verified_covers=verified, validation_active=active_layers,
                maxima=maximums, max_metadata_scratch=max(int(r['metadata'])+int(r['scratch']) for r in prep+rs),
                last={k: last[k] for k in quiet_keys}, moving=moving,
                startup=rows(replay / 'startup.csv'), preparation_ticks=rows(replay / 'preparation-ticks.csv'),
                cumulative_io_us={k: int(last[k]) for k in ['io_queue_total_us', 'io_read_total_us', 'io_verify_total_us']},
                counters=counters, gpu_csv_samples=len(gpu), timings=timings, worst_advance_pose=rs[worst]['frame'],
                slow_loop_rows=[r for r in wall if int(r['work_wall_us']) > 20000],
                process=proc, worst_advance_sections_ms={k: float(delta(rs, k)[worst]/1000) for k in section_keys})


def summarize():
    results = {}
    for run in sorted(OUT.iterdir()):
        if not (run / 'process.json').exists():
            continue
        if run.name.startswith('below-'):
            p = load(run / 'process.json')
            err = (run / 'stderr.log').read_text()
            expected = ('geometry capacity below pinned minimum plus one upload staging page'
                        if 'geometry' in run.name else 'output')
            if run.name == 'below-output-closed-sphere':
                expected = 'max_clusters must be positive and finite'
            assert p['exit'] == 1 and expected in err.lower(), run
            assert not (run / 'replay/frames.csv').exists(), run
            results[run.name] = dict(expected_rejection=True, exit=1, error=err.strip())
        else:
            results[run.name] = evidence(run)
        print(run.name, 'verified', flush=True)
    (EVIDENCE / 'run-summary.json').write_text(json.dumps(results, indent=2))
    timing = {}
    for fixture in ['material-thin', 'closed-sphere']:
        cases = [r for name, r in results.items() if name.startswith('timing-'+fixture)]
        if not cases:
            continue
        assert len(cases) >= 3
        timing[fixture] = {}
        for key in cases[0]['timings']:
            timing[fixture][key] = {stat: dict(median=float(np.median([r['timings'][key][stat] for r in cases])),
                                              range=[min(r['timings'][key][stat] for r in cases), max(r['timings'][key][stat] for r in cases)])
                                   for stat in ['p50', 'p99', 'maximum', 'total']}
    (EVIDENCE / 'timing-spread.json').write_text(json.dumps(timing, indent=2))


def images():
    original = ROOT / '.refs/m4-limits/images.py'
    source = original.read_text()
    substitutions = {"root = Path('measurements/m4-limits')": "root = Path('measurements/m4-acceptance')",
                     "Path('.refs/m4-limits/image-metrics.json')": "Path('.refs/m4-acceptance/image-metrics.json')"}
    for old, new in substitutions.items():
        assert source.count(old) == 1
        source = source.replace(old, new)
    adapted = EVIDENCE / 'scorer.py'
    if adapted.exists():
        assert adapted.read_text() == source
    else:
        adapted.write_text(source)
    (EVIDENCE / 'scorer-identity.json').write_text(json.dumps(dict(original=str(original), original_sha256=digest(original),
                                                                  adapted_sha256=digest(adapted), path_substitutions=substitutions), indent=2))
    subprocess.run([sys.executable, str(adapted)], cwd=ROOT, check=True)
    metrics = load(EVIDENCE / 'image-metrics.json')
    registered_references = {m['reference'].replace('\\', '/'): m['reference_sha256']
                             for m in load(ROOT / '.refs/m4-limits/image-metrics.json')}
    for m in metrics:
        assert digest(ROOT / m['candidate']) == m['candidate_sha256']
        assert digest(ROOT / m['reference']) == m['reference_sha256']
        assert registered_references[m['reference'].replace('\\', '/')] == m['reference_sha256']
    normal = [m for m in metrics if 'equal-' in m['candidate']]
    forced = [m for m in metrics if 'capacity-' in m['candidate']]
    pairs = []
    for fixture in ['material-thin', 'closed-sphere']:
        for pose in [0, 210, 269, 270, 630]:
            a = np.fromfile(OUT / f'equal-{fixture}-stream/replay/frame-{pose:04}.rgba8', dtype=np.uint8).reshape(-1, 4)
            b = np.fromfile(OUT / f'equal-{fixture}-simple/replay/frame-{pose:04}.rgba8', dtype=np.uint8).reshape(-1, 4)
            pairs.append(dict(fixture=fixture, pose=pose, differing_pixels=int(np.count_nonzero(np.any(a != b, axis=1)))))
    (EVIDENCE / 'quality-summary.json').write_text(json.dumps(dict(normal=normal, forced_capacity=forced,
                    normal_pass=sum(m['pass'] for m in normal), normal_count=len(normal),
                    forced_pass=sum(m['pass'] for m in forced), forced_count=len(forced), stream_simple_pairs=pairs), indent=2))
    for fixture in ['material-thin', 'closed-sphere']:
        names = ['full reference', 'equal stream', 'equal simple', 'minimum cap1', 'minimum cap64 delay100']
        paths = [ROOT / 'measurements/m4-stream/c1b' / ('final-full-v2-'+fixture) / 'replay',
                 OUT / ('equal-'+fixture+'-stream/replay'), OUT / ('equal-'+fixture+'-simple/replay'),
                 OUT / ('capacity-'+fixture+'-a/replay'), OUT / ('capacity-'+fixture+'-d/replay')]
        sheet = Image.new('RGB', (5*384, 5*240), '#202630')
        draw = ImageDraw.Draw(sheet)
        for y, (name, directory) in enumerate(zip(names, paths)):
            for x, pose in enumerate([0, 210, 269, 270, 630]):
                raw = directory / f'frame-{pose:04}.rgba8'
                if not raw.exists():
                    continue
                a = np.fromfile(raw, dtype=np.uint8).reshape(1080, 1920, 4)
                im = Image.fromarray(a).convert('RGB').resize((384, 216))
                sheet.paste(im, (x*384, y*240+24))
                draw.text((x*384+5, y*240+5), f'{name} {pose}', fill='white')
        sheet.save(EVIDENCE / f'contact-{fixture}.png')
    assert len(normal) == 20 and all(m['pass'] for m in normal), 'Normal quality failures retained; report the failure.'


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('phase', choices=['summary', 'images'])
    args = parser.parse_args()
    if args.phase == 'summary':
        summarize()
    else:
        images()
