"""Self-contained local S2/S3 comparison chart and actual capture sheets; no server."""
import argparse
import html
import json
from pathlib import Path

import numpy as np


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--root",type=Path,required=True);args=parser.parse_args()
    data={};images=[]
    for bench in ("S2","S3"):
        root=args.root/bench
        scores=json.loads((root/"per-frame.json").read_text());table=json.loads((root/"comparison.json").read_text())
        timing=json.loads((root/"timing-frames.json").read_text());groups={}
        for row in timing:groups.setdefault((row["case"],row["mode"],row["frame"]),[]).append(row)
        rows=[]
        for row in scores:
            values=groups.get((row["case"],row["mode"],row["frame"]),[])
            extra={key:float(np.median([r[key] for r in values if r.get(key) is not None])) for key in ("gpu_ms","shadow_depths_ms","projection_ms") if any(r.get(key) is not None for r in values)}
            rows.append(dict(case=row["case"],mode=row["mode"],frame=row["frame"],phase=row["phase"],partial_rms=row["partial"].get("rms"),
                hdr_rms=row.get("radiance",{}).get("linear_hdr",{}).get("rms"),contact_loss=row["contact_loss_rate"],leaks=row["leaks"],solid_flips=row["solid_flips"],
                jump_ghost_rms=row.get("jump_ghost_rms"),old_sun_projection=row.get("old_sun_projection"),jump_hdr_ghost_rms=row.get("jump_hdr_ghost_rms"),hdr_old_sun_projection=row.get("hdr_old_sun_projection"),**extra))
        data[bench]=dict(rows=rows,table=table,cases=sorted({r["case"] for r in rows}))
        frames=(0,16,63,95,159) if bench=="S2" else (31,319,352,353,355,359,384)
        for case in data[bench]["cases"]:
            for frame in frames:
                path=root/"images"/f"{case}-f{frame:04}.png"
                if path.exists():images.append(f'<img hidden data-bench="{bench}" data-case="{html.escape(case)}" data-frame="{frame}" src="{html.escape(path.as_posix())}" alt="{bench} {html.escape(case)} frame {frame}, actual film and seven measured modes">')
    template='''<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><style>
html,body{margin:0;font-family:var(--font-sans,system-ui);color:var(--foreground)}
.controls{display:flex;flex-wrap:wrap;gap:8px;align-items:center;margin:8px 0 14px}select{background:var(--secondary);color:var(--foreground);border:1px solid var(--border);border-radius:var(--radius);padding:7px;font:inherit;max-width:100%}
.chart{border:1px solid var(--border);border-radius:var(--radius);padding:12px 4px}svg{display:block;width:100%;height:250px}svg text{fill:var(--muted-foreground);font-size:11px}.grid{stroke:var(--border);stroke-width:1}.legend{display:flex;gap:6px 15px;flex-wrap:wrap;font-size:12px;margin:8px 0 12px}.legend span{white-space:nowrap}.legend i{display:inline-block;width:13px;height:3px;vertical-align:middle;margin-right:5px}p{font-size:12px;color:var(--muted-foreground);line-height:1.5}table{border-collapse:collapse;width:100%;font-size:12px;font-variant-numeric:tabular-nums}td,th{padding:7px 5px;text-align:right;border-bottom:1px solid var(--border)}td:first-child,th:first-child{text-align:left}th{color:var(--muted-foreground);font-weight:500}.scroll{overflow:auto}img{max-width:100%;height:auto;margin-top:8px}img[hidden]{display:none}.label{font-size:13px;margin-top:18px}
</style></head><body>
<div class="controls"><select id="bench" aria-label="Bench"><option>S2</option><option>S3</option></select><select id="case" aria-label="Track and source"></select><select id="metric" aria-label="Chart metric"><option value="partial_rms">Partial visibility RMS</option><option value="hdr_rms">Final HDR RMS</option><option value="gpu_ms">GPU milliseconds</option><option value="contact_loss">Contact loss fraction</option><option value="leaks">Leaks</option><option value="solid_flips">Solid flips</option><option value="jump_ghost_rms">Jump ghost RMS (mask)</option><option value="jump_hdr_ghost_rms">Jump ghost RMS (final HDR)</option><option value="hdr_old_sun_projection">Old-sun projection (final HDR)</option><option value="old_sun_projection">Old-sun projection</option><option value="shadow_depths_ms">UE shadow-depth ms</option><option value="projection_ms">UE shadow-projection ms</option></select></div>
<div class="chart"><svg id="plot" viewBox="0 0 680 250" role="img" aria-label="Measured per-frame comparison"></svg></div><div id="legend" class="legend"></div>
<p>Native 1920x1080 rendering. Film scores cover registered native-pixel crops: S2 512x288, S3 384x216. Full frames inspected separately. Renderlab render spans and UE whole-GPU CSV time are diagnostic scopes; no accepted speed ratio. Renderlab software execution is missing on this base.</p>
<div class="controls"><label for="phase">Table phase</label><select id="phase"></select></div><div class="scroll"><table><thead><tr><th>Mode</th><th>Partial RMS</th><th>HDR RMS</th><th>Contact loss</th><th>Max leaks</th><th>Max flips</th><th>GPU p50 ms</th></tr></thead><tbody id="table"></tbody></table></div>
<div class="controls"><label for="frame">Actual crop sheet - frame</label><select id="frame"></select></div>
__IMAGES__
<p>Red/blue panels show signed visibility error x8. Film integrates 256 footprint strata and 256 independently salted sun directions. HRT 256 is a candidate diagnostic. Zero-radius Renderlab modes use the production hard path, including Temporal. Black environment and visible background match; constant-environment coverage is absent.</p>
<script>const DATA=__DATA__;const $=id=>document.getElementById(id);const MODES=['RL hard','RL Exact','RL Interactive','RL Temporal','UE VSM','UE HRT1','UE HRT256'];const color=i=>'var(--chart-'+(i%6+1)+')';let current=[];
function options(id,values,preferred){$(id).innerHTML=values.map(v=>'<option>'+v+'</option>').join('');if(values.includes(preferred))$(id).value=preferred;}
function changeBench(){const d=DATA[$('bench').value];options('case',d.cases,d.cases.find(v=>v==='camera-0.25px-r0.04'));changeCase();}
function changeCase(){const d=DATA[$('bench').value];current=d.rows.filter(r=>r.case===$('case').value);options('phase',[...new Set(current.map(r=>r.phase))]);const frames=[...document.querySelectorAll('img[data-bench]')].filter(im=>im.dataset.bench===$('bench').value&&im.dataset.case===$('case').value).map(im=>im.dataset.frame);options('frame',frames,frames.includes('95')?'95':frames[0]);draw();table();gallery();}
function draw(){const key=$('metric').value;const available=current.filter(r=>r[key]!==null&&r[key]!==undefined);const jump=key.includes('ghost')||key.includes('sun_projection');const minX=jump&&$('bench').value==='S3'?352:0;const maxX=Math.max(...current.map(r=>r.frame),1);const values=available.filter(r=>r.frame>=minX).map(r=>r[key]);const low=Math.min(0,...values);const high=Math.max(...values,low+1e-6);const x=v=>48+(v-minX)/(maxX-minX||1)*618;const y=v=>215-(v-low)/(high-low||1)*195;let s='';for(let i=0;i<5;i++){const v=low+(high-low)*i/4;const yy=y(v);s+='<path class="grid" d="M48 '+yy+'H666"/><text x="43" y="'+(yy+4)+'" text-anchor="end">'+(Math.abs(v)<.1?v.toPrecision(2):v.toFixed(2))+'</text>';}for(let i=0;i<5;i++){const v=minX+(maxX-minX)*i/4;s+='<text x="'+x(v)+'" y="239" text-anchor="middle">'+Math.round(v)+'</text>';}
MODES.forEach((mode,i)=>{const row=available.filter(r=>r.mode===mode&&r.frame>=minX).sort((a,b)=>a.frame-b.frame);if(row.length)s+='<path fill="none" stroke="'+color(i)+'" stroke-width="1.5" '+(i===6?'stroke-dasharray="5 3"':'')+' d="'+row.map((r,j)=>(j?'L':'M')+x(r.frame).toFixed(2)+' '+y(r[key]).toFixed(2)).join(' ')+'"/>';});$('plot').innerHTML=s;$('legend').innerHTML=MODES.map((mode,i)=>'<span><i style="background:'+color(i)+'"></i>'+mode+'</span>').join('');}
const fmt=v=>v==null?'n/a':Number(v).toFixed(4);function table(){const rows=DATA[$('bench').value].table.filter(r=>r.case===$('case').value&&r.phase===$('phase').value);$('table').innerHTML=MODES.map(mode=>{const r=rows.find(r=>r.mode===mode);return r?'<tr><td>'+mode+'</td><td>'+fmt(r.partial_rms)+'</td><td>'+fmt(r.hdr_rms)+'</td><td>'+r.contact_lost+'/'+r.contact_n+'</td><td>'+r.leaks_max+'</td><td>'+r.solid_flips_max+'</td><td>'+fmt(r.gpu_ms_p50)+'</td></tr>':'';}).join('');}
function gallery(){document.querySelectorAll('img[data-bench]').forEach(im=>im.hidden=!(im.dataset.bench===$('bench').value&&im.dataset.case===$('case').value&&im.dataset.frame===$('frame').value));}
$('bench').onchange=changeBench;$('case').onchange=changeCase;$('metric').onchange=draw;$('phase').onchange=table;$('frame').onchange=gallery;changeBench();</script></body></html>'''
    document=template.replace('__DATA__',json.dumps(data,separators=(',',':'))).replace('__IMAGES__','\n'.join(images))
    (args.root/"comparison.html").write_text(document,encoding="utf-8")
    print("wrote",args.root/"comparison.html","crop sheets",len(images))


if __name__=="__main__":main()
