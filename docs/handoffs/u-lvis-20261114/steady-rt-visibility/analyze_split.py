from pathlib import Path
import json,statistics as st
ROOT=Path(__file__).resolve().parent;rows=json.loads((ROOT/'split-runs/rows.json').read_text());summary=[]
for scene in ['showcase','outpost']:
 for motion in ['held','moving']:
  ab={state:[r for r in rows if r['scene']==scene and r['motion']==motion and r['state']==state] for state in ['A','B']}
  assert all(len(v)==2 for v in ab.values())
  def mean(state,key):return st.mean(r[key] for r in ab[state])
  row={'scene':scene,'motion':motion,'a_fused_ms':mean('A','visibility_ms'),'b_camera_receiver_repair_ms':mean('B','visibility_ms'),'b_sun_ms':mean('B','sun_ms'),'b_sum_ms':mean('B','visibility_plus_sun_ms'),'a_span_ms':mean('A','span_ms'),'b_span_ms':mean('B','span_ms'),'a_tlas_ms':mean('A','tlas_ms'),'b_tlas_ms':mean('B','tlas_ms'),'rounds':{s:[{k:r[k] for k in ['case','coverage','visibility_ms','sun_ms','visibility_plus_sun_ms','span_ms']} for r in ab[s]] for s in ab}}
  row['sum_delta_ms']=row['b_sum_ms']-row['a_fused_ms'];row['span_delta_ms']=row['b_span_ms']-row['a_span_ms'];summary.append(row)
(ROOT/'split-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
lines=['RT steady visibility split, RTX 4070 SUPER, P202/Renderlab 0b3f1698,1080p,32 lanes.','Two reversed rounds A/B then B/A. No cuts. A SUN_PASS unset; B=1; SUN_ZONE_STOP unset both.','Values are means of two per-run medians. Sum is median of per-frame visibility+sun; not sum of separate medians.','B camera bucket includes receiver publication/repair; splitting changes codegen/dispatch/record cost, so subtraction does not precisely isolate A\'s internal stages.','Held samples: 60 warm frames ending 200 CPU rows before exit; moving: clip frames20..59. Joined by frame ID with first-frame anchor.','']
for r in summary:
 lines.append(f"{r['scene']} {r['motion']}: A fused {r['a_fused_ms']:.4f}, span {r['a_span_ms']:.4f}; B camera/receiver/repair {r['b_camera_receiver_repair_ms']:.4f}, sun {r['b_sun_ms']:.4f}, combined {r['b_sum_ms']:.4f} (delta {r['sum_delta_ms']:+.4f}), span {r['b_span_ms']:.4f} (delta {r['span_delta_ms']:+.4f}); TLAS A/B {r['a_tlas_ms']:.4f}/{r['b_tlas_ms']:.4f} ms.")
(ROOT/'split-report.txt').write_text('\n'.join(lines)+'\n')
print('\n'.join(lines))
maxv=max(max(r['a_fused_ms'],r['b_sum_ms']) for r in summary)*1.05
html=['<!doctype html><meta charset="utf-8"><style>body{font:13px var(--font-sans);color:var(--foreground)}.group{margin:18px 0}.row{display:flex;gap:8px;align-items:center;margin:5px 0}.label{width:90px;flex-shrink:0}.track{height:24px;display:flex;flex:1}.bar{height:24px;display:inline-block}.num{font-variant-numeric:tabular-nums;width:62px;text-align:right}.a{background:var(--chart-1)}.camera{background:var(--chart-2)}.sun{background:var(--chart-3)}small{color:var(--muted-foreground)}</style><small>GPU milliseconds · RTX 4070 SUPER · P202 · 1080p · 2 reversed rounds</small>']
for r in summary:
 html.append(f"<div class=group><b>{r['scene'].title()} · {r['motion']}</b><div class=row><span class=label>A fused</span><div class=track><span class='bar a' style='width:{r['a_fused_ms']/maxv*100:.3f}%'></span></div><span class=num>{r['a_fused_ms']:.2f}</span></div><div class=row><span class=label>B split</span><div class=track><span class='bar camera' style='width:{r['b_camera_receiver_repair_ms']/maxv*100:.3f}%'></span><span class='bar sun' style='width:{r['b_sun_ms']/maxv*100:.3f}%'></span></div><span class=num>{r['b_sum_ms']:.2f}</span></div><small>Camera + receiver + repair {r['b_camera_receiver_repair_ms']:.2f} · sun {r['b_sun_ms']:.2f}. Full span {r['a_span_ms']:.2f} → {r['b_span_ms']:.2f} ms.</small></div>")
html.append('<small>Blue: fused visibility · teal: camera/receiver/repair · third color: sun. B changes shader codegen and dispatch boundaries; it is a diagnostic split.</small>')
(ROOT/'split-chart.html').write_text(''.join(html))

# Preserve the current reporting correction when regenerating this result.
_correction=(ROOT/"sun-comparison-correction.txt").read_text(encoding="utf-8")
_report=ROOT/"split-report.txt"
_report.write_text(_correction+"\n"+_report.read_text(encoding="utf-8"),encoding="utf-8")
