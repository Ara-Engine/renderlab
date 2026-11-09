"""Build the local, self-contained measured report from immutable evidence.

No publishing. Images remain lossless; the embedded JSON preserves hardware,
source/binary identity, resolution and contention for every reported number.
"""
import base64
import html
import json
import mimetypes
from pathlib import Path
import statistics

import numpy as np
from PIL import Image

from capture import digest
from images import crop_box, sheet
from prepare import ROOT, save

NAMES={'nonrt':'RL non-RT','rt':'RL RT','raster':'Ara raster','hybrid':'Ara hybrid','fidelity':'Ara RT fidelity'}
COLORS={'raster':'#a6afbb','hybrid':'#d5dbe2','fidelity':'#747d89','rt':'#85c3b4','nonrt':'#4c8c7e'}
INLINE_IMAGES={}
PRIMARY=('game-base','ara-pbr-oriented','quarry-daylight')
ASPECTS=[('GI / bounce','gi_bounce'),('Sun shadows','sun_shadow'),('Mirror reflections','mirror'),('Glossy reflections','glossy')]


def esc(v):return html.escape(str(v))
def number(v, digits=3):return 'unavailable' if v is None else f'{v:.{digits}f}'
def table(head,rows):
    return '<div class="table"><table><thead><tr>'+''.join(f'<th>{esc(h)}</th>' for h in head)+'</tr></thead><tbody>'+''.join('<tr>'+''.join(f'<td>{v}</td>' for v in row)+'</tr>' for row in rows)+'</tbody></table></div>'
def details(title,body):return f'<details><summary>{esc(title)}</summary>{body}</details>'
def image(path,caption):
    data=base64.b64encode(path.read_bytes()).decode()
    mime=mimetypes.guess_type(path)[0] or 'image/png'
    url=f'data:{mime};base64,{data}'
    INLINE_IMAGES[url]=path.as_posix()
    return f'<figure><img loading="lazy" src="{url}" alt="{esc(caption)}"><figcaption>{esc(caption)}</figcaption></figure>'
def fragment_chart(runs):
    maximum=max(r['segments']['still']['cpu_service_ms']['median'] for r in runs)
    rows=[]
    for r in runs:
        v=r['segments']['still']['cpu_service_ms']['median'];mode=r['mode']
        rows.append(f'<div class="barrow"><span>{NAMES[mode]}</span><i style="width:{v/maximum*65:.2f}%;background:{COLORS[mode]}"></i><b>{v:.2f} ms</b></div>')
    return '<div class="chart">'+''.join(rows)+'</div>'


