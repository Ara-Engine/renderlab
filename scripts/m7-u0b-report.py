"""CPU summaries of retained U0b measurements; never fits image exposure or drops frames."""
import argparse
import datetime as dt
import json
from pathlib import Path
import re
import statistics
import hashlib
import subprocess

import numpy as np

OUT=Path("E:/renderlab-work/m7-u0b")


def save(path,value):path.write_text(json.dumps(value,indent=2,allow_nan=False)+"\n")


def sha(path):
    with path.open("rb") as stream:return hashlib.file_digest(stream,"sha256").hexdigest()


def annotate_black(args):
    """Retrospective source audit, explicitly distinct from a render-time readback.
    Neither the lossless payload nor mixed-edge samples are modified."""
    repo=Path(__file__).resolve().parent.parent
    checkpoint="aea9dd4"
    source=subprocess.check_output(["git","show",checkpoint+":src/reference_quarry_tests.rs"],cwd=repo,text=True)
    statement="scene.camera_background = Some([0.0; 3]);"
    if statement not in source:raise ValueError("historical hard-coded control missing")
    shader=(repo/"shaders/reference_path_tracer.wgsl").read_text()
    miss="if depth==1u && frame.background.w>0.0 {return frame.background.rgb;}"
    if miss not in shader:raise ValueError("primary-only shader control missing")
    root=OUT/"reference/quarry-final"
    audits={}
    for name in ("quarry-p0","quarry-p0-sky","quarry-p1","quarry-p1-sky"):
        path=root/(name+".json");meta=json.loads(path.read_text());payload=root/(name+".npz")
        identity=sha(payload)
        if identity!=meta["archive_sha256"]:raise ValueError("archive identity changed: "+name)
        with np.load(payload) as arrays:
            # The upper 64 rows are wholly outside the quarry at both registered poses.
            clear=arrays["hit"][:64,:,0]==0
            if not clear.all():raise ValueError("clear-region geometry changed")
            if any(np.any(arrays[k][:64]!=0) for k in ("L0","L1","D0","D1")):
                raise ValueError("retained clear region is not exactly black")
        before=sha(path)
        proof=dict(kind="retrospective source-validated control; not a render-time metadata readback",
                   annotated_utc=dt.datetime.now(dt.timezone.utc).isoformat(),
                   archive_sha256=identity,metadata_sha256_before_annotation=before,
                   source_checkpoint=checkpoint,source_path="src/reference_quarry_tests.rs",
                   source_statement=statement,source_sha256=hashlib.sha256(source.encode()).hexdigest(),
                   shader_path="shaders/reference_path_tracer.wgsl",primary_miss_statement=miss,
                   runtime_record=str(root/"process/process.json"),
                   evidence="U0b implementation/run provenance plus historical hard-coded black control; original JSON omitted this field. No immutable per-run source snapshot was saved.",
                   payload_audit=dict(clear_top_rows=64,pixels=64*meta["width"],centre_hits=0,L0_L1_D0_D1_exactly_zero=True),
                   archive_modified=False,secondary_environment="retained RLE2, unchanged",sky_background_silhouettes="untested")
        if "camera_background_provenance" in meta:
            proof=meta["camera_background_provenance"]
        meta|=dict(camera_background=[0,0,0],camera_background_provenance=proof)
        save(path,meta);audits[name]=proof
    save(root/"black-primary-provenance.json",audits)
    print(json.dumps({k:v["archive_sha256"] for k,v in audits.items()},indent=1))


