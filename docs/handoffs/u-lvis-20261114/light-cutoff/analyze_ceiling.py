from pathlib import Path
import json,statistics as st
ROOT=Path(__file__).resolve().parent;rows=json.loads((ROOT/'ceiling-runs/rows.json').read_text());out=[]
for path in ['rt','nonrt']:
 for scene in ['flight','night']:
  cases={state:sorted([r for r in rows if r['path']==path and r['scene']==scene and r['state']==state],key=lambda r:r['round']) for state in ['off','black']}
  assert all(len(v)==2 for v in cases.values()),(path,scene,'incomplete')
  cell={'path':path,'scene':scene,'motion':'moving','rounds':{},'metrics':{}}
  for state,rr in cases.items():cell['rounds'][state]=[{k:r[k] for k in ['case','shading_ms','span_ms','samples','seconds']} for r in rr]
  for metric in ['shading_ms','span_ms']:
   a=[r[metric] for r in cases['off']];b=[r[metric] for r in cases['black']];ma=st.mean(a);mb=st.mean(b);spread=max(abs(a[0]-a[1]),abs(b[0]-b[1]));savings=[x-y for x,y in zip(a,b)]
   cell['metrics'][metric]={'off':ma,'black':mb,'saving_ms':ma-mb,'saving_percent':100*(ma-mb)/ma,'max_state_spread_ms':spread,'paired_savings_ms':savings,'paired_delta_spread_ms':abs(savings[0]-savings[1]),'beats_state_spread':ma-mb>spread,'both_rounds_save':all(x>0 for x in savings)}
  cell['ceiling_clears']=all(m['beats_state_spread'] and m['both_rounds_save'] for m in cell['metrics'].values());out.append(cell)
(ROOT/'ceiling-summary.json').write_text(json.dumps(out,indent=2)+'\n')
lines=['Existing hard bound cutoff ceiling: no new renderer code.','RTX4070 SUPER,release0428d73c,1920x1080,32lanes,corrected fixture with --diag program=standard,moving flight/night,both paths,2reversed rounds.','OFF bound unset; ON LIGHT_BOUND=black,CODES=.5,SKIP=unshadowed. LVIS=0. Plain pass log,zero PNG/readbacks.','60 actual renderer-frame samples per run,IDs41..100. Summary = mean of2 per-run medians; spread=max(abs(off1-off2),abs(on1-on2)).','']
for c in out:
 lines.append(c['path']+' '+c['scene'])
 for key,m in c['metrics'].items():lines.append(f"  {key}: {m['off']:.4f} -> {m['black']:.4f} ms; saving {m['saving_ms']:+.4f} ({m['saving_percent']:+.2f}%); max state spread {m['max_state_spread_ms']:.4f}; paired savings {[round(x,4) for x in m['paired_savings_ms']]}; clears={m['beats_state_spread'] and m['both_rounds_save']}")
 lines.append('  joint shading+span screen clears='+str(c['ceiling_clears']))
lines+=['','Caveats: hard/smooth is an optimistic operational screen, not a strict per-light ray superset: smooth B*(1-w) reserves a different remaining shared budget. The coordinator kill rule still applies.','Scope: ordinary custom game_scene calls light_visibility directly and does not reach the bound. Existing program=standard activates its live within seam on fixture props; toon remains exact. Ara STANDARD_PROGRAM, ara-standard-emissive and ara-terrain-splat retain within, but these fixture results are not measured Ara gains.','Global slope 6.443: hard initial full budget .5/(255*6.443)=3.043e-4 exposed maximum RGB; smooth full-skip half-window=1.522e-4; both become stricter after previous lights spend the shared receiver budget.','The .5 code is one shared total bound per receiver, not per light, under the modeled Filmic+sRGB transform. Hard existing mode does not certify arbitrary grade/LUT/bloom. Saved-ray counts alone do not decide this experiment.','Sun follow-up caveats: our Outpost RT sun costs2.6-3.2ms; no matched Unreal Outpost shadow number exists yet. SUN_PASS diagnostic overhead means it is not a proposed default. FORCE_OPAQUE plus Ara alpha-contour mesh cutting means fence exclusions test geometry/traversal, not per-crossing alpha callbacks.']
(ROOT/'ceiling-report.txt').write_text('\n'.join(lines)+'\n',encoding='utf-8');print('\n'.join(lines))
html=['<!doctype html><meta charset="utf-8"><style>body{font:13px var(--font-sans);color:var(--foreground)}.g{margin:18px 0}.r{display:flex;gap:8px;align-items:center;margin:4px 0}.l{width:95px;flex-shrink:0}.t{height:18px;flex:1}.b{height:18px}.a{background:var(--chart-1)}.on{background:var(--chart-2)}.n{width:58px;text-align:right;font-variant-numeric:tabular-nums}small{color:var(--muted-foreground)}</style><small>Hard cutoff feasibility screen / RTX4070 SUPER / 1080p / moving / 2 reversed rounds</small>']
for c in out:
 html.append(f'<div class=g><b>{c["path"].upper()} / {c["scene"]}</b>')
 maxv=max(c['metrics']['span_ms']['off'],c['metrics']['span_ms']['black'])*1.03
 for key,label in [('shading_ms','Shading'),('span_ms','GPU span')]:
  m=c['metrics'][key]
  for state,lab,cls in [('off','OFF','a'),('black','Black','on')]:html.append(f'<div class=r><span class=l>{label} {lab}</span><div class=t><div class="b {cls}" style="width:{m[state]/maxv*100:.2f}%"></div></div><span class=n>{m[state]:.2f}</span></div>')
 html.append(f'<small>Span saving {c["metrics"]["span_ms"]["saving_ms"]:+.2f} ms; paired-state spread {c["metrics"]["span_ms"]["max_state_spread_ms"]:.2f} ms. Joint screen: {"clears" if c["ceiling_clears"] else "does not clear"}.</small></div>')
html.append('<small>Existing standard-program control, not an Ara timing. A lower bar is faster; blue OFF / teal hard black bound. Units ms. Both shading and span must improve beyond spread to clear this conservative screen.</small>')
(ROOT/'ceiling-chart.html').write_text(''.join(html),encoding='utf-8')
