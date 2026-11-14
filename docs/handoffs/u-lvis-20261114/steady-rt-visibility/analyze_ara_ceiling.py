from pathlib import Path
import json,statistics as st
ROOT=Path(__file__).resolve().parent.parent/'light-cutoff';results=[]
for path in ['rt','nonrt']:
 base=ROOT/('ara-ceiling-'+path);latest=base/'latest-run.txt'
 if not latest.exists():continue
 run=Path(latest.read_text().strip());gate=json.loads((run/'activation-gate.json').read_text());assert gate['passed'],'ARA bound activation not confirmed'
 rows=json.loads((run/'rows.json').read_text())
 for scene in ['outpost','showcase']:
  for motion in ['held','moving']:
   cases={s:sorted([r for r in rows if r['scene']==scene and r['motion']==motion and r['state']==s],key=lambda r:r['round']) for s in ['A','B']}
   assert all(len(v)==2 for v in cases.values()),(path,scene,motion,'incomplete')
   row={'path':path,'scene':scene,'scene_file':cases['A'][0]['scene_file'],'scene_sha256':cases['A'][0]['scene_sha256'],'player_sha256':cases['A'][0]['player_sha256'],'motion':motion,'run_dir':str(run),'activation_gate':str(run/'activation-gate.json'),'metrics':{},'rounds':{s:[{k:r[k] for k in ['case','coverage','shading_ms','span_ms']} for r in rr] for s,rr in cases.items()}}
   for key in ['shading_ms','span_ms']:
    a=[r[key] for r in cases['A']];b=[r[key] for r in cases['B']];ma=st.mean(a);mb=st.mean(b);spread=max(abs(a[0]-a[1]),abs(b[0]-b[1]));save=ma-mb
    row['metrics'][key]={'off_ms':ma,'black_ms':mb,'saving_ms':save,'max_state_spread_ms':spread,'excess_over_spread_ms':save-spread,'paired_savings_ms':[x-y for x,y in zip(a,b)],'clears_spread_by_0_2':save-spread>=.2}
   row['go_cell']=row['metrics']['span_ms']['clears_spread_by_0_2'];results.append(row)
(ROOT/'ara-ceiling-summary.json').write_text(json.dumps(results,indent=2)+'\n')
lines=['Ara existing black-bound ceiling. OwnP202,Renderlab0b3f1698,RTX4070 SUPER,1920x1080,32lanes.','Outpost and Showcase main scenes; no Outpost night variant exists in owned project. Both paths,held/moving,2reversed rounds.','OFF bound unset;ON LIGHT_BOUND=black,CODES=.5,SKIP=unshadowed. LVIS=0. SUN_PASS/ZONE_STOP unset.','Activation FIRST: existing STARTUP_PROFILE program_compile log has equal raster_bytes and larger traced_bytes for the same standard-family program on OFF/ON. First pair must pass before accepting timing. Gate JSON retains actual compiler lines.','Means of two per-run medians. Spread=max(abs(off1-off2),abs(on1-on2)); coordinator go criterion interpreted strictly: GPU-span saving minus that spread >=0.2ms in any complete cell.','']
for r in results:
 lines.append(r['path']+' '+r['scene']+' '+r['motion']+' '+r['scene_sha256'])
 for key,m in r['metrics'].items():lines.append(f"  {key}: {m['off_ms']:.4f}->{m['black_ms']:.4f};saving {m['saving_ms']:+.4f};spread {m['max_state_spread_ms']:.4f};excess {m['excess_over_spread_ms']:+.4f}ms;paired {m['paired_savings_ms']}")
paths={r['path'] for r in results};lines+=['','Complete paths: '+','.join(sorted(paths)),('Coordinator threshold clears in at least one cell.' if any(r['go_cell'] for r in results) else 'No completed cell clears the coordinator threshold.'),'No smooth implementation decision is taken from incomplete paths. All counts are diagnostic; this is a GPU-span gate, not ray-count success.','Existing hard bound is an optimistic operational screen, not a strict ray-superset theorem for smooth shared-budget allocation. Existing hard mode bounds modeled Filmic+sRGB, not arbitrary grading/bloom. All pictures remain retained.','']
(ROOT/'ara-ceiling-report.txt').write_text('\n'.join(lines)+'\n',encoding='utf-8');print('\n'.join(lines))

_context=ROOT/"budget-counter-context.txt"
if _context.exists():
 _report=ROOT/"ara-ceiling-report.txt"
 _report.write_text(_report.read_text(encoding="utf-8")+"\n"+_context.read_text(encoding="utf-8"),encoding="utf-8")