def aliases(args):
    """Publish only exact camera/sample aliases whose lossless source already exists."""
    manifest=json.loads(args.manifest.read_text());root=OUT/"reference/quarry-final"
    by_name={f["name"]:f for f in manifest["frames"]};verified={};published=[]
    keys=("open","close","fov_y","width","height","samples","sky_only")
    for frame in manifest["frames"]:
        source=frame.get("alias_of")
        if source is None or not (root/(source+".npz")).exists():continue
        if any(frame[k]!=by_name[source][k] for k in keys):raise ValueError("nonidentical alias")
        if source not in verified:
            meta=json.loads((root/(source+".json")).read_text())
            identity=sha(root/(source+".npz"))
            if identity!=meta["archive_sha256"] or meta.get("camera_background")!=[0,0,0]:
                raise ValueError("unverified alias source: "+source)
            verified[source]=identity
        save(root/(frame["name"]+".json"),dict(frame=frame,alias_of=source,
             camera_background=[0,0,0],source_archive_sha256=verified[source],
             sequence_sha256=manifest["sequence_sha256"],
             alias_definition="identical shutter endpoints, FOV, resolution, samples and lighting mode; no resampling"))
        published.append(frame["name"])
    print(json.dumps(dict(published=len(published),names=published)))


def holds():
    intervals=[];start=None
    for line in Path("E:/renderlab-work/gpu-schedule.md").read_text(encoding="utf-8").splitlines():
        if not line.startswith("[GAME HOLD"):continue
        stamp=re.search(r"20\d\d-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|\+00:00)",line)
        if not stamp:continue
        when=dt.datetime.fromisoformat(stamp[0].replace("Z","+00:00"))
        if line.startswith("[GAME HOLD RELEASED]"):
            if start:intervals.append((start,when));start=None
        elif start is None:start=when
    if start:intervals.append((start,dt.datetime.max.replace(tzinfo=dt.timezone.utc)))
    return intervals


def quiet(args):
    rows=[]
    for directory in sorted((OUT/"study").glob("quiet-*")):
        for path in directory.glob("*/summary.json"):
            case=path.parent
            process=json.loads((case/"process.json").read_text())
            readback=json.loads((case/"readback.json").read_text())
            if process["exit_code"] or not readback["all_ok"]:continue
            start,end=(dt.datetime.fromisoformat(process[k]) for k in ("start_utc","end_utc"))
            overlap=any(start<b and end>a for a,b in holds())
            summary=json.loads(path.read_text());gpu=summary["statistics"]["GPUTime"]
            match=re.search(r"-(static|camera|sun|pan)-(\d+)$",case.name)
            if not match:continue
            clean=not overlap and gpu["p95"]/gpu["median"]<=1.15
            rows.append(dict(case=str(case),config=readback["config"],phase=match[1],visit=int(match[2]),
                         gpu=gpu,clean=clean,game_overlap=overlap,
                         pass_costs=summary["pass_costs_async_off"],effective_mode=readback["effective_mode"]["tracing"]))
    groups={}
    for row in rows:groups.setdefault(row["config"]+"/"+row["phase"],[]).append(row)
    table={}
    for key,values in groups.items():
        clean=[r for r in values if r["clean"]]
        def span(v,k):return [min(r["gpu"][k] for r in v),max(r["gpu"][k] for r in v)] if v else None
        table[key]=dict(visits=len(values),clean_visits=len(clean),median_of_clean_process_medians_ms=statistics.median(r["gpu"]["median"] for r in clean) if clean else None,
            clean_process_median_range_ms=span(clean,"median"),all_process_median_range_ms=span(values,"median"),
            clean_process_p95_range_ms=span(clean,"p95"),game_overlap_visits=sum(r["game_overlap"] for r in values))
    result=dict(clean_rule="preregistered p95/p50 <=1.15; no hold overlap; all64 native frames retained per visit",
                scope="per-process medians and p95 ranges, not pooled percentiles; quiet-start, wallpaper paused, exclusive lease; clocks not locked",
                table=table,visits=rows)
    save(args.out,result)
    print(json.dumps(table,indent=1))


def main():
    p=argparse.ArgumentParser(description=__doc__);s=p.add_subparsers(dest="command",required=True)
    q=s.add_parser("quiet");q.add_argument("--out",type=Path,default=OUT/"quiet-summary.json")
    s.add_parser("annotate-black")
    q=s.add_parser("aliases");q.add_argument("manifest",type=Path)
    a=p.parse_args();globals()[a.command.replace("-","_")](a)


if __name__=="__main__":main()
