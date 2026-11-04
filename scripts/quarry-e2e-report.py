"""Summarize per-process and pooled quarry timings plus contention and GPU leases.

CPU only: py -3 -B scripts/quarry-e2e-report.py <session-directory>
Writes analysis.json beside immutable raw measurements. Ratios pair processes in
the same registered block; ranges are observed spreads, never confidence bounds.
"""
from pathlib import Path
import collections, datetime as dt, importlib.util, json, statistics, sys

ROOT = Path(__file__).resolve().parent.parent
OUT = Path(sys.argv[1]).resolve()
spec = importlib.util.spec_from_file_location('bench', ROOT / 'scripts/quarry-e2e.py')
b = importlib.util.module_from_spec(spec); spec.loader.exec_module(b)

records = []
raw = collections.defaultdict(lambda: collections.defaultdict(list))
pre, monitor, foreground, run_seconds, native_pre, owned_foreground = [], [], [], [], [], []
pressure_runs = []
for d in sorted(OUT.glob('[0-9][0-9]')):
    if not (d / 'run.json').exists(): continue
    r = json.loads((d / 'run.json').read_text()); key = (r['sequence'], r['mode'])
    snapshot = json.loads((d/'preflight.json').read_text())
    pre.append(snapshot)
    contention = json.loads((d/'contention.json').read_text())
    monitor += contention['samples']
    if r.get('memory_pressure'):
        available = [s['available_gib'] for s in contention['memory_samples']]
        pressure_runs.append(dict(directory=d.name,mode=r['mode'],block=r['block'],
            preflight_available_gib=snapshot['available_gib'], minimum_available_gib=min(available),
            maximum_available_gib=max(available), memory_samples=len(available),
            monitor_failures=contention['failures']))
    if r['mode'] == 'renderlab':
        f = json.loads((d/'frames.json').read_text()); rows = f['rows']
        for name in ('gpu_ms','cpu_frame_ms','cpu_calls_ms','submit_ms','update_ms','draw_preparation_ms','collection_ms','shadow_preparation_ms','upload_ms','encode_ms','completion_latency_ms'):
            raw[key][name] += [s[name] for s in rows]
            r[name] = statistics.median(s[name] for s in rows)
        raw[key]['frame_ms'] += [s['frame_to_frame_ms'] for s in rows if s['index'] > 0]
        r['frame_ms'] = statistics.median(s['frame_to_frame_ms'] for s in rows if s['index'] > 0)
        r['e2e_ms'] = f['completed_ms_per_frame']
        r['startup_ms'] = f['startup_to_first_completed_image_ms']
        r['load_upload_ms'] = f['load_upload_ms']
        r['hdr_equal'] = f['hdr_matches_accepted']
        r['updates'] = dict(camera=sum(s['camera_bytes'] for s in rows), light=sum(s['light_bytes'] for s in rows), geometry=sum(s['geometry_bytes'] for s in rows))
        r['cpu_critical_proxy_ms'] = statistics.median(s['cpu_frame_ms'] + s['collection_ms'] for s in rows)
        raw[key]['cpu_critical_proxy_ms'] += [s['cpu_frame_ms'] + s['collection_ms'] for s in rows]
        run_seconds.append(r['elapsed_seconds'])
    else:
        summary = json.loads((d/'ue-summary.json').read_text())
        rows = b.csv_rows(Path(summary['csv']))[96:160]
        fields = dict(gpu_ms='GPUTime', frame_ms='FrameTime', gt_critical='GameThreadTime_CriticalPath', rt_critical='RenderThreadTime_CriticalPath', rhi='RHIThreadTime', gt='GameThreadTime', rt='RenderThreadTime')
        for name, field in fields.items():
            raw[key][name] += [s[field] for s in rows]
            r[name] = statistics.median(s[field] for s in rows)
        cp = [max(s['GameThreadTime_CriticalPath'],s['RenderThreadTime_CriticalPath'],s['RHIThreadTime']) for s in rows]
        r['cpu_critical_proxy_ms'] = statistics.median(cp)
        raw[key]['cpu_critical_proxy_ms'] += cp
        r['e2e_ms'] = statistics.mean(s['FrameTime'] for s in rows)
        r['trees'] = len(summary['log_evidence']['gpu_tree'])
        r['blas_triangles'] = [x['native_nanite_quarry_triangles'] for x in summary['log_evidence']['ray_tracing_dumps']]
        evidence = json.loads((d/'ue-evidence.json').read_text())
        proc = json.loads((Path(evidence['launch'])/'process.json').read_text())
        foreground.append(proc['same_foreground_before_after'])
        if not r.get('memory_pressure'):
            native_pre.append(json.loads((Path(evidence['launch'])/'preflight.json').read_text(encoding='utf-8-sig'))['memory']['FreePhysicalMemory']/1024**2)
        for name in ('preflight.json','postflight.json'):
            snapshot = json.loads((Path(evidence['launch'])/name).read_text(encoding='utf-8-sig'))
            owned_foreground.append(snapshot['foreground_read_only']['process_id'] == proc['process_id'])
        run_seconds.append(proc['elapsed_seconds'])
    records.append(r)

