"""CPU-only RTDEF radiance scoring with the existing S1 scorer and retained films.

S2 reports every frame and phase against its registered film and HRT1 rows. Quarry
scores only the registered still poses; the short motion loop is inspected, not given
a fabricated moving reference. No Unreal or reference renderer runs here.
"""
import csv
import importlib.util
import json
import os
from pathlib import Path
import time

os.environ.update(OPENBLAS_NUM_THREADS='1', OMP_NUM_THREADS='1')
import numpy as np
from PIL import Image, ImageDraw

ROOT = Path('D:/Coding/renderlab-work/rtdef')


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


s1 = load('bench-contact-fold-score')


def save(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + '\n', encoding='utf-8')


def image_sheet(panels, path, width=512):
    height = round(panels[0][1].height * width / panels[0][1].width)
    columns = min(3, len(panels))
    sheet = Image.new('RGB', (columns * width, ((len(panels) + columns - 1) // columns) * (height + 24)), (20, 27, 35))
    draw = ImageDraw.Draw(sheet)
    for i, (label, im) in enumerate(panels):
        x, y = i % columns * width, i // columns * (height + 24)
        draw.text((x + 4, y + 4), label, fill='white')
        sheet.paste(im.resize((width, height)), (x, y + 24))
    sheet.save(path)


def hdr(path, shape):
    return np.fromfile(path, '<f2').reshape(*shape, 4).astype('f4')


def quarry():
    rt1 = load('rt1-film-replay')
    rows = []
    for pose in (0, 1):
        root = ROOT / f'quarry-p{pose}'
        ref = Path(f'E:/renderlab-work/m6-exact-quality/r1/quarry-viewed-p{pose}/pose-{pose}.film.bin')
        film = np.fromfile(ref, rt1.FILM).reshape(*rt1.SHAPE)
        reference = film['rest'][..., 4:7].astype('f4') / 256
        for frame in (0, 7, 15, 31, 63):
            panels = []
            for name in ('raster-exact', 'traced', 'tr1'):
                value = hdr(root / name / f'f{frame:03}.rgba16f', rt1.SHAPE)
                scores = s1.radiance(value, reference)
                rows.append(dict(pose=pose, frame=frame, mode=name, radiance=scores))
                panels.append((name, s1.film.tone(value)))
            if frame in (0, 31, 63):
                image_sheet(panels, root / f'comparison-f{frame:03}.png', 960)
        # Motion checkpoints: complete images plus native-pixel crops around the silhouettes.
        for frame in (32, 39, 47, 48, 55):
            panels = [(name, s1.film.tone(hdr(root / name / f'f{frame:03}.rgba16f', rt1.SHAPE)))
                      for name in ('raster-exact', 'traced', 'tr1')]
            image_sheet(panels, root / f'comparison-f{frame:03}.png', 960)
        for frame in (31, 39, 47, 55, 63):
            panels = [(name, s1.film.tone(hdr(root / name / f'f{frame:03}.rgba16f', rt1.SHAPE)).crop((704, 288, 1216, 800)))
                      for name in ('raster-exact', 'traced', 'tr1')]
            image_sheet(panels, root / f'crop-f{frame:03}.png')
        for name in ('raster-exact', 'traced', 'tr1'):
            value = hdr(root / name / 'f031.rgba16f', rt1.SHAPE)
            s1.film.tone(value).save(root / f'{name}-still.png')
        print('quarry', pose, flush=True)
    save(ROOT / 'quarry-scores.json', rows)


def s2():
    scene_path = Path('E:/renderlab-work/motion-benches/S2/fixture/scene.json')
    scene = json.loads(scene_path.read_text())
    retained = json.loads(Path('E:/renderlab-work/motion-benches/S2/per-frame.json').read_text())
    hrt = {(r['case'], r['frame']): r for r in retained if r['mode'] == 'UE HRT1'}
    x0, y0, x1, y1 = scene['crop']
    shape = (y1 - y0, x1 - x0)
    rows = []
    for root in sorted((ROOT / 'S2').glob('*-r*')):
        track = next(t for t in scene['tracks'] if root.name.startswith(t['name'] + '-r'))
        for p in track['frames']:
            frame = p['frame']
            reference_path = Path('D:/Coding/renderlab-work/rt1d/motion/S2/renderlab') / root.name / 'reference' / f'pose-{frame}.npz'
            with np.load(reference_path) as d:
                reference = d['hdr'].astype('f4')
            phase = hrt[(root.name, frame)]['phase']
            panels = [('Film', s1.film.tone(reference, 4))]
            for name in ('Raster-Exact', 'Interactive', 'TR1'):
                value = hdr(root / name / f'{frame:04}.rgba16f', shape)
                rows.append(dict(case=root.name, frame=frame, phase=phase, mode=name,
                                 radiance=s1.radiance(value, reference)))
                if frame in (0, 15, 31, 32, 63, 95, 127, 159):
                    panels.append((name + ' +4 stops', s1.film.tone(value, 4)))
            rows.append(dict(case=root.name, frame=frame, phase=phase, mode='HRT1', radiance=hrt[(root.name, frame)]['radiance']))
            if frame in (0, 15, 31, 32, 63, 95, 127, 159):
                image_sheet(panels, root / f'plus4-f{frame:03}.png')
        for name in ('Raster-Exact', 'Interactive', 'TR1'):
            for path in (root / name).glob('*-full.rgba16f'):
                value = hdr(path, (1080, 1920))
                if path.stem[:4] in ('0000', '0031', '0063', '0095', '0159'):
                    s1.film.tone(value).save(path.with_suffix('.png'))
        print('S2', root.name, flush=True)
    save(ROOT / 's2-per-frame.json', rows)
    groups = {}
    for row in rows:
        key = (row['case'], row['phase'], row['mode'])
        groups.setdefault(key, []).append(row)
    summary = []
    for (case, phase, mode), group in groups.items():
        summary.append(dict(case=case, phase=phase, mode=mode, frames=len(group),
                            **{k: float(np.sqrt(np.mean([r['radiance'][k]['rms'] ** 2 for r in group])))
                               for k in ('linear_hdr', 'display_+0_codes', 'display_+4_codes')}))
    save(ROOT / 's2-summary.json', summary)
    with (ROOT / 's2-summary.csv').open('w', newline='') as stream:
        writer = csv.DictWriter(stream, fieldnames=list(summary[0]))
        writer.writeheader()
        writer.writerows(summary)


def witness():
    """Diagnose grain on one registered surface; never replace the complete-crop score."""
    rows = []
    for case in ('camera-0.25px-r0.04', 'camera-1px-r0.04', 'shadow-step-r0.04'):
        root = ROOT / 'S2' / case
        for frame in (0, 31, 63, 95, 127, 159):
            path = Path('D:/Coding/renderlab-work/rt1d/motion/S2/renderlab') / case / 'reference' / f'pose-{frame}.npz'
            with np.load(path) as d:
                reference = d['hdr'].astype('f4')
                # Original estimator's all-256-on-centre-placement/depth population,
                # restricted to a partially shadowed floor receiver, away from silhouettes.
                use = (d['n'] == 256) & (d['hit'][..., 0] == 1) & (d['ref'] > .1) & (d['ref'] < .9)
            ref_display = np.asarray(s1.film.tone(reference, 4), dtype=float)
            panels = [('Film +4', s1.film.tone(reference, 4))]
            for name in ('Raster-Exact', 'Interactive', 'TR1'):
                value = hdr(root / name / f'{frame:04}.rgba16f', (288, 512))
                display = np.asarray(s1.film.tone(value, 4), dtype=float)
                rows.append(dict(case=case, frame=frame, mode=name, pixels=int(use.sum()),
                                 grain_display_plus4=s1.metrics.summary((display - ref_display)[use].ravel())))
                panels.append((name + ' +4', Image.fromarray(display.astype('u1'))))
            image_sheet(panels, root / f'plus4-f{frame:03}.png')
    save(ROOT / 's2-grain-diagnostic.json', rows)


def comparison():
    rows = json.loads((ROOT / 's2-per-frame.json').read_text())
    index = {(r['case'], r['frame'], r['mode']): r for r in rows}
    results = {}
    for metric in ('linear_hdr', 'display_+0_codes', 'display_+4_codes'):
        misses = {'HRT1': [], 'Raster-Exact': []}
        for (case, frame, mode), row in index.items():
            if mode != 'TR1':
                continue
            candidate = row['radiance'][metric]['rms']
            for comparator in misses:
                baseline = index[(case, frame, comparator)]['radiance'][metric]['rms']
                if candidate > baseline:
                    misses[comparator].append(dict(case=case, frame=frame, candidate=candidate, baseline=baseline))
        results[metric] = dict(frames=1280, misses=misses)
    save(ROOT / 's2-comparison.json', results)


def timing():
    data = json.loads((ROOT / 'timing/tr1.json').read_text())
    configurations = {}
    for name, variant in data['variants'].items():
        visits = [[r for f, r in enumerate(visit) if f % 32 >= 16] for visit in variant['visits'][1:]]
        assert len(visits) == 8 and all(len(v) == 32 and all(r and r['span_ms'] > 0 for r in v) for v in visits)
        metrics = {}
        for metric in ('span_ms', 'receiver_ms', 'shading_ms', 'reconstruction_ms', 'present_ms'):
            values = [r[metric] for visit in visits for r in visit if r[metric] is not None]
            metrics[metric] = (dict(n=len(values), **dict(zip(('p50', 'p95', 'p99'),
                                                         np.quantile(values, [.5, .95, .99]).tolist())))
                               if values else None)
        configurations[name] = dict(metrics=metrics, visit_span_medians=[float(np.median([r['span_ms'] for r in v])) for v in visits])
    run = next(p for p in (ROOT / 'runs').glob('timing-*.json') if not p.name.endswith(('preflight.json', 'deferred.json')))
    guard = json.loads(run.read_text())
    pre = json.loads(run.with_name(run.stem + '-preflight.json').read_text())
    identity = json.loads((ROOT / 'input-identities.json').read_text())[str(ROOT / 'bin/rtdef-tests.exe')]
    save(ROOT / 'timing-summary.json', dict(configurations=configurations, guard=str(run),
         guard_seconds=guard['seconds'], gpu=pre['gpu'], binary_sha256=identity['sha256'],
         minimum_available_mbytes=min(r['available_mbytes'] for r in guard['resource_samples']),
         max_rustc=max(r['rustc_count'] for r in guard['resource_samples']),
         guard_foreign_builds=sum(len(r['builds']) for r in guard['samples']),
         guard_games=sum(len(r['games']) for r in guard['samples'])))


def chart():
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    rows = json.loads((ROOT / 's2-grain-diagnostic.json').read_text())
    data = {(r['frame'], r['mode']): r['grain_display_plus4']['rms'] for r in rows if r['case'] == 'camera-0.25px-r0.04'}
    cost = json.loads((ROOT / 'timing-summary.json').read_text())['configurations']
    fig, axes = plt.subplots(1, 2, figsize=(13, 4.6), layout='constrained')
    x = np.arange(3)
    for mode, offset, colour in [('Raster-Exact', -.18, '#707983'), ('TR1', .18, '#137a70')]:
        values = [data[(f, mode)] for f in (0, 31, 95)]
        bars = axes[0].bar(x + offset, values, width=.36, label=mode, color=colour)
        axes[0].bar_label(bars, fmt='%.2f', padding=3)
    axes[0].set(xticks=x, xticklabels=['Cold 0', 'Moving 31', 'Hold 95'], ylabel='+4-stop display RMS (codes)',
                title='S2 single-surface floor penumbra', ylim=(0, 26))
    axes[0].legend(frameon=False)
    names = ['raster-exact', 'traced', 'tr1']
    p50 = [cost[n]['metrics']['span_ms']['p50'] for n in names]
    p95 = [cost[n]['metrics']['span_ms']['p95'] for n in names]
    bars = axes[1].bar(x, p50, color=['#707983', '#a98637', '#137a70'],
                       yerr=[np.zeros(3), np.array(p95) - p50], capsize=4)
    axes[1].bar_label(bars, fmt='%.2f', padding=3)
    axes[1].set(xticks=x, xticklabels=['Raster Exact', 'Traced 8', 'Traced 8 + TR1'],
                ylabel='GPU rendered span (ms)', title='Quarry static p50; whisker p95', ylim=(0, 25))
    for ax in axes:
        ax.spines[['top', 'right']].set_visible(False)
        ax.set_axisbelow(True)
        ax.grid(axis='y', alpha=.18)
    fig.suptitle('RTDEF: held views improve; cold and moving grain block promotion', fontsize=14)
    fig.savefig(ROOT / 'comparison.png', dpi=150)
    plt.close(fig)


if __name__ == '__main__':
    import sys
    start = time.monotonic()
    action = sys.argv[1] if len(sys.argv) > 1 else 'all'
    if action in ('quarry', 'all'):
        quarry()
    if action in ('s2', 'all'):
        s2()
    if action in ('witness', 'all'):
        witness()
    if action in ('comparison', 'all'):
        comparison()
    if action in ('timing', 'all'):
        timing()
    if action in ('chart', 'all'):
        chart()
    print('CPU score seconds', time.monotonic() - start)
