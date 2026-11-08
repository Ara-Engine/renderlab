"""VF1's local inspection sheets and paired timing summary (no renderer/GPU work)."""
import json
from pathlib import Path
import sys

from PIL import Image, ImageDraw, ImageFont
import numpy as np

ROOT = Path('D:/Coding/renderlab-work/m7-vf1/images') / (sys.argv[1] if len(sys.argv) > 1 else 'a')
font = ImageFont.truetype('C:/Windows/Fonts/arial.ttf', 18)


def sheet(name, rows, width=640):
    height = width * 9 // 16
    image = Image.new('RGB', (len(rows[0]) * width, len(rows) * (height + 30)), '#17191b')
    draw = ImageDraw.Draw(image)
    for y, row in enumerate(rows):
        for x, (label, file) in enumerate(row):
            with Image.open(file) as source:
                image.paste(source.resize((width, height)), (x * width, y * (height + 30) + 30))
            draw.text((x * width + 8, y * (height + 30) + 4), label, font=font, fill='white')
    image.save(ROOT / name)


paths = ['nonrt', 'traced', 'raster']
sheet('quarry-sheet.png', [[(p + ': ' + mode, ROOT / p / (p + '-' + config + '.png'))
    for mode, config in [('analytic only', 'low'), ('default 0.001/m', 'low-default'), ('shafts 0.01/m', 'low-volumetric')]] for p in paths])
sheet('quarry-default-sheet.png', [[(p + ': ' + mode, ROOT / p / (p + '-' + config + '.png'))
    for mode, config in [('AT1 default', 'legacy'), ('VF1 default', 'default')]] for p in paths])
sheet('motion-sheet.png', [[(p + ': frame ' + str(k), ROOT / p / f'{p}-motion-{k:02}.png') for k in [0,7,15,23]] for p in paths], 480)
if (ROOT / 'nonrt/nonrt-shaft-view-dense.png').exists():
    sheet('shafts-sheet.png', [[(p + ': ' + mode, ROOT / p / f'{p}-shaft-view-{mode}.png')
        for mode in ['off','default','dense']] for p in paths])
game_rows = []
for p in ['nonrt', 'rt']:
    game_rows.append([(p + ': ' + mode, sorted((ROOT / ('game-' + p + '-' + mode)).glob('*.png'))[-1])
        for mode in ['off', 'default', 'shafts']])
sheet('game-sheet.png', game_rows, 640)

summary = {'quarry': {}, 'game': {}}
previous = json.loads((ROOT / 'summary.json').read_text()) if (ROOT / 'summary.json').exists() else {}
for p in paths:
    data = json.loads((ROOT / p / (p + '-at1.json')).read_text())
    summary['quarry'][p] = data
    for name, before, after in [('captured-default', 'off', 'default'),
                              ('low-default', 'procedural-low', 'default-fog'),
                              ('low-shafts', 'procedural-low', 'volumetric')]:
        pairs = [[data['timing'][f'{key}-r{k}']['wall_ms_per_frame'] for key in [before, after]] for k in [0,1]]
        print(p, name, [[round(a,4),round(b,4),round(b-a,4)] for a,b in pairs])
    # Preserve useful full-HDR comparisons before deleting bulk raw dumps.
    comparisons = {}
    for a,b in [('legacy','default'),('low','low-default'),('low','low-volumetric')]:
        if not (ROOT / p / f'{p}-{a}.f32').exists():
            comparisons[a+'-to-'+b] = previous['quarry'][p]['hdr_comparisons'][a+'-to-'+b]
            continue
        old = np.fromfile(ROOT / p / f'{p}-{a}.f32', dtype='<f4').reshape(-1,3)
        new = np.fromfile(ROOT / p / f'{p}-{b}.f32', dtype='<f4').reshape(-1,3)
        d = new-old
        comparisons[a+'-to-'+b] = dict(mean_delta=d.mean(axis=0,dtype=np.float64).tolist(),
            mean_absolute_delta=np.abs(d).mean(axis=0,dtype=np.float64).tolist(),
            finite=bool(np.isfinite(new).all()))
    summary['quarry'][p]['hdr_comparisons'] = comparisons
for p in ['nonrt','rt']:
    summary['game'][p] = {}
    for mode in ['off','default','shafts']:
        data = json.loads((ROOT / ('game-'+p+'-'+mode) / 'run.json').read_text())
        samples = data['gpu_render_span_ms']
        summary['game'][p][mode] = dict(adapter=data['adapter'], debug_build=data['debug_build'],
            gpu_span_ms_p50=float(np.median(samples)) if samples else None,
            cpu_render_ms_p50=float(np.median([f['cpu_render_ms'] for f in data['frames'][8:]])))
print(json.dumps(summary['game'], indent=2))
(ROOT / 'summary.json').write_text(json.dumps(summary, indent=2))

chart = Image.new('RGB', (1800, 430), '#17191b')
draw = ImageDraw.Draw(chart)
draw.text((24, 14), 'Quarry low sun: shared-host wall throughput (ms/frame); dots are the two rounds, not an overhead bound', font=font, fill='white')
settings = [('analytic only', 'procedural-low', '#87939b'), ('default fog', 'default-fog', '#6db59b'), ('dense fog', 'volumetric', '#d6b27c')]
ceiling = max(v['timing'][f'{key}-r{k}']['wall_ms_per_frame'] for v in summary['quarry'].values() for _,key,_ in settings for k in [0,1]) * 1.1
for i,p in enumerate(paths):
    x = i*600 + 24
    draw.text((x,55),p,font=font,fill='white')
    for row,(label,key,color) in enumerate(settings):
        y = 98 + row*85
        values = [summary['quarry'][p]['timing'][f'{key}-r{k}']['wall_ms_per_frame'] for k in [0,1]]
        draw.text((x,y),f'{label}: {values[0]:.3f} / {values[1]:.3f}',font=font,fill='white')
        length = sum(values)/2/ceiling*530
        draw.rectangle((x,y+29,x+length,y+46),fill=color)
        for value in values:
            px=x+value/ceiling*530
            draw.ellipse((px-3,y+34,px+3,y+40),fill='white')
    draw.line((x,375,x+530,375),fill='#777777')
    for tick in range(0,int(ceiling)+1,5):
        px=x+tick/ceiling*530
        draw.line((px,370,px,380),fill='#aaaaaa')
        draw.text((px,387),str(tick),font=font,fill='white')
chart.save(ROOT/'timing-chart.png')
