"""Summarize retained quiet P-RETIME ABBA runs; reject missing or unequal witnesses."""
import json
from pathlib import Path
import statistics as st
import sys

ROOT = Path('D:/Coding/renderlab-work/p-retime')
rows = [json.loads(p.read_text()) for p in (ROOT / 'runs').glob('*/*/*/result.json')]
result = []
for prefix,case,mode in [('a','soft-quarry','rt'),('a','soft-quarry','nonrt'),
    ('a','quarry','rt'),('a','quarry','nonrt'),('a','lights','rt'),('a','lights','nonrt'),
    ('mem','game','nonrt'),('mem','quarry','nonrt')]:
    selected = sorted([r for r in rows if r['case']==case and r['mode']==mode and r['variant'].startswith(prefix+'-')],key=lambda r:r['index'])
    if '--partial' in sys.argv and len(selected)!=4:
        continue
    assert len(selected)==4, (prefix,case,mode,len(selected))
    assert [r['variant'] for r in selected] == [f'{prefix}-{s}' for s in ['before','after','after','before']]
    assert all(r['compiler_samples']==0 for r in selected)
    witnesses = ['hashes'] if case=='soft-quarry' else ['hdr','png']
    identity = {key:all(r[key]==selected[0][key] for r in selected[1:]) for key in witnesses}
    assert all(identity.values()), (prefix,case,mode,identity)
    timings = {}
    if case=='soft-quarry':
        for phase in ['time_default','time_camera_moving','time_moving_shadowed_lights']:
            if phase in selected[0]['report']:
                timings[phase] = {side:st.mean(r['report'][phase]['median_ms'] for r in selected if r['variant'].endswith(side)) for side in ['before','after']}
    else:
        for phase in ['still','moving','cut','recovered']:
            timings[phase] = {metric:{side:dict(mean_ms=st.mean(values),median_ms=st.median(values),min_ms=min(values),max_ms=max(values),n=len(values))
                for side in ['before','after'] for values in [[v for r in selected if r['variant'].endswith(side) for v in r['phases'][phase][metric]]]}
                for metric in ['gpu','service']}
    result.append(dict(unit=prefix,case=case,mode=mode,identity=identity,witnesses={key:len(selected[0][key]) for key in witnesses},
        audit_samples=sum(r['audit_samples'] for r in selected),min_available_mib=min(r['min_available_mib'] for r in selected),timings=timings))
    print(json.dumps(result[-1]),flush=True)
(ROOT / 'summary.json').write_text(json.dumps(result,indent=2))