def main():
    out=ROOT/'report';out.mkdir(exist_ok=True)
    performance=json.loads((ROOT/'analysis/performance.json').read_text())
    allruns=performance['runs']
    primary=[r for r in allruns if Path(r['folder']).name==r['mode'] and Path(r['folder']).parent.name.replace('-timing','') in PRIMARY and r['resolution']==[1920,1080]]
    supplements={}
    for file in (ROOT/'memory-supplements').glob('*/qualification.json'):
        value=json.loads(file.read_text());record=json.loads(Path(value['process']).read_text())
        original=json.loads(Path(value['original_scene']).read_text())
        replay=json.loads(Path(record['scene']).read_text())
        if record['exit_code'] or not record['source_status_unchanged'] or replay['frames'][:88]!=original['frames']:
            raise ValueError('invalid memory-supplement input/runner identity')
        for key in ('asset','size','to_light','sun_lux','sun_radius','environment','exposure_stops','lights','effects','synchronous'):
            if original.get(key)!=replay.get(key):raise ValueError('memory-supplement configuration changed')
        matches=[r for r in allruns if r['scene_sha256']==value['original_scene_sha256'] and r['mode']==record['mode']]
        if not matches or not any(r['binary_sha256']==record['binary_sha256'] and r['profile']==record['profile']
            and r['asset_sha256']==record['asset_sha256'] and r['hardware']['output']==record['hardware']['output'] for r in matches):
            raise ValueError('memory-supplement binary, hardware or asset differs from original capture')
        case=Path(value['original_scene']).stem.removesuffix('-timing')
        supplements[case+'/'+record['mode']]=dict(qualification=value,process=record)
    quality={}
    for case in (*PRIMARY,'game-no-gi','game-no-reflections'):
        for ara in ('raster','hybrid','fidelity'):
            file=ROOT/'analysis'/f'{case}-quality-{ara}-scores.json'
            if file.exists():quality[(case,ara)]=json.loads(file.read_text())['rows']
    def qr(case,ara,engine,index=39):
        return next((r for r in quality.get((case,ara),[]) if r.get('engine')==engine and r.get('index')==index),None)
    def error(case,ara,engine,region='whole_image',index=39):
        row=qr(case,ara,engine,index)
        return row['linear_display_rgb'][region]['rms'] if row else None
    def verdict(a,b,noise=0):
        if a is None or b is None:return 'Unranked: no qualified reference'
        change=(1-a/b)*100 if b else 0
        if abs(a-b)<=2*noise:return f'Equal within conservative reference-noise bound ({a:.5f} / {b:.5f}; noise {noise:.5f})'
        if abs(change)<2:return f'Equal within 2% descriptive band ({a:.5f} / {b:.5f})'
        return f'{"Better" if change>0 else "Worse"}: {abs(change):.1f}% {"lower" if change>0 else "higher"} error ({a:.5f} / {b:.5f})'
    verdict_rows=[];verdict_json=[]
    for title,region in ASPECTS:
        ar=error('game-base','hybrid','ara',region);rr=error('game-base','hybrid','rt',region)
        an=error('game-base','raster','ara',region);rn=error('game-base','raster','nonrt',region)
        noise=qr('game-base','hybrid','ara').get('reference_noise_linear_rgb',{}).get(region,{}).get('rms',0) if qr('game-base','hybrid','ara') else 0
        pair=[title,verdict(rr,ar,noise),verdict(rn,an,noise),'Game crop; final displayed linear-RGB RMS. Includes other shading in the crop; not an isolated transport score.']
        verdict_rows.append([esc(x) for x in pair]);verdict_json.append(pair)
    temporal=[]
    game_frames=json.loads((ROOT/'phase2-scenes/game-base-quality.json').read_text())['frames']
    moving_indices={i for i,f in enumerate(game_frames) if i>0 and f['phase']=='move'
        and any(f[k]!=game_frames[i-1][k] for k in ('eye','target','fov_y'))}
    for ara,engine in [('hybrid','rt'),('raster','nonrt')]:
        def temporal_error(mode):
            values=[r['screen_residual_change_by_region']['silhouette']['rms'] for r in quality.get(('game-base',ara),[]) if r.get('engine')==mode and r.get('index') in moving_indices and 'screen_residual_change_by_region' in r]
            return statistics.mean(values) if values else None
        noises=[r['screen_residual_noise_by_region']['silhouette']['rms'] for r in quality.get(('game-base',ara),[]) if r.get('engine')=='ara' and r.get('index') in moving_indices and 'screen_residual_noise_by_region' in r]
        temporal.append(verdict(temporal_error(engine),temporal_error('ara'),statistics.mean(noises) if noises else 0))
    verdict_rows.append(['AA / temporal stability',esc(temporal[0]),esc(temporal[1]),'Moving game silhouette: screen-coordinate reference-residual change, mean over consecutive pairs with actual pose changes (the zero-shift move-entry frame is excluded). Spatial error and cuts remain separate. Native resolution; no upscaling verdict.'])
    verdict_json.append(['AA / temporal stability',*temporal])
    limits=[
        ['Local shadows / many lights','Larger configured coverage: all 64 lights eligible for RT visibility','Larger budget: up to 64 atlas lights versus Ara High’s four','Matched 64 point lights. Atlas residency/actual contributing counts were not read back; this is an eligibility-budget comparison, not a calibrated shadow-accuracy winner. Ara’s other 60 lights still illuminate without local shadow maps.'],
        ['Transparency','Matched alpha panel; quality unranked','Matched alpha panel; quality unranked','Same 0.32 alpha, color, geometry and camera. No refractive-glass or multilayer reference; ordinary coverage is not a refraction test.'],
        ['Particles','Unmatched','Unmatched','Ara VFX runtime host and Renderlab emitters use different simulation/composite interfaces; frozen geometry does not exercise either particle runtime. No numeric winner.'],
        ['Sky / fog / atmosphere','Matched controls; quality unranked','Matched controls; quality unranked','Physical-sky models differ. Height fog matches 0.01/m, 90 m scale height, albedo 0.9 and g=0.3. No matched participating-medium reference.'],
        ['Post processing','ACES-fit matched; bloom unranked','ACES-fit matched; bloom unranked','Same manual exposure and named 0.035 bloom control; strength/threshold/filter semantics are not calibrated. DOF, lens effects and motion blur are not matched.'],
        ['Skinned characters','Unmatched','Unmatched','Ara native deforming geometry and Renderlab palette/clip APIs exist. The frozen game hero is rigid PBR geometry; no shared animated character/pose/motion-vector fixture was ported.']]
    verdict_rows += [[esc(x) for x in row] for row in limits];verdict_json+=limits
    save(out/'verdicts.json',dict(scope='measured fixtures only; Better/Worse is reference error, coverage or explicitly named performance, never a universal engine ranking',aspects=verdict_json))
    parts=['<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>ARA1 · Ara and Renderlab measured comparison</title>',
        '''<style>:root{color-scheme:dark}*{box-sizing:border-box}body{margin:0;background:#101214;color:#edf0f2;font:16px/1.6 system-ui,sans-serif}main{max-width:1220px;margin:auto;padding:52px 28px 96px}h1{font-size:42px;line-height:1.14;max-width:900px}h2{font-size:27px;margin-top:56px}h3{margin-top:30px}p{max-width:960px}.muted,figcaption,small{color:#abb2b8}.kicker{color:#85c3b4;font-size:13px;letter-spacing:.1em;text-transform:uppercase}.lead{font-size:20px;color:#d4dbdf}.table{overflow:auto;margin:24px 0}table{border-collapse:collapse;width:100%;font-size:14px;line-height:1.5}td,th{text-align:left;vertical-align:top;padding:12px;border-bottom:1px solid #32383d}th{color:#a6cfc5;font-weight:600}td:first-child{white-space:nowrap}details{border-top:1px solid #32383d;margin:24px 0;padding:16px 0}summary{cursor:pointer;font-size:18px;color:#cddfd9}figure{margin:24px 0}figure img{width:100%;height:auto;border:1px solid #30363b}figcaption{font-size:13px;margin-top:7px}.chart{margin:20px 0 28px}.barrow{display:flex;align-items:center;gap:12px;height:34px;font-size:13px}.barrow span{width:120px;flex:none}.barrow i{display:block;height:14px;min-width:1px}.barrow b{white-space:nowrap;font-weight:500}code{font-size:13px;overflow-wrap:anywhere}a{color:#9dd3c5}.note{border-left:3px solid #85c3b4;padding:6px 18px;background:#171c1f}pre{white-space:pre-wrap;overflow-wrap:anywhere;font-size:12px}section{scroll-margin-top:25px}@media(max-width:650px){main{padding:30px 14px}h1{font-size:31px}td,th{padding:8px}.barrow span{width:95px}}</style><main>''',
        '<div class="kicker">ARA1 · Phase 2 · 9 November 2026 · local review copy</div><h1>Ara versus Renderlab: measured scenes, visible differences and remaining gaps</h1>',
        '<p class="lead">Renderlab has lower displayed-image error against the matched references in all three opaque scenes. Non-RT takes substantially longer per frame. RT is faster on the quarry and slower on the two smaller scenes. The first new shader edit still hitches for 23–24.5 seconds.</p><p>Prioritize non-RT frame cost, new-shader latency, camera-cut spikes and moving mirror glints next. Particle and animated-character comparisons remain unmeasured. These controlled profiles and same-model references do not certify all-feature shipping parity or an overall 2× speed win.</p>',
        '<p>Renderlab renderer source: <code>ae857501ab4792d1889e11111064c3957cc8af44</code>, through comparison merge <code>6d2d8957965b1aafb7cced0eefa1ceb179923577</code>. Ara shipping main: <code>1d2d647f0074f9f1f1d7f69a86f6d71b3142c9e3</code>. Earlier a2c92b1 Renderlab captures are superseded. No renderer code was changed for this report. Ara remained read-only.</p>',
        '<p class="muted">Hardware: NVIDIA RTX 4070 SUPER, 12,282 MiB reported VRAM, driver 617.14; Ryzen 7 7800X3D, 8 cores / 16 threads. Windows. Release builds: Renderlab Rust 1.96, Ara Rust 1.99.1. Quality/reference 960×540; timing 1920×1080. Native internal resolution, one frame in flight. No stock-upscaling or Unreal comparison is claimed.</p>',
        '<nav><a href="#verdicts">Verdicts</a> · <a href="#pictures">Pictures</a> · <a href="#timings">Frame costs</a> · <a href="#startup">Startup and edits</a> · <a href="#method">Method and limits</a></nav>',
        '<section id="verdicts"><h2>Per-aspect verdicts</h2><p>“Better” and “worse” below mean lower or higher reference RMS on the named game crop. They do not prove an isolated GI or reflection algorithm is superior. The 2% equality band is descriptive, not a statistical confidence interval; finite-reference noise is reported separately.</p>',
        table(['Aspect','Renderlab RT vs Ara hybrid','Renderlab non-RT vs Ara raster','Scope / limit'],verdict_rows),'</section>',
        '<section id="pictures"><h2>The same cameras and content</h2><p>Columns are always Ara | Renderlab RT | Renderlab non-RT | path-traced reference. PNGs are embedded without resampling or exposure fitting. The page scales sheets for reading; open the HTML image at its native extent to inspect pixels.</p>']
    labels={'game-base':'Frozen game: standard PBR geometry, no animation or decals','ara-pbr-oriented':'Ara PBR: canonical winding, original textures and factors','quarry-daylight':'Quarry: full-detail asset, neutral sky, common −8.5 stop exposure'}
    for case in PRIMARY:
        parts.append(f'<h3>{esc(labels[case])}</h3>')
        path=ROOT/'analysis/sheets'/f'{case}-quality-hybrid/frame-0039.png'
        if path.exists():parts.append(image(path,f'{labels[case]} · still frame 39 · Ara hybrid'))
        extras=[]
        for mode in ('raster','fidelity'):
            path=ROOT/'analysis/sheets'/f'{case}-quality-{mode}/frame-0039.png'
            if path.exists():extras.append(image(path,f'{labels[case]} · Ara {mode}, same Renderlab/reference panels'))
        parts.append(details('Ara raster and opt-in RT fidelity', ''.join(extras)))
        if case!='game-base':
            sequence=[]
            for index in (71,72):
                p=ROOT/'analysis/sheets'/f'{case}-quality-hybrid/frame-{index:04}.png'
                if p.exists():sequence.append(image(p,f'{case}: {"late camera movement" if index==71 else "first cut"}, frame {index}'))
            parts.append(details('Late camera motion and first cut',''.join(sequence)))
    rows=[]
    for case in PRIMARY:
        for mode in ('raster','hybrid','fidelity'):
            for engine,label in [('ara',NAMES[mode]),('rt',NAMES['rt']),('nonrt',NAMES['nonrt'])]:
                if mode!='hybrid' and engine!='ara':continue
                row=qr(case,mode,engine)
                if not row:continue
                rows.append([esc(case),label,number(row['linear_display_rgb']['whole_image']['rms'],5),
                    number(row['linear_display_rgb']['whole_image']['p99'],5),number(error(case,mode,engine,index=72),5),number(error(case,mode,engine,index=87),5)])
    parts += [table(['Scene','Renderer','Still RMS','Still |error| p99','First-cut RMS','Recovered-cut RMS'],rows),
        '<p class="muted">Errors use displayed linear RGB in [0,1], after common ACES-fit/sRGB. RMS includes all geometry, shadows, material/filtering and temporal differences. It is not calibrated HDR irradiance error. Sky pixels can dominate whole-image averages; crops and complete images are both required.</p>']
    cropfig=[]
    for region in ('gi_bounce','sun_shadow','mirror','glossy','silhouette'):
        path=ROOT/'analysis/sheets/game-base-quality-hybrid'/f'frame-0039-{region}.png'
        if path.exists():cropfig.append(image(path,f'Game {region} · fixed world-position crop · still frame 39'))
    parts.append(details('Per-aspect still crops',''.join(cropfig)))
    moving_verdicts=[]
    for title,region in ASPECTS:
        for index,phase in [(71,'late movement'),(72,'first cut')]:
            row=qr('game-base','hybrid','ara',index)
            noise=row.get('reference_noise_linear_rgb',{}).get(region,{}).get('rms',0) if row else 0
            moving_verdicts.append([title,f'{phase}, frame {index}',
                esc(verdict(error('game-base','hybrid','rt',region,index),error('game-base','hybrid','ara',region,index),noise)),
                esc(verdict(error('game-base','raster','nonrt',region,index),error('game-base','raster','ara',region,index),noise))])
    parts.append(details('Per-aspect quality during movement and the first cut', '<p>Same preregistered screen crops and reference-noise rule as the static table. Crops do not track objects, so these are view-dependent diagnostics, not a controlled per-object temporal score.</p>'+table(['Aspect','Phase','RL RT vs Ara hybrid','RL non-RT vs Ara raster'],moving_verdicts)))
    parts.append('<p class="note">Visible misses remain despite lower aggregate error. At late-pan frame 71, both Renderlab paths show a box-shaped sun glint in the mirror; the reference has a small point. The mirror crop rises from about 0.042 RMS while still to 0.069 during that pan. The alpha panel also produces a blue ground-shadow response in Renderlab that the Ara blend panel does not match. Sky/horizon distributions differ. These effect differences lack an independent calibrated reference, so no accuracy winner is assigned to them.</p>')
    ablations=[]
    for ara,engine,label in [('hybrid','ara','Ara hybrid'),('raster','ara','Ara raster'),('hybrid','rt','RL RT'),('hybrid','nonrt','RL non-RT')]:
        ablations.append([label,number(error('game-base',ara,engine,'gi_bounce'),5),number(error('game-no-gi',ara,engine,'gi_bounce'),5),
            number(error('game-base',ara,engine,'mirror'),5),number(error('game-no-reflections',ara,engine,'mirror'),5)])
    parts.append(details('Quality ablations against the same physical target', '<p>Only GI/reflection estimator switches differ. Reference reuse checks every physical/display input and preserves source identities; changing geometry, media, lights, camera or display controls rejects reuse. These are crop RMS values, not isolated irradiance or visibility errors.</p>'+table(['Renderer','Bounce: GI on','Bounce: GI off','Mirror: reflections on','Mirror: reflections off'],ablations)))
    motion=[]
    for index in (40,41,71,72,73,87):
        path=ROOT/'analysis/sheets/game-base-quality-hybrid'/f'frame-{index:04}.png'
        if path.exists():motion.append(image(path,f'Game frame {index}: {"camera motion" if index<72 else "cut / recovery"}'))
    parts.append(details('Camera motion, first cut and recovery',''.join(motion)))
    featurefig=[]
    for variant in ('alpha','lights','lights-unshadowed','atmosphere','fog','post'):
        path=ROOT/'analysis/sheets'/f'game-{variant}-quality-hybrid/frame-0039.png'
        if path.exists():featurefig.append(image(path,f'Game {variant} · matched controls, no qualified reference for this effect'))
        region={'alpha':'alpha_overlap','lights':'light_overlap','lights-unshadowed':'light_overlap','atmosphere':'sky_horizon','fog':'sun_shadow','post':'glossy'}[variant]
        crop=path.with_name(f'frame-0039-{region}.png')
        if crop.exists():featurefig.append(image(crop,f'{variant}: {region} crop · same panel order'))
        for index in (71,72):
            crop=path.with_name(f'frame-{index:04}-{region}.png')
            if crop.exists():featurefig.append(image(crop,f'{variant}: {region}, {"late camera movement" if index==71 else "first cut"} (frame {index})'))
    parts += [details('Transparency, lights, atmosphere, fog and bloom',''.join(featurefig)),'</section>',
        '<section id="timings"><h2>1080p still, motion and cut costs</h2><p>CPU service includes render submission and a completed GPU drain; PNG encoding/readback is outside frame service. These are single-frame latency measurements, not pipelined interactive throughput or presented FPS. Medians use eight still frames (32–39), eight moving frames (64–71), and eight recovered-cut frames (80–87). The first cut is one observed frame, not a distribution. Ara pass sums are never presented as an end-to-end GPU span.</p>']
    timingrows=[]
    for case in PRIMARY:
        runs=[r for r in primary if Path(r['folder']).parent.name==case+'-timing']
        if not runs:continue
        parts.append(f'<h3>{esc(case)} · still service time</h3>'+fragment_chart(runs))
        for r in runs:
            s=r['segments'];gpu=s['still']['gpu'].get('span_ms')
            memory=r['peak_sampled_process_dedicated_bytes'];supplement=supplements.get(case+'/'+r['mode']) if not memory else None
            if supplement:memory=supplement['qualification']['dedicated_bytes']
            timingrows.append([esc(case),NAMES[r['mode']],number(s['still']['cpu_service_ms']['median']),number(s['move']['cpu_service_ms']['median']),
                number(s['cut_first']['cpu_service_ms']['median']),number(s['cut_settled']['cpu_service_ms']['median']),
                number(gpu['median'] if gpu else None),number(memory/2**20 if memory else None,1)+(' †' if supplement else '')])
    parts.append(table(['Scene','Renderer','Still ms','Moving ms','Cut ms','Recovered ms','Still GPU span ms','Sampled dedicated MiB'],timingrows))
    if supplements:parts.append('<p class="muted">† Dedicated memory comes from a separate same-configuration run with the original 88 poses followed by a stationary 2,048-frame tail, to keep short processes alive long enough for WDDM sampling. Original timing/startup observations remain unchanged. This also applies to marked feature rows. Supplemental scene/process identities and hardware are embedded in provenance.</p>')
    parts.append('<p>GI-enabled game variants expose another latency gap: Renderlab RT first-cut service is roughly 21–31 ms, versus about 2–4 ms for Ara hybrid. With 64 local lights, still service is 11.99 / 27.86 ms for Renderlab RT / non-RT versus 3.70 / 3.26 ms for Ara hybrid / raster, with different local-shadow budgets. With local shadows disabled on both sides it is 9.97 / 15.59 ms versus 3.53 / 3.15 ms; those remain whole-frame costs.</p>')
    passsections=[]
    for r in primary:
        passrows=[];segments=r['segments']
        if r['mode'] in ('rt','nonrt'):
            keys=[k for k,v in segments['still']['gpu'].items() if v]
            for key in keys:passrows.append([esc(key),*[number(segments[s]['gpu'][key]['median']) if segments[s]['gpu'].get(key) else 'unavailable' for s in ('still','move','cut_first')]])
        else:
            for key in segments['still']['ara_passes']:
                passrows.append([esc(key),*[number(segments[s]['ara_passes'][key]['median']) if key in segments[s]['ara_passes'] else 'not scheduled' for s in ('still','move','cut_first')]])
        passsections.append(details(f'{Path(r["folder"]).parent.name} / {NAMES[r["mode"]]}',table(['Pass','Still ms','Move ms','First cut ms'],passrows)))
    parts.append(details('Available GPU pass timings',''.join(passsections)))
    feature_rows=[]
    for r in allruns:
        case=Path(r['folder']).parent.name
        if r['resolution']!=[1920,1080] or not case.startswith('game-') or case=='game-base-timing' or Path(r['folder']).name!=r['mode']:continue
        base=next((b for b in primary if Path(b['folder']).parent.name=='game-base-timing' and b['mode']==r['mode']),None)
        s=r['segments'];value=s['still']['cpu_service_ms']
        memory=r['peak_sampled_process_dedicated_bytes'];supplement=supplements.get(case.removesuffix('-timing')+'/'+r['mode']) if not memory else None
        if supplement:memory=supplement['qualification']['dedicated_bytes']
        if value:feature_rows.append([esc(case),NAMES[r['mode']],number(value['median']),number(s['move']['cpu_service_ms']['median']),number(s['cut_first']['cpu_service_ms']['median']),number(value['median']-base['segments']['still']['cpu_service_ms']['median'] if base else None),
            number(memory/2**20 if memory else None,1)+(' †' if supplement else ''),number(r['launch_to_ready_file_ms']/1000 if r['launch_to_ready_file_ms'] else None)])
    parts.append(details('Feature variants: whole-frame service, memory, startup and toggle deltas', '<p>Toggle deltas include scheduling, cache, visibility and shader-variant changes; they are not isolated per-pass costs. Lights, fog and bloom alter the workload, so only GI/reflection off variants are ablations of an otherwise identical target. Some feature leases overlapped CPU image scoring/compositing; primary 1080p timing campaigns did not.</p>'+table(['Case','Renderer','Still ms','Moving ms','Cut ms','Still minus base ms','Dedicated MiB','Launch → first s'],feature_rows)))
    parts += ['<p class="muted">WDDM GPU Process Memory / Dedicated Usage is sampled, not an allocator total or a residency guarantee. Board-wide NVIDIA memory is retained separately. Missing/zero counters are unavailable; they are never interpreted as zero VRAM. Full distributions, settings, binary hashes, hardware and contention are embedded in the provenance below.</p></section>',
        '<section id="startup"><h2>Startup, shader edits and Rust edits</h2><p>Startup is process launch to observed first GPU readback, before PNG encode, not display scanout. Primary 1080p runs use fresh per-run application cache directories with a warm system/driver; they are not clean-driver cold-start measurements. Ara’s earliest image can be raster fallback or fading GI. Its configured image waits for GI arrival and RT pipelines; acquisition is a separate process.</p>']
    startrows=[]
    for r in primary:
        startrows.append([esc(Path(r['folder']).parent.name),NAMES[r['mode']],number(r['launch_to_ready_file_ms']/1000 if r['launch_to_ready_file_ms'] else None),
            number(r.get('configured_image_ms')/1000 if r.get('configured_image_ms') else None),number(r['startup_memory'].get('peak_working_set_bytes',0)/2**30,2),number(r['startup_memory'].get('peak_pagefile_bytes',0)/2**30,2)])
    parts.append(table(['Case','Renderer','Launch → earliest s','Engine → configured s','Startup peak working set GiB','Peak process commit GiB'],startrows))
    warm=[r for r in allruns if Path(r['folder']).name.startswith('warm-')]
    warmrows=[]
    for mode in ('nonrt','rt','raster','hybrid'):
        rows=[r for r in warm if r['mode']==mode]
        if not rows:continue
        launches=[r['launch_to_ready_file_ms']/1000 for r in rows]
        edits=[r['shader_reload']['edit_to_readback_ms'] for r in rows if r.get('shader_reload')]
        compiles=[r['shader_reload']['compile_ms'] for r in rows if r.get('shader_reload')]
        warmrows.append([NAMES[mode],str(len(rows)),number(statistics.median(launches)),f'{min(launches):.3f}–{max(launches):.3f}',number(edits[0]/1000 if edits else None),f'{min(edits[1:]):.1f}–{max(edits[1:]):.1f}' if len(edits)>1 else 'unavailable'])
    parts.append(table(['Renderer','n','Warm startup median s','Range s','First new WGSL edit → image s','Cached edit repeats ms'],warmrows))
    parts.append('<p class="note">A new shader variant still has a large hitch: the first material edit takes about 23–24.5 seconds. Cached repeats are about 109–126 ms. These cache states must not be pooled into a reassuring median.</p><p>Renderlab’s edit changes the live standard material’s red output by 17%, invokes the public transactional reload API, renders and reads back. Each edit visibly changed the output. Ara shader edit-to-image is <strong>unavailable</strong>: no genuine renderer-shader edit was made under the read-only policy. A cached startup is not relabelled a shader edit.</p>')
    loops=[]
    for engine in ('renderlab','ara'):
        file=ROOT/'devloops'/engine/'result/result.json'
        if file.exists():
            value=json.loads(file.read_text());loops.append([engine,number(value.get('edit_to_ready_ms')/1000 if value.get('edit_to_ready_ms') else None),esc(value['recipe']['scope'])])
    parts.append(table(['Engine','Adapter Rust edit → image s','Measured scope'],loops))
    parts.append('<p>Rust values are <strong>adapter-edit proxies</strong>: a real staged Rust exposure edit (+0.125 stops), Cargo rebuild/relink, process restart and first GPU readback. Core renderer sources are unchanged. Neither number certifies a core Rust edit-to-viewer loop. Genuine renderer-source edits remain unavailable under this measurement-only policy.</p>')
    bake=[]
    for file in sorted((ROOT/'bakes').glob('p2-*-normal/bake.json')):
        v=json.loads(file.read_text());bake.append([esc(file.parent.name),number(v['seconds']),str(v['probes']),number((file.parent/'probes.json').stat().st_size/2**20,2)])
    parts.append(details('Ara acquisition costs (charged separately)',table(['Cache','Acquire / bake s','Probes','Retained surface MiB'],bake)))
    parts += ['</section><section id="method"><h2>What is matched, and what is not</h2>',
        '<p>All primary rows use the same RLA bytes, placements, PBR factors, textures, camera rays, sun/lights and exposure. Ara native deduplicates identical keyed geometry to satisfy its admission contract and consumes the same uncompressed mip chain; Renderlab uses its public prepared-asset loader. Startup/memory compare these adapter paths, not the original full-game host. Ara’s public fidelity API flattens instances and regenerates texture mips from level zero; its startup, memory and texture differences stay separate. Uniform environment, a 0.00465 rad sun, 0.1 m camera near plane, fixed exposure, ACES-fit and sRGB are shared. Ara uses an infinite reversed-Z far plane; Renderlab uses 10 km.</p>',
        '<p>The game is a frozen opaque PBR subset of <code>examples/game_scene.rs</code>: no toon hero, decals, rigid animation, VFX, skinning or medium. Effects are additional explicitly named variants. Ara PBR retains Avocado and its authored texture/tangent data, plus the original roughness/metallic sphere grid. The reference subset omits the authored sample?s three local lights on both engines and uses sun/uniform sky; the shared reference API supports sun/sky, with local lighting exercised separately by the 64-light fixture. Cyan emissive and blended spheres are excluded. The export’s 22,083 reversed triangles were normalized against authored normals on both engines; 1,641 triangles were kept. Original uncorrected PBR captures are rejected for primary verdicts. Quarry uses the original full-detail private scan, not decimated geometry; the old HDR-sky exposure was superseded by a common −8.5 stops after neutral-sky substitution. No per-engine exposure fitting was performed.</p>',
        '<p>Ara main raster/hybrid use High shading budgets at native resolution (four 2048 cascades, 1024 VSM pages / 32 updates, 2048-probe GI runtime budget, four 512-face local shadow lights). Hybrid adds reflection queries below roughness 0.3 and rough occlusion to 0.6. Non-RT raster runtime uses no RT queries, but its probe acquisition here was performed with RT and charged separately. Automatic local reflection captures are not baked. Opt-in fidelity uses its separate DDGI/reservoir path. These are controlled native-resolution profiles, not stock High’s 0.6-axis reconstruction. Renderlab uses final-base default GI, TR1, reflection and local-shadow algorithms, with explicitly named neutral-scene sky/media/post controls. Non-RT gets the same feature targets.</p>',
        '<p>References use the existing plain path tracer: 16,384 samples/pixel for the game, 4,096 for PBR/quarry, next-event estimation, MIS, Russian roulette and depth limit 1024; no temporal reuse or denoising. Its transport was cross-checked against independent Mitsuba fixtures in existing U0 evidence; this campaign is not a new independent material-model cross-check. The reference uses Renderlab’s standard Lambert/GGX BSDF, so error rankings have a same-model bias. Split halves provide finite-sample noise estimates, not proof of convergence or a film-quality certificate. Rare bright paths keep the game reference noisy even at 16k samples. No old HDR quarry reference was reused.</p>']
    inventory=[
        ['GI','Acquired probe surfaces + runtime relight on raster/hybrid; DDGI / reservoir gather in fidelity','Opt-in patch transport / event solver','World radiance cache, hardware rays or software BVH; final G3 filter/refine'],
        ['Sun / local shadows','VSM + cascades; four High local maps; fidelity ray visibility','Opt-in sun-page service / filtering experiments','RT visibility; non-RT cached virtual sun and local atlas, up to 128 lights'],
        ['Mirror / glossy','Raster SSR + probe/environment estimates; hybrid ray queries; fidelity gather','Opt-in software secondary transport, separate temporal constraints','Hardware reflections or software BVH, final R1n non-RT counterpart'],
        ['AA','High TSR; fidelity TAAU','Experimental software-reflection primary can bypass normal temporal path','Native TR1 measured; final TR2 upscale interpolation present but not tested here'],
        ['Transparency / VFX','Forward blend/transmission; separate ara-vfx-gpu runtime','Ordered software glass experiments; inherited VFX runtime','Programmable coverage/transmittance, GPU particles; alpha subset only matched'],
        ['Atmosphere / fog','Physical sky, height/froxel fog','Additional media work, not promoted by this report','Atmosphere, aerial perspective, height and volumetric fog on both paths'],
        ['Many lights','256 scene / 128 tile limits; separate local-shadow budget','G1 finite-range grid experiment has no production caller','Clustered point/spot evaluation and local-shadow budgets'],
        ['Post','Exposure, grading, ACES-fit, bloom and lens options','Shipping look plus experiments','Bloom, tone mapping, grading, blur/DOF/lens controls; bloom subset matched'],
        ['Skinning','Native deforming geometry revisions, current/previous pose; changed RT BLAS rebuilds','Inherited native geometry path','Palette/clip and GPU deformation APIs; no common animated fixture measured']]
    parts.append(details('Renderer inventory: main, render-v3 and final Renderlab',table(['Aspect','Ara shipping main','Ara render-v3 (unmeasured)','Renderlab ae85750'],[[esc(v) for v in row] for row in inventory])+'<p>Primary source paths: Ara <code>crates/ara-render/src/{look,renderer,instanced}</code>, <code>ara-render-fidelity/src/{presets,settings,view}.rs</code>, <code>ara-vfx-gpu</code>; render-v3 <code>look/{transport,reflect,lights}</code>; Renderlab <code>src/engine/settings.rs</code>, <code>src/{gi_cache,reflections,skinning,post}.rs</code> and final-base gap documents. See repository <code>docs/m7-ara1-plan.md</code> for the detailed source inventory. Historical HTML reports are secondary context, not current measurements.</p>'))
    noise=[]
    for case in PRIMARY:
        manifest=ROOT/'analysis'/f'{case}-quality-hybrid.json'
        for row in json.loads(manifest.read_text())['frames'] if manifest.exists() else []:
            if not row.get('reference'):continue
            v=json.loads(Path(row['reference']+'.json').read_text())
            if v['frame_index'] in (39,41,72,73,87):noise.append([esc(case),str(v['frame_index']),str(v['samples']),number(v['split_half_mean_noise_rms_codes'],4),str(v['invalid_pixels'])])
    parts.append(details('Reference split-half noise and invalid pixels',table(['Case','Frame','Samples','Mean-image noise RMS (8-bit codes)','Invalid pixels'],noise)))
    parts += ['<p>Quality and timing are different passes. Initial diagnostic quality captures used the lease launcher’s injected Vulkan validation and Renderlab logged SPIR-V layout diagnostics. Primary game quality was then recaptured without instrumentation, as were corrected PBR/quarry, effect, release performance and startup captures; effective environments are recorded. Renderer code was not changed to suppress diagnostics, and release captures do not certify validation-clean rendering. References briefly overlapped CPU asset-inspection builds; reference-generation wall time is not a comparative engine performance claim. One 16k reference batch was rejected when a harness-test edit changed whole-checkout status; it was rerun successfully with a renderer/build-input mutation check. Ara always retains the stricter entire-checkout check. Rejected references and their aliases are excluded.</p>',
        '<p>Root’s held window completed the deferred startup checks and the exact release line was written. All later GPU work used leases of ten minutes or less with the 2 GiB available-RAM guard. Ara process checks ran before every Ara invocation. Root subsequently assigned this campaign the sole GPU; ordinary OS activity remains. The JSON for each run retains source/binary/scene hashes, resolution, hardware, sampled memory, cache state, lease/order and contention. Old G1–G6/B3 evidence is unavailable on this machine and supplies no current numbers. Render-v3 experiments were inventoried/typechecked, not measured or silently substituted for shipping main.</p>',
        '<p class="note">This evidence can guide the next fixes, but it does not certify all-feature shipping parity, a 2× speed win, Unreal equivalence or film quality. Missing fixtures are particles, skinned animation, refractive/multilayer glass, full authored worlds, moving local lights, core source edit loops, native HDR radiometric scores and stock upscaling presets. The comparison intentionally changes no renderer implementation.</p>']
    provenance=dict(performance=performance,reference_sources=[str(p) for p in (ROOT/'phase2-runs').glob('*/*/reference/engine.json')],
        binary_identities=json.loads((ROOT/'bin/final/identities.json').read_text()),sources=json.loads((ROOT/'bin/final/sources.json').read_text()),
        regions=json.loads((ROOT/'phase2-scenes/regions.json').read_text()),verdicts=verdict_json)
    provenance['devloops']={engine:{'result':json.loads((ROOT/'devloops'/engine/'result/result.json').read_text()),
        'capture':json.loads((ROOT/'devloops'/engine/'capture/process.json').read_text())} for engine in ('renderlab','ara') if (ROOT/'devloops'/engine/'result/result.json').exists()}
    provenance['registered_scenes']={f.stem:json.loads(f.read_text()) for f in (ROOT/'phase2-scenes').glob('*.json') if f.name not in ('identities.json','regions.json')}
    sizes_path=ROOT/'target-sizes-phase2.json'
    target_sizes=json.loads(sizes_path.read_text(encoding='utf-8-sig')) if sizes_path.exists() else []
    provenance['target_cleanup']=target_sizes
    provenance['memory_supplements']=supplements
    if target_sizes:
        retained=sum(r['bytes'] for r in target_sizes if not r['removed'])/2**30
        parts.append(f'<p class="muted">Cleanup gap: {retained:.2f} GiB of owned Cargo targets remains. Automatic approval review rejected recursive cleanup (including a verified literal path) with “blocked by policy.” Sizes and exact paths are preserved in target-sizes-phase2.json. No disposable raw measurement dump above 5 MiB was found in the capture/lease/CPU-verification directories; prepared assets and bake inputs are retained separately.</p>')
    save(out/'provenance.json',provenance)
    parts.append(details('Embedded machine-readable provenance', '<pre>'+esc(json.dumps(provenance,indent=2))+'</pre>'))
    parts.append('<p class="muted">Local evidence root: D:/Coding/renderlab-work/ara1/. This page is a local review artifact; root controls publication. Avocado model/textures: Microsoft; original 2017 Public / CC0 1.0 notice retained in Ara?s examples/assets/Avocado-LICENSE.md, sourced from Khronos glTF Sample Assets. Source and private scan redistribution rights are not granted by this report.</p></section></main></html>')
    markup=''.join(parts)
    target=out/'index.html';target.write_text(markup,encoding='utf-8')
    # A small view of the same page lets T3 inline local images without sending
    # megabytes of base64 through command output. The deliverable above stays portable.
    inline=markup
    for url,path in INLINE_IMAGES.items():inline=inline.replace(url,path)
    start=inline.index('<details><summary>Embedded machine-readable provenance')
    end=inline.index('</details>',start)+len('</details>')
    inline=inline[:start]+'<p class="muted">Full machine-readable provenance is embedded in the standalone index.html.</p>'+inline[end:]
    inline=inline.replace('</style>','body{background:transparent;color:var(--foreground);font-family:var(--font-sans)}main{padding:0;margin:0;max-width:none}</style>')
    (out/'inline.html').write_text(inline,encoding='utf-8')
    def mdtable(headers,rows):
        return '| '+' | '.join(headers)+' |\n| '+' | '.join('---' for _ in headers)+' |\n'+''.join('| '+' | '.join(str(v).replace('|','/') for v in row)+' |\n' for row in rows)
    summary='''# M7 ARA1: measured Ara / Renderlab comparison

Phase 2, 2026-11-09, branch `t3/m7-ara1`. **Renderlab has lower displayed-image error against the matched references in all three opaque scenes; performance is mixed and non-RT is substantially slower.** RT is faster on the quarry and slower on the smaller scenes. New-shader latency remains a 23–24.5 s hitch. Prioritize those latency costs and the missing particle/animated-character fixtures next. Same-model references and incomplete feature coverage prevent a universal quality/speed or shipping-parity certificate.

Visible misses: both Renderlab paths show a box-shaped mirror-sun glint during the late pan (frame 71); mirror-crop RMS rises from about 0.042 still to 0.069 in that view. Alpha-panel ground-shadow response and sky/horizon distributions differ between engines. GI-enabled game variants also have roughly 21–31 ms RT first-cut service versus about 2–4 ms for Ara hybrid. These remain gaps even where the aggregate reference error is lower. The page includes per-aspect late-motion/cut error verdicts and crops.

Local self-contained page: `D:/Coding/renderlab-work/ara1/report/index.html` (embedded lossless images and provenance). `inline.html` is the small local view used to show the same content in T3; `provenance.json` and `verdicts.json` are alongside. Root owns publication; nothing was uploaded.

## Frozen sources and context

- Renderlab renderer: **`ae857501ab4792d1889e11111064c3957cc8af44`**, merged into this comparison branch at `6d2d8957965b1aafb7cced0eefa1ceb179923577`. `src/`, `shaders/`, `crates/` and `viewer/` match that final base. Earlier a2c92b1 Renderlab captures are superseded. Only measurement adapters/tools/docs changed here.
- Ara shipping main: `1d2d647f0074f9f1f1d7f69a86f6d71b3142c9e3`, read-only checkout `D:/Coding/renderlab-work/ara1/ara-main`. Ara render-v3 `1e04fe5dd9d04ede28592597dace9c0c06a15f9a` was inventoried/preparation-typechecked, **not GPU measured** or substituted for shipping main. No Ara worktree files were edited, committed or pushed.
- RTX 4070 SUPER, 12,282 MiB reported VRAM, NVIDIA 617.14; Ryzen 7 7800X3D, 8 cores / 16 threads; Windows. Optimized release profiles, Renderlab Rust 1.96 and Ara Rust 1.99.1. Native resolution: quality/reference **960×540**, timings **1920×1080**. No stock-upscaling or Unreal comparison.
- Each capture records binary/source/harness/scene/asset hashes, settings, resolution, hardware, cache state, lease/order and contention. Root assigned the sole GPU to ARA1 for final work. Primary timing samples ran without parallel ARA1 builds/scoring; some feature leases overlapped CPU image scoring. Ordinary OS activity remains. See embedded per-run provenance rather than treating these values as portable engine constants.

## Matched content

| Scene | What is shared | Correction / limit |
| --- | --- | --- |
| Frozen game | Same RLA geometry/materials/placements from `examples/game_scene.rs`, camera, sun/sky and exposure | Standard PBR replaces toon hero; no decals, rigid animation, VFX or skinning. Alpha, 64 lights, atmosphere, fog and bloom are separate named variants. |
| Ara PBR | Original common-sample sphere grid and Avocado, texture/tangent data, factors, camera and lighting | Shared canonical winding: 22,083 triangles flipped against authored normals, 1,641 kept. CPU roundtrip is byte-idempotent. Original inward procedural winding produced a false brightness discrepancy and is rejected from primary verdicts. Cyan emissive/blended spheres and studio HDR are excluded. |
| Quarry | Original full-detail private RLA, same camera, neutral sky/sun | A common **−8.5 stop** exposure replaces the nearly black inherited HDR-sky exposure after uniform-sky substitution. No per-engine exposure fit or decimation. Original dark case is superseded. |

Both native engines consume the same RLA bytes and mip chains. Ara fidelity's public API flattens instances and regenerates mips from level zero; keep its startup/memory/filtering differences separate. Uniform environment, 0.00465-rad sun, 0.1 m near plane, fixed exposure, ACES-fit and sRGB are shared. Ara's infinite far plane versus Renderlab's 10 km far plane preserves camera rays. Ara High shading budgets at native resolution are a controlled profile, not stock High's 0.6-axis reconstruction.

## Per-aspect verdicts

The numerical better/worse statements are **displayed linear-RGB RMS on preregistered game crops**, not isolated algorithm scores or all-scene shipping certificates. They include shading/filtering/history differences. The same-model reference bias and split-half uncertainty apply; a descriptive 2% band is not a statistical confidence interval.

'''+mdtable(['Aspect','RL RT vs Ara hybrid','RL non-RT vs Ara raster'],[row[:3] for row in verdict_json])+'''
## Frame time and memory

Single frame in flight. CPU service includes submission plus GPU drain, excludes image readback/PNG. Still/moving/recovered medians use eight samples each; first cut is one observation. Ara GPU pass sums are not an end-to-end span. Full per-pass still/move/cut costs, distributions and per-feature ablations are in the page/provenance.

'''+mdtable(['Scene','Renderer','Still ms','Moving ms','First cut ms','Recovered ms','GPU span ms','Dedicated MiB'],timingrows)+'''
VRAM is sampled per-process WDDM Dedicated Usage, separate from board memory and allocator estimates. Missing/zero short-process counters remain unavailable. Startup working-set and peak process-commit observations are separate from GPU memory; the page records each engine/case peak. The RT quarry path can be faster while the small scenes are slower; non-RT is substantially slower than Ara raster. No 2× overall performance gate is claimed.

† Memory-only supplemental runs keep short processes alive with 2,048 stationary tail frames after the same first 88 poses/settings. Their sampled memory is marked separately and does not replace any timing/startup observation. This is synchronous one-frame latency, not pipelined throughput or presented FPS. Ara native deduplicates keyed geometry; Renderlab uses the prepared-asset loader, so startup/memory describe these adapter paths.

## Startup and editing

Fresh per-run application-cache directories with a warm driver are not clean-driver cold starts. Startup is process launch to first readback, not scanout. Ara's earliest image can be a loading fallback; its configured-image time waits for GI/RT readiness. RT acquisition/baking is a separately charged preparation step, even for Ara's non-RT raster runtime.

'''+mdtable(['Renderer','Warm n','Warm startup median s','Startup range s','First WGSL edit→image s','Cached repeats ms'],warmrows)+'''
The WGSL edit visibly reduces the live standard material's red output by 17%. **The first new edit still hitches for about 23–24.5 s; cached repeats must not hide it in a median.** Ara shader edit-to-image is unavailable under the read-only policy; cached startup is not an edit measurement.

'''+mdtable(['Engine','Rust adapter edit→image s','Scope'],loops)+'''
Rust rows are real staged exposure edits (+0.125 stops), warm Cargo rebuild/relink, restart and fresh image. They are **adapter proxies**, not core-renderer source edit measurements or interactive viewer/scanout latency. All staged source is under the evidence root; Ara renderer source is unchanged.

## Reference and audit limits

The existing plain tracer uses 16,384 spp for game and 4,096 for corrected PBR/quarry, NEE/MIS/Russian roulette, depth limit 1,024, no reuse/denoising. References cover consecutive motion and cut witnesses. Zero invalid pixels is necessary, not a convergence certificate. Rare bright paths keep finite-reference noise visible even at 16k. Existing independent Mitsuba transport cross-checks are secondary validation; this campaign uses Renderlab's standard Lambert/GGX material model and therefore has same-model bias. No old HDR quarry reference is reused.

GI/reflection-off references are reused only after exact physical/display identity checks that exclude documentary fields and those estimator switches; source process/image identities remain in sidecars. Changes to camera, size, sun, media or other physical controls fail the CPU reuse contract. One high-sample batch was rejected when a harness-test edit changed whole-checkout status; it was rerun successfully with renderer/build-input checks. Rejected data/aliases are excluded. Ara retains an entire-checkout mutation check.

Initial validation-instrumented logs contain Renderlab SPIR-V layout diagnostics. Primary game quality was recaptured without instrumentation; corrected scenes, features and performance use recorded uninstrumented release environments. No renderer fix or validation-clean certificate is claimed.

## Gaps

- Particles and animated skinned characters: both engines have counterparts, but no common runtime host/animated fixture was ported. Frozen hero geometry does not exercise either feature. This is unmeasured coverage, not evidence a feature is absent.
- Refractive/multilayer glass, physical atmosphere/fog and bloom accuracy: controls/images/timings are matched where stated, but no qualified effect reference or calibrated look-equivalence exists. Do not force a quality winner from different models.
- Local shadows: compare actual configured budgets (RL up to 128 maps / all contributing RT lights; Ara High four 512-face maps). Larger eligibility is not a readback of atlas residency or a calibrated visibility score. All 64 lights are present on both sides.
- Genuine Ara shader and core-source Rust edit loops; native HDR radiometric scores; stock upscaling; full authored Ara worlds; moving lights/geometry beyond camera motion; independent full-scene material-model references remain open.
- Historical `D:/AraBuild/render-v3/{README.md,B3-report.md,gates/G1-G6}` evidence is unavailable. Reports/source in Ara are secondary context, not reused current numbers.

## Execution and reproduction

Root's held startup window completed and the exact `m7-ara1 HELD WINDOW done` line was appended. Admission required 8 GiB available for held startup; the 2 GiB runtime guard remained active. Subsequent leases are capped at **10 minutes**, reread the latest anchored root order and poll in-process at 50 ms. Every Ara invocation checks for existing Ara/snengine renderer processes. All output is headless, under `D:/Coding/renderlab-work/ara1/`; no foreground windows, desktop/game captures or input automation.

Key CPU tools: `tools/ara1/{prepare,matrix,reference_alias,analyze,report}.py`; engine wrappers `run-renderlab.py` and `run-ara.py`; `warm.py`, `stage_loop.py` and `devloop.py` for development loops. `build.py ... --release --jobs 1` places targets under the evidence root. `ara1_prepare --orient SOURCE.rla OUTPUT.rla` makes the canonical sample before `matrix.py prepare`; `matrix.py freeze` snapshots binaries before further staging builds. Captures refuse overwrite: use new tags. After a fresh `lease.py order`, invoke the matrix only through `lease.py --expected-order HASH --label UNIQUE --minutes 10 --handoff-seconds 600 -- ...`.

`memory.py` is the leased, resource-only replay for the seven short-process WDDM gaps; it reuses successful qualifications and checks original settings/poses. Its retained-memory values are marked † and stored separately from timing observations. The PBR reference subset uses sun plus uniform sky and omits the authored sample's three local lights on both engines, because the shared reference API supports sun/sky; the separate 64-light fixtures exercise local lighting.

Final aggregation: `analyze.py --tag final --extra-tag final-clean`, then `report.py`. The scorer reuses `scripts/scan-quality-compare.py` and `support/fresh_level_b_metrics.py`; composition is CPU only. Seven focused CPU contract tests pass, including strict reference identity and physical-reuse rejection; optimized adapters ran end to end on GPU and canonical orientation is byte-idempotent. Render-v3 preparation checks are not new GPU verification.

'''
    sizes=ROOT/'target-sizes-phase2.json'
    if sizes.exists():
        retained=sum(r['bytes'] for r in target_sizes if not r['removed'])/2**30
        summary+=f'Own Cargo targets remain at **{retained:.2f} GiB**: automatic approval review rejected recursive cleanup, including a verified literal target, with “blocked by policy.” No target was removed. Exact paths/sizes/status are in `D:/Coding/renderlab-work/ara1/target-sizes-phase2.json`. This is a cleanup gap. Frozen executables, prepared assets, bake inputs, small summaries and inspected PNGs remain; no disposable raw measurement dump over 5 MiB was found in capture/lease/CPU-verification outputs. Ara worktrees and pre-existing files remain untouched.\n'
    (Path(__file__).resolve().parents[2]/'docs/m7-ara1-report.md').write_text(summary,encoding='utf-8')
    save(out/'manifest.json',dict(report=str(target),report_sha256=digest(target),bytes=target.stat().st_size,
        renderer_base='ae857501ab4792d1889e11111064c3957cc8af44',ara_main='1d2d647f0074f9f1f1d7f69a86f6d71b3142c9e3',
        primary_timing_rows=len(primary),successful_capture_records=len(allruns),retained_primary_variant_warm_records=sum(not r['campaign_role'].startswith('superseded') for r in allruns),
        supplemental_memory_captures=len(supplements),
        validation='seven focused CPU contracts; canonical orientation byte-idempotent; leased end-to-end GPU captures; inspected full images and aspect/motion/cut crops',
        target_cleanup=dict(status='blocked by automatic approval review',records=target_sizes,
            retained_gib=sum(r['bytes'] for r in target_sizes if not r['removed'])/2**30),published=False))
    print(json.dumps(dict(report=str(target),bytes=target.stat().st_size,sha256=digest(target),primary_timing_rows=len(primary))))


if __name__=='__main__':main()
