"""Summarize AT2's retained images and timing; never launches GPU work."""
import argparse
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

p=argparse.ArgumentParser(description=__doc__)
p.add_argument("root",type=Path)
a=p.parse_args()
root=a.root

def sheet(name,rows,width):
    height=width*9//16
    canvas=Image.new("RGB",(width*len(rows[0]),(height+26)*len(rows)),"#151820")
    draw=ImageDraw.Draw(canvas)
    for y,row in enumerate(rows):
        for x,(title,file) in enumerate(row):
            image=Image.open(file).convert("RGB").resize((width,height))
            canvas.paste(image,(x*width,y*(height+26)+26))
            draw.text((x*width+6,y*(height+26)+6),title,fill="white")
    canvas.save(root/name)

quarry={}
for path in ["nonrt","traced","raster"]:
    data=json.loads((root/path/(path+"-at2.json")).read_text())
    quarry[path]={"size":data["size"],"generation_before":data["initial_map_generation"],"generation_after":data["final_map_generation"],"gi_still_age":data["gi_still_age"],"moving_images":data["moving_images"],"timing":[]}
    for t in data["timings"]:
        quarry[path]["timing"].append(dict(moving=t["moving"],wall_ms_per_frame=t["wall_ms_per_frame"],cpu_p50_ms=float(np.median(t["cpu_submit_ms"])),gpu_span_p50_ms=float(np.median(t["gpu_span_ms"]))))
    energies=[d["energy_exposed_rgb"] for d in data["disk"]]
    quarry[path]["disk_energy"]=energies
    quarry[path]["disk_energy_range_percent"]=100*(max(energies)-min(energies))/np.mean(energies)

rows=[]
for path in quarry:
    rows.append([(path+" / "+s,root/path/(path+"-"+s+".png")) for s in ["still","sun-00","sun-15","sun-31"]])
sheet("quarry-sheet.png",rows,480)
rows=[]
for path in ["nonrt","rt"]:
    rows.append([(path+" / "+s+" / "+str(frame),root/("game-"+path+"-"+s)/f"frame-{frame:03}.png") for s,frame in [("still",96),("sun",96),("sun",112),("sun",143)]])
sheet("game-sheet.png",rows,480)
# Preserve pixel structure when magnifying the tiny physical sun disk.
canvas=Image.new("RGB",(8*192,3*218),"#151820")
draw=ImageDraw.Draw(canvas)
disk_arrays={}
for y,path in enumerate(quarry):
    data=json.loads((root/path/(path+"-at2.json")).read_text())
    disk_arrays[path]=[np.array(d["crop_hdr_rgb"],dtype=np.float64).reshape(48,48,3) for d in data["disk"]]
    for x in range(8):
        values=disk_arrays[path][x]
        display=np.uint8(np.clip((values/(1+values))**(1/2.2)*255+.5,0,255))
        image=Image.fromarray(display).resize((192,192),Image.Resampling.NEAREST)
        canvas.paste(image,(192*x,218*y+26))
        draw.text((192*x+6,218*y+6),f"{path} / sun {x*.01:.2f} deg",fill="white")
canvas.save(root/"disk-sheet.png")
difference={}
for path in ["nonrt","raster"]:
    comparisons=[]
    for a,b in zip(disk_arrays[path],disk_arrays["traced"]):
        comparisons.append(dict(rms=float(np.sqrt(np.mean((a-b)**2))),max_abs=float(np.max(abs(a-b))),pixels_above_001=int(np.any(abs(a-b)>.001,axis=2).sum())))
    difference[path+"_vs_traced"]=comparisons

game={}
for path in ["nonrt","rt"]:
    for mode in ["still","sun"]:
        data=json.loads((root/("game-"+path+"-"+mode)/"run.json").read_text())
        samples=[f for f in data["frames"] if f["frame"]>=96]
        game[path+"-"+mode]=dict(wall_ms_per_frame=data["timing"]["wall_ms_per_frame"],adapter=data["adapter"],settings=data["settings"],cpu_p50_ms=float(np.median([f["cpu_render_ms"] for f in samples])),sun_api_p50_ms=float(np.median([f["cpu_sun_update_ms"] for f in samples])),gpu_span_p50_ms=float(np.median(data["gpu_render_span_ms"])))
summary=dict(quarry=quarry,game=game,disk_difference=difference,note="Shared-host diagnostic samples, not a quiet performance claim. RT GPU spans exclude pre-primary atmosphere.")
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
fig, axes = plt.subplots(1,3,figsize=(15,4),constrained_layout=True)
labels=list(quarry)
x=np.arange(len(labels));w=.36
for i,label in enumerate(["stationary", "moving sun"]):
    axes[0].bar(x+(i-.5)*w,[quarry[k]["timing"][i]["wall_ms_per_frame"] for k in labels],w,label=label)
axes[0].set_xticks(x,labels);axes[0].set_ylabel("ms/frame wall throughput");axes[0].set_title("Quarry / 32 frames per segment");axes[0].legend()
for i,mode in enumerate(["still","sun"]):
    axes[1].bar(np.arange(2)+(i-.5)*w,[game[k+"-"+mode]["wall_ms_per_frame"] for k in ["nonrt","rt"]],w,label=mode)
axes[1].set_xticks(np.arange(2),["nonrt","rt"]);axes[1].set_ylabel("ms/frame drained wall throughput");axes[1].set_title("game_scene / 32 frames per segment");axes[1].legend()
for k in labels:
    v=np.array(quarry[k]["disk_energy"])
    axes[2].plot(np.arange(8)*.01,100*(v/v[0]-1),".-",label=k)
axes[2].set_xlabel("sun elevation change (degrees)");axes[2].set_ylabel("crop HDR energy change (%)");axes[2].set_title("Disk / temporal reconstruction off");axes[2].legend()
fig.suptitle("AT2 shared-host diagnostics / not a quiet performance claim")
fig.savefig(root/"timing-chart.png",dpi=140)
plt.close(fig)
(root/"summary.json").write_text(json.dumps(summary,indent=2))
print(json.dumps(summary,indent=2))
