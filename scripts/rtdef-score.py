"""CPU-only RTDEF radiance scoring with the existing S1 scorer and retained films.

S2 reports every frame and phase against its registered film and HRT1 rows. Quarry
scores only the registered still poses; the short motion loop is inspected, not given
a fabricated moving reference. No Unreal or reference renderer runs here.
"""
import csv
import hashlib
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
    print('CPU score seconds', time.monotonic() - start)