ratios=[]
for r in records:
    if r['mode']=='renderlab': continue
    rl=next((x for x in records if x['mode']=='renderlab' and x['sequence']==r['sequence'] and x['block']==r['block']),None)
    if rl is None: continue
    ratios.append(dict(sequence=r['sequence'],mode=r['mode'],block=r['block'],
        gpu=r['gpu_ms']/rl['gpu_ms'],cpu=r['cpu_critical_proxy_ms']/rl['cpu_critical_proxy_ms'],
        frame_median=r['frame_ms']/rl['frame_ms'],throughput=r['e2e_ms']/rl['e2e_ms']))

grouped=[]
for sequence in ('static','camera','sun'):
    for mode in ('renderlab','vsm','hrt'):
        rs=[r for r in records if r['sequence']==sequence and r['mode']==mode]
        if not rs: continue
        grouped.append(dict(sequence=sequence,mode=mode,processes=len(rs),
            process_statistics={name:b.stats([r[name] for r in rs]) for name in ('gpu_ms','cpu_critical_proxy_ms','frame_ms','e2e_ms')},
            pooled={name:b.stats(v) for name,v in raw[(sequence,mode)].items()}))
ratio_groups=[]
for sequence in ('static','camera','sun'):
    for mode in ('vsm','hrt'):
        rs=[r for r in ratios if r['sequence']==sequence and r['mode']==mode]
        if rs: ratio_groups.append(dict(sequence=sequence,mode=mode,statistics={k:b.stats([r[k] for r in rs]) for k in ('gpu','cpu','frame_median','throughput')}))

windows=[]
for d in OUT.parent.iterdir():
    if (d/'lease-history.json').exists(): windows += json.loads((d/'lease-history.json').read_text())
    elif (d/'lease-released.json').exists(): windows.append(json.loads((d/'lease-released.json').read_text()))
for w in windows:
    w['seconds']=(dt.datetime.fromisoformat(w['released_utc'])-dt.datetime.fromisoformat(w['acquired']['utc'])).total_seconds()

meta=dict(accepted_preflight_min_gib=min([p['free_gib'] for p in pre if not p.get('memory_pressure')]+native_pre),samples=len(monitor),
    monitor_min_gib=min(p['free_gib'] for p in monitor), monitor_max_builds=max(len(p['builds']) for p in monitor),
    monitor_build_samples=sum(bool(p['builds']) for p in monitor),
    conflicts=sum(bool(p['games'] or p['foreign_renderers']) for p in monitor),
    native_foreground_same=foreground, owned_unreal_foreground_endpoints=sum(owned_foreground),
    total_owned_run_seconds=sum(run_seconds),
    pressure_runs=pressure_runs,
    lease_windows=windows,lease_total_seconds=sum(w['seconds'] for w in windows), lease_max_seconds=max(w['seconds'] for w in windows))
b.save(OUT/'analysis.json',dict(records=records,grouped=grouped,ratios=ratios,ratio_groups=ratio_groups,meta=meta))
def cell(s): return f"{s['median']:.4f} [{s['minimum']:.4f}, {s['maximum']:.4f}]"
print('TIMES: medians/range of process values')
for g in grouped:
    print(g['sequence'],g['mode'],g['processes'],*[f'{k}: {cell(v)}' for k,v in g['process_statistics'].items()])
print('RATIOS: medians/range of block ratios')
for g in ratio_groups:
    print(g['sequence'],g['mode'],*[f'{k}: {cell(v)}' for k,v in g['statistics'].items()])
print('META',json.dumps(meta,indent=2))
