"""CPU-only RTDEF2 paired film scoring. Uses RTDEF's unchanged radiance scorer.

The small screen captures registered cold/moving/hold checkpoints, not all phases.
References and old HRT results remain read-only. No new reference campaign.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import time

os.environ.update(OPENBLAS_NUM_THREADS='1', OMP_NUM_THREADS='1')
import numpy as np

ROOT = Path('D:/Coding/renderlab-work/rtdef2')


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


base = load('rtdef-score')


def score_s2(directory, case_filter=None):
    old = json.loads(Path('E:/renderlab-work/motion-benches/S2/per-frame.json').read_text())
    hrt = {(r['case'], r['frame']): r for r in old if r['mode'] == 'UE HRT1'}
    rows = []
    for case in sorted(directory.glob('*-r*')):
        if case_filter and case.name != case_filter:
            continue
        modes = sorted(p for p in case.iterdir() if p.is_dir() and (p / '0000.rgba16f').exists())
        if not modes:
            continue
        frames = sorted(int(p.stem) for p in modes[0].glob('[0-9][0-9][0-9][0-9].rgba16f'))
        for frame in frames:
            ref_path = Path('D:/Coding/renderlab-work/rt1d/motion/S2/renderlab') / case.name / 'reference' / f'pose-{frame}.npz'
            with np.load(ref_path) as d:
                reference = d['hdr'].astype('f4')
                partial = (d['n'] == 256) & (d['hit'][..., 0] == 1) & (d['ref'] > .1) & (d['ref'] < .9)
            ref_display = np.asarray(base.s1.film.tone(reference, 4), dtype=float)
            panels = [('Film +4', base.s1.film.tone(reference, 4))]
            for mode in modes:
                value = base.hdr(mode / f'{frame:04}.rgba16f', (288, 512))
                info = json.loads((mode / f'{frame:04}.json').read_text())
                display = np.asarray(base.s1.film.tone(value, 4), dtype=float)
                grain = base.s1.metrics.summary((display - ref_display)[partial].ravel())
                rows.append(dict(case=case.name, frame=frame, mode=mode.name, phase=hrt[(case.name, frame)]['phase'],
                                 rays=info.get('shadow_rays'), pixels=int(partial.sum()), grain_plus4=grain,
                                 radiance=base.s1.radiance(value, reference), hrt_radiance=hrt[(case.name, frame)]['radiance']))
                label = mode.name if info.get("shadow_rays") is None else f'{mode.name}: {info["shadow_rays"]} rays'
                panels.append((label, base.s1.film.tone(value, 4)))
            if frame in (0, 7, 31, 63, 95, 127, 159):
                base.image_sheet(panels, case / f'plus4-f{frame:03}.png')
        print('Scored', case.name, len(frames), 'checkpoints x', len(modes), 'variants', flush=True)
    base.save(directory / 'scores.json', rows)
    for case in sorted({r['case'] for r in rows}):
        for frame in (0, 31, 63, 95):
            print(case, frame, ' | '.join(f"{r['mode']} n={r['rays']} whole={r['radiance']['display_+4_codes']['rms']:.3f} grain={r['grain_plus4'].get('rms',0):.3f}"
                                        for r in rows if r['case'] == case and r['frame'] == frame), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', nargs='?', type=Path, default=ROOT / 'S2')
    parser.add_argument('--case')
    args = parser.parse_args()
    start = time.monotonic()
    score_s2(args.directory, args.case)
    print('Scoring seconds', time.monotonic() - start)
