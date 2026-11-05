"""Make a local results chart and selected decision crops from saved R1 evidence."""
import json
from pathlib import Path
from PIL import Image, ImageDraw

def main():
    root=Path("D:/Coding/renderlab-work/m6-exact-quality/r1")
    d=json.loads((root/"scores/summary.json").read_text())
    c=d["0"]["policies"]["magnitude"]["convergence"]["partial"]
    keys=("visibility_rms","radiance_rms")
    curves=[[r[k]/c["fits"][k]["exact_point_rms"] for r in c["curve"]] for k in keys]
    maximum=max(max(v) for v in curves)*1.05
    px=lambda i:45+610*i/15
    py=lambda v:180-150*v/maximum
    svg=['<svg viewBox="0 0 700 220" role="img" aria-label="Saved-sample visibility and radiance RMS relative to Exact">']
    for value in (0,1,2):
        svg.append(f'<path d="M45 {py(value)}H655" stroke="var(--border)"/><text x="33" y="{py(value)+4}" text-anchor="end">{value:g}</text>')
    for n in (1,2,4,8,16):svg.append(f'<text x="{px(n-1)}" y="204" text-anchor="middle">{n}</text>')
    svg.append('<text x="350" y="220" text-anchor="middle">Saved primary samples</text>')
    for i,values in enumerate(curves):
        points=' '.join(f'{px(j):.2f},{py(v):.2f}' for j,v in enumerate(values))
        svg.append(f'<polyline points="{points}" fill="none" stroke="var(--chart-{i+1})" stroke-width="2.5"/>')
    svg.append('</svg>')
    figures=[]
    for pose in (0,1):
        x,y=d[str(pose)]["viewed_crop_coordinates"][0]
        size=260;out=Image.new("RGB",(size*2,(size+25)*2),(20,27,35));draw=ImageDraw.Draw(out)
        for row,channel in enumerate(("R_L+4","R_D+4")):
            for col,policy in enumerate(("magnitude","viewed")):
                im=Image.open(root/f"images/pose-{pose}-{policy}-{channel}.png")
                draw.text((col*size+3,row*(size+25)+4),f'{policy} {channel}',fill='white')
                out.paste(im.crop((x-32,y-32,x+33,y+33)).resize((size,size),Image.Resampling.NEAREST),(col*size,row*(size+25)+25))
        path=root/f"images/pose-{pose}-decision-summary.png";out.save(path)
        figures.append(f'<figure><figcaption>Pose {pose}: ({x}, {y}), backlit crop</figcaption><img src="{path.as_posix()}" alt="Magnitude and viewed-side full and sun-only radiance at plus four stops"/></figure>')
    html='''<!doctype html><html><head><meta charset="utf-8"><style>
body{margin:0;color:var(--foreground);font-family:var(--font-sans)}
.chart{padding:16px;background:var(--card);border-radius:var(--radius)}
svg{display:block;width:100%;height:220px}svg text{fill:var(--muted-foreground);font:12px var(--font-sans)}
.legend{display:flex;gap:18px;font-size:13px}.legend span:first-child{color:var(--chart-1)}.legend span:last-child{color:var(--chart-2)}
p,figcaption{font-size:13px;color:var(--muted-foreground);line-height:1.5}p{margin:8px 0}
.crops{display:grid;grid-template-columns:1fr 1fr;gap:12px;margin-top:16px}figure{margin:0;min-width:0}figcaption{margin-bottom:8px}img{display:block;width:100%;height:auto}
@media(max-width:500px){.crops{grid-template-columns:1fr}}
</style></head><body><div class="chart"><strong>Ideal saved-sample convergence · pose 0, magnitude policy</strong><p>RMS / point Exact RMS. Crossing below 1 reaches Exact’s image error. These are 256-ray samples; the real AA path is untested.</p><div class="legend"><span>Visibility</span><span>Full radiance</span></div>'''+''.join(svg)+'''</div><div class="crops">'''+''.join(figures)+'''</div><p>Identical exposure and transforms. Viewed-side is an estimand change and remains unadopted. Complete five-crop sheets and both exposures are in the report.</p></body></html>'''
    (root/"results.html").write_text(html,encoding="utf-8")
    print(root/"results.html")

if __name__=="__main__":main()
