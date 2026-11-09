"""Quiet frozen-binary ABBA timing screen; always run through the P-MEM GPU lease."""
import json
import os
from pathlib import Path
import statistics
import subprocess
import time
from p_mem_schedule import latest_order

root=Path(os.environ.get("P_MEM_OUT","D:/Coding/renderlab-work/p-mem"))
schedule=Path("E:/renderlab-work/gpu-schedule.md")
order=latest_order(schedule)
with schedule.open("a",encoding="utf-8") as f:
    f.write(f"\np-mem retime admitted {time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())} (8 GiB admission passed; COMPILER-QUIET until p-mem retime window released).\n")
out=root/"retime"
out.mkdir(exist_ok=True)
records=[]
for scene in ["game-base","quarry-daylight"]:
    spec=json.loads((root/"baseline1080"/f"{scene}-nonrt"/"scene.json").read_text())
    track=spec["frames"]
    assert len(track)==88
    spec["frames"]=[dict(frame,capture=False) for _ in range(8) for frame in track]
    for index,tag in enumerate(["baseline1080","candidate","candidate","baseline1080"]):
        latest=latest_order(schedule)
        if latest!=order and "Running leases are not stopped" not in latest:
            raise RuntimeError("new root order; release and re-read")
        order=latest
        case=out/f"{scene}-{index}-{tag}"
        case.mkdir(exist_ok=True)
        path=case/"scene.json"
        path.write_text(json.dumps(spec))
        env=os.environ.copy()
        for key in ["ARA1_MEMORY","ARA1_HDR_HASH"]:env.pop(key,None)
        env["VK_INSTANCE_LAYERS"]=""
        start=time.monotonic()
        with (case/"stdout.log").open("w") as f:
            subprocess.run([str(root/"bin"/f"{tag}.exe"),str(path),"nonrt",str(case)],
                           env=env,stdout=f,stderr=subprocess.STDOUT,check=True,
                           creationflags=subprocess.CREATE_NO_WINDOW)
        e=json.loads((case/"engine.json").read_text())
        gpu={g["frame_id"]:g["span_ms"] for g in e["gpu"]}
        samples={phase:[statistics.mean(gpu[e["frames"][repeat*88+i]["frame_id"]]
                                      for i in frames) for repeat in range(8)]
                 for phase,frames in [("still",range(32,40)),("moving",range(64,72)),("cut",[72])]}
        records.append(dict(scene=scene,tag=tag,order=index,seconds=time.monotonic()-start,samples=samples))
        (out/"samples.json").write_text(json.dumps(records,indent=2))
        print(scene,index,tag,{k:round(statistics.mean(v),4) for k,v in samples.items()},flush=True)
summary={}
for scene in ["game-base","quarry-daylight"]:
    summary[scene]={}
    for phase in ["still","moving","cut"]:
        values={tag:[v for record in records if record["scene"]==scene and record["tag"]==tag
                     for v in record["samples"][phase]] for tag in ["baseline1080","candidate"]}
        summary[scene][phase]={tag:dict(mean_ms=statistics.mean(v),median_ms=statistics.median(v),
                min_ms=min(v),max_ms=max(v),track_samples=len(v)) for tag,v in values.items()}
        summary[scene][phase]["candidate_over_baseline"]=statistics.mean(values["candidate"])/statistics.mean(values["baseline1080"])
(out/"summary.json").write_text(json.dumps(summary,indent=2))
print(json.dumps(summary),flush=True)
