from pathlib import Path
import json,statistics as st
ROOT=Path(__file__).resolve().parent;run=Path((ROOT/'range-latest-run.txt').read_text().strip());rows=json.loads((run/'rows.json').read_text());summary=[]
for scene in ['outpost','showcase']:
 for motion in ['held','moving']:
  cases={state:sorted([r for r in rows if r['scene']==scene and r['motion']==motion and r['state']==state],key=lambda r:r['round']) for state in ['A','B']}
  assert all(len(v)==2 for v in cases.values()),(scene,motion,'incomplete range matrix')
  row={'scene':scene,'scene_file':cases['A'][0]['scene_file'],'scene_sha256':cases['A'][0]['scene_sha256'],'motion':motion,'states':{'A':'4/12/40/1000m','B':'4/12/40m'},'metrics':{},'rounds':{s:[{k:r[k] for k in ['case','coverage','visibility_ms','sun_ms','visibility_plus_sun_ms','span_ms']} for r in rr] for s,rr in cases.items()}}
  for key in ['visibility_ms','sun_ms','visibility_plus_sun_ms','span_ms','tlas_ms']:
   a=[r[key] for r in cases['A']];b=[r[key] for r in cases['B']];ma=st.mean(a);mb=st.mean(b);spread=max(abs(a[0]-a[1]),abs(b[0]-b[1]));row['metrics'][key]={'long_ms':ma,'short_ms':mb,'saving_ms':ma-mb,'saving_percent':100*(ma-mb)/ma if ma else None,'max_state_spread_ms':spread,'paired_savings_ms':[x-y for x,y in zip(a,b)],'beats_spread':ma-mb>spread}
  summary.append(row)
(ROOT/'range-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
meta=json.loads((ROOT/'copy-manifest.json').read_text());lines=['RT sun range ablation. Own P202 Renderlab0b3f1698,RTX4070 SUPER,1080p,32lanes,2reversed rounds,Outpost first/Showcase control.','SUN_PASS=1 BOTH; SUN_ZONE_STOP unset; A default4/12/40/1000m; B SUN_SPLITS=4,12,40.','Held warm60 and moving clip20..59/40 frames,ID-joined and anchored, no cuts/video readbacks. Means of two per-run medians; spread=max within-state round difference.','Scene file/hash: '+json.dumps(meta['scenes']),'Player SHA256: '+meta['player_sha256'],'Attempt directory: '+str(run),'']
for row in summary:
 lines.append(row['scene']+' '+row['motion'])
 for key,m in row['metrics'].items():lines.append(f"  {key}: long {m['long_ms']:.4f} -> short {m['short_ms']:.4f}; saved {m['saving_ms']:+.4f} ms; spread {m['max_state_spread_ms']:.4f}; paired {m['paired_savings_ms']}")
lines+=['','This deliberately removes distant receiver shadow coverage; it is a cost ablation, not a proposed quality/default change. It can change both receiver admission and shadow-ray segment/box geometry, so timing alone cannot say ray count versus cheaper traversal. Warm census awaits U-RTVIS frame selector.','There is no matched Unreal Outpost/Showcase shadow timer here; never compare this to the old10-Nov quarry shadow0.5-.63ms as a ratio. SUN_PASS diagnostic overhead remains, since BOTH states use it.','(a) needs frame-selectable SUN_RAYS_PROBE; (d) proposed measurement counter: sun_facing_queries, splitting geometric versus resolved-material facing, thin/two-sided status and query count; current probe has no such field. Geometric facing is diagnostic, not a safe reject gate. No counter written.','(c) fence/foliage/terrain exclusions, if scheduled, are geometry/traversal ablations: FORCE_OPAQUE rays and alpha-contour mesh cutting mean no per-crossing alpha callback is established.']
(ROOT/'range-report.txt').write_text('\n'.join(lines)+'\n',encoding='utf-8');print('\n'.join(lines))
maxv=max(m['metrics']['sun_ms'][s] for m in summary for s in ['long_ms','short_ms'])*1.05
html=['<!doctype html><meta charset="utf-8"><style>body{font:13px var(--font-sans);color:var(--foreground)}.g{margin:18px 0}.r{display:flex;gap:8px;align-items:center;margin:5px 0}.l{width:90px;flex-shrink:0}.t{flex:1;height:23px}.b{height:23px}.a{background:var(--chart-1)}.b40{background:var(--chart-2)}.n{width:60px;text-align:right;font-variant-numeric:tabular-nums}small{color:var(--muted-foreground)}</style><small>Sun range cost ablation / RTX4070 SUPER / P202 / 1080p / 2 reversed rounds</small>']
for r in summary:
 m=r['metrics']['sun_ms'];sp=r['metrics']['span_ms'];html.append(f'<div class=g><b>{r["scene"].title()} / {r["motion"]}</b>')
 for key,label,cls in [('long_ms','1000m','a'),('short_ms','40m','b40')]:html.append(f'<div class=r><span class=l>{label}</span><div class=t><div class="b {cls}" style="width:{m[key]/maxv*100:.2f}%"></div></div><span class=n>{m[key]:.2f} ms</span></div>')
 html.append(f'<small>Sun saving {m["saving_ms"]:+.2f} ms; state spread {m["max_state_spread_ms"]:.2f}. Full span {sp["long_ms"]:.2f} to {sp["short_ms"]:.2f} ms.</small></div>')
html.append('<small>Both states use SUN_PASS=1. The40m row deliberately loses distant shadows; this is not a proposed default or quality-equivalent mode. Receiver and ray counts remain unmeasured.</small>');(ROOT/'range-chart.html').write_text(''.join(html),encoding='utf-8')

# Preserve the current reporting correction when regenerating this result.
_correction=(ROOT/"sun-comparison-correction.txt").read_text(encoding="utf-8")
_report=ROOT/"range-report.txt"
_report.write_text(_correction+"\n"+_report.read_text(encoding="utf-8"),encoding="utf-8")
