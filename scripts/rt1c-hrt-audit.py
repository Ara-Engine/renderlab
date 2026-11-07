"""Audit the retained S1-S3 evidence against HRT1 without changing its metrics.

CPU only. Phase means pool squared errors by their original population counts;
defect counts retain both their pixel-frame sums and individual-frame maxima.
The registered +4-stop final-image diagnostic remains in the output.
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import time


def pooled(rows, getter):
    values = [getter(row) for row in rows]
    values = [value for value in values if value.get('n', 0)]
    count = sum(value['n'] for value in values)
    return math.sqrt(sum(value['n'] * value['rms'] ** 2 for value in values) / count) if count else None


def phase_values(rows):
    values = {'frames': len(rows)}
    for population in ('partial', 'lit', 'occluded', 'narrow', 'faint', 'silhouette', 'wide', 'wide6', 'contact', 'solid', 'contour'):
        values[population + '_rms'] = pooled(rows, lambda row: row[population])
    for name in ('contact_lost', 'leaks', 'solid_flips', 'solid_error_gt_002', 'missing_covered', 'false_shadows', 'missed_shadows'):
        values[name + '_sum'] = sum(row[name] for row in rows)
        values[name + '_max'] = max(row[name] for row in rows)
    for name in ('linear_hdr', 'display_+0_codes', 'display_+4_codes'):
        values[name + '_rms'] = pooled(rows, lambda row: row['radiance'][name])
    flicker = [row['residual_change_rms'] for row in rows if row.get('residual_change_rms') is not None]
    values['visibility_flicker_frames'] = len(flicker)
    values['visibility_flicker_mean'] = sum(flicker) / len(flicker) if flicker else None
    values['visibility_flicker_max'] = max(flicker) if flicker else None
    return values


def compare(scene, case, phase, fast, hrt):
    rows = []
    for measure, a in fast.items():
        if measure in ('frames', 'visibility_flicker_frames'):
            assert a == hrt[measure], (scene, case, phase, measure)
            continue
        b = hrt[measure]
        rows.append(dict(scene=scene, case=case, phase=phase, measure=measure,
                         interactive=a, hrt1=b,
                         passes=a <= b + 1e-12 if a is not None and b is not None else None))
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--motion-root', type=Path, default=Path('E:/renderlab-work/motion-benches'))
    parser.add_argument('--s1-root', type=Path, default=Path('E:/renderlab-work/contact-fold'))
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    start = time.perf_counter()
    comparisons, phases, identities = [], [], {}

    def read(path):
        raw = path.read_bytes()
        identities[str(path)] = hashlib.sha256(raw).hexdigest()
        return json.loads(raw)

    for scene, expected in (('S2', 1280), ('S3', 385)):
        directory = args.motion_root / scene
        all_rows = read(directory / 'per-frame.json')
        read(directory / 'metrics-registration.json')
        selected = {mode: [row for row in all_rows if row['mode'] == mode]
                    for mode in ('RL Interactive', 'UE HRT1')}
        assert all(len(rows) == expected for rows in selected.values()), scene
        for case, phase in sorted({(row['case'], row['phase']) for row in selected['RL Interactive']}):
            groups = [[row for row in selected[mode] if row['case'] == case and row['phase'] == phase]
                      for mode in ('RL Interactive', 'UE HRT1')]
            assert [row['frame'] for row in groups[0]] == [row['frame'] for row in groups[1]]
            fast, hrt = map(phase_values, groups)
            phases.append(dict(scene=scene, case=case, phase=phase, interactive=fast, hrt1=hrt))
            comparisons.extend(compare(scene, case, phase, fast, hrt))

    for case, data in read(args.s1_root / 'scores.json').items():
        groups = [[data['modes'][mode]] for mode in ('RL Interactive', 'UE HRT1')]
        fast, hrt = map(phase_values, groups)
        phases.append(dict(scene='S1', case=case, phase='static', interactive=fast, hrt1=hrt))
        comparisons.extend(compare('S1', case, 'static', fast, hrt))

    misses = [row for row in comparisons if row['passes'] is False]
    result = dict(phases=phases, comparisons=comparisons, misses=misses,
                  seconds=time.perf_counter()-start,
                  scope='Retained captures only; not a new current-source or full-frame film certificate',
                  missing='S1 motion, quarry motion quality/flicker, and matched optimized current-source cost/memory are not supplied by this audit')
    (args.output / 'audit.json').write_text(json.dumps(result, indent=2, allow_nan=False))
    (args.output / 'identities.json').write_text(json.dumps(identities, indent=2))
    for name, rows in (('comparison', comparisons), ('misses', misses)):
        with (args.output / (name + '.csv')).open('w', newline='') as stream:
            writer = csv.DictWriter(stream, fieldnames=list(comparisons[0]))
            writer.writeheader()
            writer.writerows(rows)
    print(json.dumps(dict(phases=len(phases), misses=misses, seconds=result['seconds']), indent=2))


if __name__ == '__main__':
    main()
