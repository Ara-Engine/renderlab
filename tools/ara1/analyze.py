"""Summarize immutable captures, register comparisons, and reuse image metrics."""
import argparse
import json
from pathlib import Path
import statistics

import numpy as np
from PIL import Image

from capture import digest
from images import compose, score
from prepare import ROOT, save

SEGMENTS={'still':range(32,40),'move':range(64,72),'cut_first':[72],
          'cut_recovery':range(73,80),'cut_settled':range(80,88)}


def mark_roles(captures):
    latest={}
    for capture in captures:
        if Path(capture['folder']).name!=capture['mode']:continue
        key=(capture['scene_sha256'],capture['mode'])
        if key in latest:latest[key]['campaign_role']='superseded later capture of the same frozen case'
        latest[key]=capture


def stats(values):
    values=[float(v) for v in values if v is not None]
    return dict(n=len(values),median=float(np.median(values)),p90=float(np.percentile(values,90)),
                minimum=min(values),maximum=max(values)) if values else None


def summarize(folder):
    p=json.loads((folder/'process.json').read_text())
    e=json.loads((folder/'engine.json').read_text())
    result={k:p.get(k) for k in ('engine','mode','resolution','profile','binary_sha256','source',
        'scene_sha256','asset_sha256','contention','hardware','cpu','lease','root_order','process_seconds','launch_to_ready_file_ms','gpu_environment','seeded_gpu_cache')}
    result['startup_memory']=p.get('startup_memory')
    result['peak_sampled_process_dedicated_bytes']=p.get('peak_process_dedicated_bytes') or None
    result['startup_to_readback_ms']=e.get('startup_to_readback_ms')
    result['settings']=e.get('settings')
    result['readiness_frames']=e.get('readiness_frames')
    result['configured_image_ms']=e.get('configured_image_ms')
    result['shader_reload']=e.get('shader_reload')
    result['timing_scope']=e.get('timing_scope')
    result['folder']=str(folder)
    result['campaign_role']='superseded orientation/exposure case' if folder.parent.name.startswith(('ara-pbr-base-','quarry-base-')) else 'primary / named variant'
    gpu={r['frame_id']:r for r in e.get('gpu',[])}
    result['segments']={}
    for name,indices in SEGMENTS.items():
        rows=[r for r in e['frames'] if r['index'] in indices]
        data={'cpu_service_ms':stats([r.get('cpu_service_ms') for r in rows]),
              'cpu_submit_ms':stats([r.get('cpu_submit_ms') for r in rows])}
        samples=[gpu[r['frame_id']] for r in rows if r.get('frame_id') in gpu]
        data['gpu']={key:stats([s.get(key) for s in samples]) for key in
            ('span_ms','shading_ms','primary_ms','visibility_ms','shadow_generation_ms','post_ms','reconstruction_ms')}
        passes=sorted({t['pass'] for r in rows for t in r.get('passes',[])})
        data['ara_passes']={key:stats([t['ms'] for r in rows for t in r.get('passes',[]) if t['pass']==key]) for key in passes}
        result['segments'][name]=data
    if e.get('path')=='hybrid':
        result['hybrid_ray_pass_observed']=any(t['pass']=='RT.Reflect' for r in e['frames'] for t in r.get('passes',[]))
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--tag',default='baseline')
    p.add_argument('--extra-tag',action='append',default=[])
    a=p.parse_args()
    out=ROOT/'analysis';out.mkdir(exist_ok=True)
    captures=[]
    for tag in [a.tag,*a.extra_tag]:
        for file in sorted((ROOT/'phase2-runs'/tag).glob('*/*/engine.json')):
            folder=file.parent
            record=json.loads((folder/'process.json').read_text())
            if record.get('exit_code')==0 and record.get('source_status_unchanged'):
                captures.append(summarize(folder))
    mark_roles(captures)
    save(out/'performance.json',dict(scope='single frame in flight; service excludes capture/PNG; shared CPU contention is retained with every row',runs=captures))
    regions=json.loads((ROOT/'phase2-scenes/regions.json').read_text())
    quality={}
    for capture in captures:
        if capture['resolution']!=[960,540] or capture['mode']=='reference': continue
        if Path(capture['folder']).name!=capture['mode']: continue
        if capture['campaign_role'].startswith('superseded'):continue
        key=Path(capture['folder']).parent.name
        if capture['scene_sha256']!=digest(ROOT/'phase2-scenes'/f'{key}.json'):
            raise ValueError('candidate capture was made with a different scene identity')
        quality.setdefault(key,{})[capture['mode']]=Path(capture['folder'])
    # References may be acquired in separate short leases/tags.
    references={}
    for file in sorted((ROOT/'phase2-runs').glob('*/*/reference/frame-*.png')):
        sidecar=json.loads(file.with_suffix('.png.json').read_text())
        process_path=Path(sidecar.get('reference_source_process',str(file.parent/'process.json')))
        if not process_path.exists():continue
        process=json.loads(process_path.read_text())
        if process.get('exit_code')!=0 or not process.get('source_status_unchanged'):continue
        references.setdefault(file.parents[1].name,{})[int(file.stem.split('-')[-1])]=str(file)
    comparisons=[]
    for case,modes in quality.items():
        scene=ROOT/'phase2-scenes'/f'{case}.json';spec=json.loads(scene.read_text())
        family='ara-pbr' if case.startswith('ara-pbr') else 'quarry' if case.startswith('quarry') else 'game'
        for ara in ('raster','hybrid','fidelity'):
            if ara not in modes:continue
            rows=[]
            for i,f in enumerate(spec['frames']):
                if not f['capture']:continue
                row=dict(index=i,ara_label=f'Ara {ara}')
                for key,mode in [('ara',ara),('rt','rt'),('nonrt','nonrt')]:
                    if mode in modes and (modes[mode]/f'frame-{i:04}.png').exists():
                        row[key]=str(modes[mode]/f'frame-{i:04}.png')
                if i in references.get(case,{}):row['reference']=references[case][i]
                rows.append(row)
            value=dict(scene=str(scene),scene_sha256=digest(scene),display_transform='AcesFit+sRGB',frames=rows,crops=regions[family])
            manifest=out/f'{case}-{ara}.json';save(manifest,value)
            score(manifest,out/f'{case}-{ara}-scores.json')
            # Keep key witnesses and consecutive motion/cut crops; do not create hundreds of sheets.
            selected=[r for r in rows if r['index'] in (39,40,41,71,72,73,87)]
            small=out/f'{case}-{ara}-sheets.json';save(small,value|dict(frames=selected))
            compose(small,out/'sheets'/f'{case}-{ara}')
            comparisons.append(str(manifest))
    save(out/'index.json',dict(comparisons=comparisons,capture_count=len(captures)))
    print(json.dumps(dict(captures=len(captures),comparisons=len(comparisons),out=str(out))))


if __name__=='__main__': main()
