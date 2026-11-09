"""Summarise complete ABBA tracks without deleting timing outliers."""
import json
import math
from pathlib import Path
import statistics as st

ROOT = Path('D:/Coding/renderlab-work/p-bvht')
PHASES = dict(still=range(32, 40), moving=range(64, 72), cut=[72], recovered=range(80, 88))


def stats(values):
    values = sorted(values)
    return dict(n=len(values), median=st.median(values), mean=st.mean(values), p90=values[math.ceil(.9 * len(values)) - 1], min=values[0], max=values[-1])


def main():
    rows = []
    for folder in sorted((ROOT / 'runs/quarry/nonrt').iterdir()):
        if not (folder / 'result.json').exists():
            continue
        result = json.loads((folder / 'result.json').read_text())
        engine = json.loads((folder / 'engine.json').read_text())
        gpu = {v['frame_id']: v for v in engine['gpu']}
        tracks = []
        for rep in range(4):
            phases = {}
            for phase, indices in PHASES.items():
                frames = [engine['frames'][rep * 88 + i] for i in indices]
                phase_gpu = [gpu[f['frame_id']] for f in frames]
                phases[phase] = dict(service=st.mean(f['cpu_service_ms'] for f in frames), **{
                    key: st.mean(f[key] for f in phase_gpu) for key in phase_gpu[0] if key.endswith('_ms') and phase_gpu[0][key] is not None})
            tracks.append(phases)
        rows.append(dict(folder=folder.name, result=result, tracks=tracks))
    if not rows:
        return
    summary = {}
    for phase in PHASES:
        summary[phase] = {metric: {side: stats([track[phase][metric] for row in rows if row['result']['variant'] == 'mem-' + side for track in row['tracks']])
            for side in ['before', 'after'] if any(row['result']['variant'] == 'mem-' + side for row in rows)} for metric in rows[0]['tracks'][0][phase]}
    first = rows[0]['result']
    identity = {key: all(row['result'][key] == first[key] for row in rows) for key in ['hdr', 'png', 'scene_sha256', 'asset_sha256']}
    output = dict(processes=len(rows), summary=summary, identity=identity,
        compiler_samples=sum(row['result']['compiler_samples'] for row in rows),
        audit_samples=sum(row['result']['audit_samples'] for row in rows),
        min_available_mib=min(row['result']['min_available_mib'] for row in rows), rows=rows)
    # Diagnostic split applied symmetrically: no threshold chosen from timing values.
    output['warm_tracks'] = {phase: {metric: {side: stats([track[phase][metric] for row in rows
        if row['result']['variant'] == 'mem-' + side for track in row['tracks'][1:]])
        for side in ['before', 'after'] if any(row['result']['variant'] == 'mem-' + side for row in rows)}
        for metric in ['service', 'span_ms']} for phase in PHASES}
    output['process_service_medians'] = {phase: [st.median(t[phase]['service'] for t in row['tracks']) for row in rows] for phase in PHASES}
    (ROOT / 'summary.json').write_text(json.dumps(output, indent=2))
    print(json.dumps({k: v for k, v in output.items() if k != 'rows'}, indent=2))


if __name__ == '__main__':
    main()
