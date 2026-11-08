"""RT1d pilot/cost inside n8-run.py; score is CPU-only and reads retained references."""
import argparse
import csv
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import zipfile


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), Path(__file__).with_name(name+'.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def save(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False)+'\n')


def leased():
    owner = json.loads(Path('D:/Coding/renderlab-work/gpu-slot.lock/owner.json').read_text())
    assert owner['owner'] == 'rt1d-shadow-filter', owner
    lines = Path('E:/renderlab-work/gpu-schedule.md').read_text().splitlines()
    directives = [s for s in lines if s.startswith(('[GAME HOLD]', '[GAME HOLD RELEASED]', '[GAME HOLD RELAXED]'))]
    assert directives and directives[-1].startswith('[GAME HOLD RELEASED]'), directives[-1:]


def run(binary, name, manifest=None):
    leased()
    env = os.environ.copy()
    if manifest is not None:
        env['RENDERLAB_PRIMARY_MANIFEST'] = str(manifest)
    command = [str(binary)]
    if name:
        command += [name, '--ignored']
    command += ['--nocapture', '--test-threads=1']
    subprocess.run(command, check=True, env=env, stdout=sys.stdout, stderr=subprocess.STDOUT,
                   creationflags=subprocess.CREATE_NO_WINDOW)


def pilot(args):
    run(args.binary, None)
    for variant in ('previous', 'filtered'):
        run(args.binary, 'gpu_motion_bench_batch', args.root/f'witness-{variant}.json')


def prepare(args):
    for bench in ('S2', 'S3'):
        root = args.root/'motion'/bench
        (root/'fixture').mkdir(parents=True, exist_ok=True)
        source = Path('E:/renderlab-work/motion-benches')/bench/'fixture/scene.json'
        scene = json.loads(source.read_text())
        save(root/'fixture/scene.json', scene)
        save(root/'manifest.json', dict(scene=str(root/'fixture/scene.json'),
            output=str(root/'renderlab'), modes=['Interactive'], repeats=8, shadow_filter=True))
    root = args.root/'S1'
    root.mkdir(exist_ok=True)
    save(root/'manifest.json', dict(scene='E:/renderlab-work/contact-fold/fixture/scene.json',
        output=str(root/'renderlab'), modes=['Interactive'], shadow_filter=True))
    m = json.loads(Path('D:/Coding/renderlab-work/m6-adaptive/gpu-phase-quarry/quarry-manifest.json').read_text())
    m['output'] = str(args.root/'quarry')
    save(args.root/'quarry.json', m)


def quality(args):
    if args.bench in ('S2', 'S3'):
        run(args.binary, 'gpu_motion_bench_batch', args.root/'motion'/args.bench/'manifest.json')
    elif args.bench == 'S1':
        run(args.binary, 'gpu_contact_fold_batch', args.root/'S1/manifest.json')
    elif args.bench == 'quarry':
        run(args.binary, 'gpu_rt1d_quarry_pair', args.root/'quarry.json')


def motion_score(args):
    os.environ.update(OPENBLAS_NUM_THREADS='1', OMP_NUM_THREADS='1')
    scorer = load('bench-motion-score')
    audit = load('rt1c-hrt-audit')
    # Keep every numeric reference bit, spending less CPU on disposable compression
    # effort. This remains an ordinary np.load-compatible NPZ archive.
    def fast_archive(file, **values):
        with zipfile.ZipFile(file, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=1) as archive:
            for name, value in values.items():
                with archive.open(name+'.npy', 'w', force_zip64=True) as stream:
                    scorer.np.lib.format.write_array(stream, scorer.np.asanyarray(value), allow_pickle=False)
    scorer.np.savez_compressed = fast_archive
    root = args.root/'motion'/args.bench
    fresh = []
    for directory in sorted((root/'renderlab').iterdir()):
        if directory.is_dir():
            cached = directory/'scores-renderlab.json'
            fresh += json.loads(cached.read_text()) if cached.exists() else scorer.score_case(root, directory.name, True)
    save(root/'per-frame.json', fresh)
    oldpath = Path('E:/renderlab-work/motion-benches')/args.bench/'per-frame.json'
    old = json.loads(oldpath.read_text())
    comparisons, phases = [], []
    for case, phase in sorted({(r['case'], r['phase']) for r in fresh}):
        groups = dict(filtered=[r for r in fresh if (r['case'],r['phase']) == (case,phase)])
        groups.update({name:[r for r in old if (r['case'],r['phase'],r['mode']) == (case,phase,mode)]
                       for name,mode in (('previous','RL Interactive'),('hrt1','UE HRT1'))})
        assert len({tuple(r['frame'] for r in rows) for rows in groups.values()}) == 1
        values = {name:audit.phase_values(rows) for name,rows in groups.items()}
        phases.append(dict(case=case, phase=phase, **values))
        for measure in values['filtered']:
            if measure.endswith('frames'):
                continue
            row = {name:v[measure] for name,v in values.items()}
            comparisons.append(dict(case=case, phase=phase, measure=measure, **row,
                passes=row['filtered']<=row['hrt1']+1e-12 if row['filtered'] is not None and row['hrt1'] is not None else None))
    save(root/'gate.json', dict(phases=phases, comparisons=comparisons,
        retained_scores_sha256=hashlib.sha256(oldpath.read_bytes()).hexdigest(),
        misses=[r for r in comparisons if r['passes'] is False]))
    with (root/'gate.csv').open('w', newline='') as stream:
        writer=csv.DictWriter(stream, fieldnames=list(comparisons[0]))
        writer.writeheader()
        writer.writerows(comparisons)
    print(json.dumps(dict(phases=len(phases), misses=[r for r in comparisons if r['passes'] is False]),indent=2))


def static_score(args):
    os.environ.update(OPENBLAS_NUM_THREADS='1', OMP_NUM_THREADS='1')
    import numpy as np
    s1 = load('bench-contact-fold-score')
    audit = load('rt1c-hrt-audit')
    root = args.root/'S1'
    scene = json.loads(Path('E:/renderlab-work/contact-fold/fixture/scene.json').read_text())
    oldpath = Path('E:/renderlab-work/contact-fold/scores.json')
    old = json.loads(oldpath.read_text())
    comparisons, rows = [], {}
    for pose in (0,1):
        for environment in (0,1):
            case = f'p{pose}-e{environment}'
            directory = root/'renderlab'/(f'p{pose}-'+('environment' if environment else 'black'))
            d = s1.reference(directory, pose, scene)
            pops = s1.metrics.populations(d)
            shape = d['ref'].shape
            vis = np.fromfile(directory/'Interactive-visibility.rgba16f','<f2').reshape(*shape,4).astype(float)[...,0]
            hdr = np.fromfile(directory/'Interactive-n32.rgba16f','<f2').reshape(*shape,4).astype(float)
            filtered = s1.scored(vis,d,pops,np.ones(shape,bool))
            filtered['radiance'] = s1.radiance(hdr,d['hdr'])
            rows[case] = filtered
            values = dict(filtered=audit.phase_values([filtered]),
                previous=audit.phase_values([old[case]['modes']['RL Interactive']]),
                hrt1=audit.phase_values([old[case]['modes']['UE HRT1']]))
            for measure in values['filtered']:
                if measure.endswith('frames'):continue
                r = {name:v[measure] for name,v in values.items()}
                comparisons.append(dict(case=case,measure=measure,**r,
                    passes=r['filtered']<=r['hrt1']+1e-12 if r['filtered'] is not None and r['hrt1'] is not None else None))
            if not (directory/'reference.npz').exists():
                np.savez_compressed(directory/'reference.npz', **{k:v for k,v in d.items() if k!='info'})
            s1.film.tone(hdr).save(directory/'filtered.png')
            s1.film.tone(hdr,4).save(directory/'filtered-plus4.png')
    save(root/'gate.json',dict(rows=rows,comparisons=comparisons,
        retained_scores_sha256=hashlib.sha256(oldpath.read_bytes()).hexdigest(),
        misses=[r for r in comparisons if r['passes'] is False]))
    print(json.dumps(dict(misses=[r for r in comparisons if r['passes'] is False]),indent=2))


def quarry_score(args):
    os.environ.update(OPENBLAS_NUM_THREADS='1', OMP_NUM_THREADS='1')
    import numpy as np
    from PIL import Image
    from scipy.ndimage import binary_dilation
    rt1 = load('rt1-film-replay')
    s1 = load('bench-contact-fold-score')
    root = args.root/'quarry'
    rows, comparisons = {}, []
    for pose in (0,1):
        refdir = Path(f'E:/renderlab-work/m6-exact-quality/r1/quarry-viewed-p{pose}')
        d = rt1.film_reference(refdir,pose)
        film = rt1.read(refdir/f'pose-{pose}.film.bin',rt1.FILM)
        hdr_ref = film['rest'][...,4:7].astype(float)/256
        variants = {}
        for name in ('previous','filtered'):
            vis = np.fromfile(root/f'p{pose}-{name}-visibility.rgba16f','<f2').reshape(*rt1.SHAPE,4).astype(float)[...,0]
            hit = np.fromfile(root/f'p{pose}-{name}.hit.u32','<u4').reshape(*rt1.SHAPE,4)
            hdr = np.fromfile(root/f'p{pose}-{name}.rgba16f','<f2').reshape(*rt1.SHAPE,4).astype(float)
            variants[name] = rt1.metrics(vis,d,hit[...,0]!=0)
            pops = rt1.gate.populations(d['ref'],d['depth'],d['cover']&(d['n']>0))
            edge = pops['occluded'] & binary_dilation(pops['lit']|pops['partial'],np.ones((7,7),bool))
            variants[name]['contact_lost'] = int((edge&(hit[...,0]!=0)&(vis-d['ref']>.5)).sum())
            variants[name]['radiance'] = s1.radiance(hdr,hdr_ref)
            s1.film.tone(hdr).save(root/f'p{pose}-{name}.png')
            s1.film.tone(hdr,4).save(root/f'p{pose}-{name}-plus4.png')
            s1.film.error(vis-d['ref'],d['cover'],hit[...,0]!=0).save(root/f'p{pose}-{name}-error.png')
        rt1.lb.NATIVE = Path('D:/Coding/renderlab-work/ue-m5/Quarry/native-shadow')
        nativepath = rt1.lb.native_dir('hrt1',pose)/'native.npz'
        with np.load(nativepath) as native:
            variants['hrt1'] = rt1.metrics(native['visibility'],d,native['valid'])
            pops = rt1.gate.populations(d['ref'],d['depth'],d['cover']&(d['n']>0))
            edge = pops['occluded'] & binary_dilation(pops['lit']|pops['partial'],np.ones((7,7),bool))
            variants['hrt1']['contact_lost'] = int((edge&native['valid']&(native['visibility']-d['ref']>.5)).sum())
        rows[str(pose)] = variants
        for measure in variants['hrt1']:
            if measure in ('covered','partial_n'):continue
            r = {name:v[measure] for name,v in variants.items()}
            comparisons.append(dict(pose=pose,measure=measure,**r,passes=r['filtered']<=r['hrt1']))
    save(root/'gate.json',dict(rows=rows,comparisons=comparisons,misses=[r for r in comparisons if not r['passes']]))
    print(json.dumps(dict(comparisons=comparisons),indent=2))


def quarry_crops(args):
    os.environ.update(OPENBLAS_NUM_THREADS='1', OMP_NUM_THREADS='1')
    import numpy as np
    rt1 = load('rt1-film-replay')
    root = args.root/'quarry'
    witnesses = []
    for pose in (0,1):
        d = rt1.film_reference(Path(f'E:/renderlab-work/m6-exact-quality/r1/quarry-viewed-p{pose}'),pose)
        values = {name:np.fromfile(root/f'p{pose}-{name}-visibility.rgba16f','<f2').reshape(*rt1.SHAPE,4)[...,0].astype(float)
                  for name in ('previous','filtered')}
        valid = np.fromfile(root/f'p{pose}-filtered.hit.u32','<u4').reshape(*rt1.SHAPE,4)[...,0]!=0
        mask = d['cover']&(d['n']>0)&valid&(d['ref']>=.99)&(values['filtered']<=.9)&(values['previous']>.9)
        y,x = np.nonzero(mask)
        chosen = [dict(x=int(x[i]),y=int(y[i])) for i in range(0,len(x),max(1,len(x)//12))][:12]
        rt1.crops(root/f'p{pose}-new-false.png',chosen,[('Film',d['ref']),*values.items()])
        witnesses.append(dict(pose=pose,new_false=int(mask.sum()),coordinates=chosen))
    save(root/'new-false-witnesses.json',witnesses)


def cost_score(args):
    os.environ.update(OPENBLAS_NUM_THREADS='1', OMP_NUM_THREADS='1')
    import numpy as np
    result = {}
    for name in ('previous','filtered'):
        directory = args.root/'cost'/name/'rt'
        data = json.loads((directory/'m1-cost.json').read_text())
        phases = {}
        for phase in ('static','camera','sun','caster'):
            values = [r for r in data['samples'] if r['phase']==phase]
            phases[phase] = {key:dict(p50=float(np.quantile([r[key] for r in values],.5)),
                                      p95=float(np.quantile([r[key] for r in values],.95)))
                             for key in ('primary_ms','shade_ms','tone_ms','span_ms')}
            phases[phase]['samples'] = len(values)
        reports = [json.loads(p.read_text()) for p in directory.glob('memory-p*.json')]
        run = json.loads((directory/'run.json').read_text())
        result[name] = dict(phases=phases,memory=dict(
            allocator_allocated_mib=max(r['allocator_allocated_bytes'] for r in reports)/2**20,
            allocator_reserved_mib=max(r['allocator_reserved_bytes'] for r in reports)/2**20,
            wddm_warm_mib=run['wddm_warm_max_bytes']/2**20,
            wddm_process_mib=run['wddm_max_bytes']/2**20,
            wddm_warm_samples=run['wddm_warm_samples']))
    native = load('ue-shadow-motion')
    hrt, identities = {}, {}
    for phase in ('static','camera','sun','caster'):
        values = []
        for directory in Path('E:/renderlab-work/m6-close/ue/s1').glob(f'*-hrt-{phase}'):
            path = sorted(directory.glob('Profile*.csv'))[-1]
            header, data = native.read_csv(path)
            excluded = {f for p in native.PROFILE_FRAMES for f in (p,p+1)}
            selected = [f for f in native.BLOCK if f<len(data) and f not in excluded]
            values += [float(data[f][header.index('GPUTime')]) for f in selected]
            identities[str(path)] = hashlib.sha256(path.read_bytes()).hexdigest()
        hrt[phase] = dict(p50=float(np.quantile(values,.5)),p95=float(np.quantile(values,.95)),samples=len(values))
    result['recorded_hrt_ms'] = hrt
    result['hrt_input_sha256'] = identities
    result['scope'] = 'Renderlab: all individual frames, no clean-visit exclusion. HRT: retained M6-close CSV frames using the original BLOCK and ProfileGPU-exclusion rule; not a new paired run. GPU boundaries differ.'
    save(args.root/'cost/summary.json',result)
    print(json.dumps(result,indent=2))


def quality_tables(args):
    gates = {name:json.loads(path.read_text()) for name,path in (
        ('S2',args.root/'motion/S2/gate.json'),('S3',args.root/'motion/S3/gate.json'),('S1',args.root/'S1/gate.json'))}
    def number(v):
        return '—' if v is None else f'{v:.6f}' if isinstance(v,float) else str(v)
    def passes(rows):
        rows = [r for r in rows if r['passes'] is not None]
        if not rows:return '—'
        old = sum(r['previous']<=r['hrt1']+1e-12 for r in rows)
        new = sum(r['passes'] for r in rows)
        return f'{old}/{len(rows)} → {new}/{len(rows)}'
    lines = ['## Every retained measure against HRT', '',
        'Each cell counts passing configurations/phases: **previous fast setup → filtered**. Ties pass the unchanged no-worse floor. The CSV/JSON artifacts retain every paired value, population count, sum and maximum. No missing measure is counted as a pass.', '',
        '| Measure | S2 (24 phases) | S3 (6 phases) | S1 (4 configurations) |', '|---|---:|---:|---:|']
    for measure in dict.fromkeys(r['measure'] for r in gates['S2']['comparisons']):
        cells = [passes([r for r in gates[name]['comparisons'] if r['measure']==measure]) for name in ('S2','S3','S1')]
        lines.append('| '+measure+' | '+' | '.join(cells)+' |')
    lines += ['', '## All S2/S3 phases', '',
        'Each value is **previous / filtered / HRT1**. Display RMS is in 8-bit codes. Flicker is mean visibility residual change, using the original screen-coordinate, film-relative metric; it is not a final-HDR flicker certificate.', '',
        '| Case / phase | Partial RMS | +0 display RMS | +4 display RMS | Visibility flicker |', '|---|---:|---:|---:|---:|']
    for bench in ('S2','S3'):
        for row in gates[bench]['phases']:
            cells = [' / '.join(number(row[name][key]) for name in ('previous','filtered','hrt1'))
                     for key in ('partial_rms','display_+0_codes_rms','display_+4_codes_rms','visibility_flicker_mean')]
            lines.append(f'| {bench} {row["case"]} / {row["phase"]} | '+' | '.join(cells)+' |')
    lines += ['', '## S1 static values', '',
        '| Configuration | Partial RMS (previous / filtered / HRT) | +0 display RMS | +4 display RMS | Strict-solid defects |', '|---|---:|---:|---:|---:|']
    audit = load('rt1c-hrt-audit')
    old = json.loads(Path('E:/renderlab-work/contact-fold/scores.json').read_text())
    for case,filtered in gates['S1']['rows'].items():
        rows = [old[case]['modes']['RL Interactive'],filtered,old[case]['modes']['UE HRT1']]
        values = list(map(lambda r:audit.phase_values([r]),rows))
        cells = [' / '.join(number(r[key]) for r in values) for key in
                 ('partial_rms','display_+0_codes_rms','display_+4_codes_rms','solid_error_gt_002_max')]
        lines.append(f'| {case} | '+' | '.join(cells)+' |')
    (args.root/'quality-tables.md').write_text('\n'.join(lines)+'\n',encoding='utf-8')
    print({name:dict(comparisons=len(g['comparisons']),misses=len(g['misses'])) for name,g in gates.items()})


def score(args):
    os.environ.update(OPENBLAS_NUM_THREADS='1', OMP_NUM_THREADS='1')
    import numpy as np
    from PIL import Image, ImageDraw
    s1 = load('bench-contact-fold-score')
    audit = load('rt1c-hrt-audit')
    case = 'camera-0.25px-r0.04'
    refdir = Path('E:/renderlab-work/m7-refs/rt1c-witness')/case/'reference'
    scene = json.loads(Path('E:/renderlab-work/rt1c/witness-scene.json').read_text())
    d = s1.reference(refdir, 0, scene)
    d['depth'] = d['surface_depth']
    pops = s1.metrics.populations(d)
    shape = d['ref'].shape
    rows, identities = {}, {}
    hdrs = {}
    for name in ('previous', 'filtered'):
        directory = args.root/f'witness-{name}'/case/'Interactive'
        paths = [directory/'0000.rgba16f', directory/'0000-visibility.rgba16f']
        hdr, visibility = [np.fromfile(p, '<f2').reshape(*shape, 4).astype(float) for p in paths]
        hdrs[name] = hdr
        rows[name] = s1.scored(visibility[..., 0], d, pops, np.ones(shape, bool))
        rows[name]['radiance'] = s1.radiance(hdr, d['hdr'])
        identities.update({str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths})
    recorded = Path('E:/renderlab-work/motion-benches/S2/per-frame.json')
    hrt, = [r for r in json.loads(recorded.read_text())
            if r['case'] == case and r['frame'] == 0 and r['mode'] == 'UE HRT1']
    rows['hrt1'] = hrt
    comparisons = []
    for metric in audit.phase_values([rows['filtered']]):
        values = {name: audit.phase_values([row])[metric] for name, row in rows.items()}
        if metric.endswith('frames'):
            continue
        comparisons.append(dict(measure=metric, **values,
            passes=values['filtered'] <= values['hrt1']+1e-12
            if values['filtered'] is not None and values['hrt1'] is not None else None))
    panels = []
    for stops in (0, 4):
        images = [('Film', s1.film.tone(d['hdr'], stops))]
        images += [(name, s1.film.tone(hdr, stops)) for name, hdr in hdrs.items()]
        suffix = '-plus4' if stops else ''
        native = Path(f'E:/renderlab-work/motion-benches/S2/unreal/{case}/HRT1/0000/final-radiance{suffix}.png')
        images.append(('HRT1', Image.open(native).convert('RGB').crop(tuple(scene['crop']))))
        for name, hdr in hdrs.items():
            display = np.asarray(s1.film.tone(hdr, stops)).astype(float)
            ref = np.asarray(images[0][1]).astype(float)
            rows[name][f'display_bias_{stops:+}'] = float(np.mean(display-ref))
        sheet = Image.new('RGB', (1024, 624), '#141b23')
        draw = ImageDraw.Draw(sheet)
        for i, (name, im) in enumerate(images):
            x, y = i % 2 * 512, i // 2 * 312
            draw.text((x+4, y+3), f'{name}, +{stops} stops', fill='white')
            sheet.paste(im, (x, y+24))
        sheet.save(args.root/f'witness{suffix}.png')
        identities[str(native)] = hashlib.sha256(native.read_bytes()).hexdigest()
    identities[str(recorded)] = hashlib.sha256(recorded.read_bytes()).hexdigest()
    for p in refdir.iterdir():
        identities[str(p)] = hashlib.sha256(p.read_bytes()).hexdigest()
    save(args.root/'witness-score.json', dict(rows=rows, comparisons=comparisons, inputs=identities,
        scope='Native S2 crop, frame 0; current GPU visibility filter; retained film and HRT. No motion certification.'))
    with (args.root/'witness-comparison.csv').open('w', newline='') as stream:
        writer = csv.DictWriter(stream, fieldnames=list(comparisons[0]))
        writer.writeheader()
        writer.writerows(comparisons)
    print(json.dumps(dict(comparisons=comparisons, biases={name: {k:v for k,v in row.items() if k.startswith('display_bias')} for name,row in rows.items()}), indent=2))


def cost(args):
    leased()
    memory = load('memory-m1')
    for name, filtered in (('previous', False), ('filtered', True)):
        leased()
        memory.run(args.binary, args.root/'cost'/name, 'rt', False, 'default', [0, 1], 8,
                   rt1c=True, shadow_filter=filtered)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('prepare', 'pilot', 'score', 'cost', 'quality', 'motion_score', 'static_score', 'quarry_score', 'quarry_crops', 'cost_score', 'quality_tables'))
    parser.add_argument('--root', type=Path, default=Path('D:/Coding/renderlab-work/rt1d'))
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--bench', choices=('S1', 'S2', 'S3', 'quarry'))
    args = parser.parse_args()
    args.root.mkdir(parents=True, exist_ok=True)
    started = time.perf_counter()
    globals()[args.action](args)
    print(args.action, 'wall seconds', time.perf_counter()-started, flush=True)


if __name__ == '__main__':
    main()
