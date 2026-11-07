"""CPU summaries of retained U0b measurements; never fits image exposure or drops frames."""
import argparse
import datetime as dt
import json
from pathlib import Path
import re
import statistics
import hashlib
import subprocess
import math
import importlib.util

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
            if gpu["n"]!=64:raise ValueError("quiet visit must retain all 64 timed frames: "+str(case))
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
    paired={}
    for config,baseline in (("ue-sw-h","ue-tsr"),("ue-sw-e","ue-tsr"),
                            ("ue-hw-h","ue-tsr-hrt"),("ue-hw-e","ue-tsr-hrt")):
        for phase in ("static","camera","sun"):
            a={r["visit"]:r for r in groups.get(config+"/"+phase,[])}
            b={r["visit"]:r for r in groups.get(baseline+"/"+phase,[])}
            delta=[dict(visit=i,delta_ms=r["gpu"]["median"]-b[i]["gpu"]["median"])
                   for i,r in a.items() if i in b and r["clean"] and b[i]["clean"]]
            paired[config+"/"+phase]=dict(baseline=baseline,pairs=delta,
                range_ms=[min(r["delta_ms"] for r in delta),max(r["delta_ms"] for r in delta)] if delta else None)
    result=dict(clean_rule="preregistered p95/p50 <=1.15; no hold overlap; all64 native frames retained per visit",
                scope="per-process medians and p95 ranges, not pooled percentiles; quiet-start, wallpaper paused, exclusive lease; clocks not locked",
                table=table,paired_lumen_increment=paired,visits=rows)
    save(args.out,result)
    print(json.dumps(table,indent=1))


def floors(args):
    """Pixel-weighted summaries; detailed per-frame tails remain in physical-floor.json."""
    configs=("ue-sw-h","ue-sw-e","ue-hw-h","ue-hw-e")
    statics={}
    paths=list(sorted((OUT/"study").glob("*/physical-static-floor.json")))
    # The later synchronized quality campaign wins over earlier timing snapshots.
    paths.sort(key=lambda p:p.parent.name=="floor-static")
    for path in paths:
        for row in json.loads(path.read_text())["frames"]:statics[row["config"]]=row
    walks={}
    for path in args.walk.glob("*/physical-floor.json"):
        value=json.loads(path.read_text());walks[value["config"]]=value
    def aggregate(rows,offset):
        populations={key for row in rows for key in row["measures"][offset]};result={}
        for key in sorted(populations):
            values=[row["measures"][offset][key] for row in rows if key in row["measures"][offset]]
            n=sum(v["n"] for v in values)
            result[key]=dict(n=n,observations=len(values),**{
                metric:math.sqrt(sum(v["n"]*v[metric]**2 for v in values)/n)
                for metric in ("rms","rms_noise_subtracted","reference_noise_rms")})
        return result
    motion={c:dict(frames=len(w["frames"]),missing=w["missing_frames"],complete=w["complete"],
        error={o:aggregate(w["frames"],o) for o in ("0","4")},
        residual={o:aggregate([dict(measures={o:r["measures"]}) for r in w["motion"] if str(r["offset"])==o],o) for o in ("0","4")})
        for c,w in walks.items()}
    result=dict(scope="physical combined error; registered F0 and +4 stops; pixel-weighted RMS across retained native frames; no gain fit",
                static=statics,motion=motion)
    save(args.out,result)
    lines=[]
    for title,source in (("Static",{c:r["measures"] for c,r in statics.items()}),
                         ("Motion image",{c:r["error"] for c,r in motion.items()}),
                         ("Motion residual",{c:r["residual"] for c,r in motion.items()})):
        if not source:continue
        lines += ["### "+title,"","RMS F0 codes at registered exposure / +4 stops.","",
                  "| Population | SW High | SW Epic | HW High | HW Epic |","|---|---:|---:|---:|---:|"]
        for key in sorted({k for r in source.values() for k in r["0"]}):
            values=["/".join(f'{source[c][o][key]["rms"]:.3f}' for o in ("0","4"))
                    if c in source and key in source[c]["0"] else "pending" for c in configs]
            lines.append("| "+key+" | "+" | ".join(values)+" |")
        lines.append("")
    args.out.with_suffix(".md").write_text("\n".join(lines))
    print(json.dumps(dict(static_configs=list(statics),motion={c:dict(frames=r["frames"],missing=len(r["missing"])) for c,r in motion.items()})))


def benches(args):
    from PIL import Image,ImageDraw
    path=Path(__file__).with_name("m7-u0-ue.py")
    spec=importlib.util.spec_from_file_location("ue",path);ue=importlib.util.module_from_spec(spec);spec.loader.exec_module(ue)
    expected={(b,c) for b in ("SV","S4","S5","S2G") for c in ue.CONFIGS};rows={}
    for path in args.root.glob("*/native.json"):
        case=path.parent
        bench=next((b for b in ("SV","S4","S5","S2G") if re.match(r"(?:retry\d+-)?"+b+"-",case.name)),None)
        runtime=json.loads((case/"readback.json").read_text());scene=json.loads((case/"scene-readback.json").read_text())
        camera=json.loads((case/"camera-readback.json").read_text());process=json.loads((case/"process.json").read_text())
        native=json.loads(path.read_text())["frames"]
        log=(case/"ue.log").read_text(encoding="utf-8-sig",errors="replace")
        platform=dict(requested_d3d12="-dx12" in process["command"],requested_sm6="-sm6" in process["command"],
            actual_pcd3d_sm6='shaderplatform="PCD3D_SM6"' in log)
        if not (runtime["all_ok"] and scene["sun_angle_ok"] and camera["all_ok"] and process["exit_code"]==0 and len(native)==1):
            raise ValueError("bench capture readback/coverage failed: "+str(case))
        if not all(platform.values()):raise ValueError("D3D12 SM6 platform proof failed")
        payload=Path(native[0]["archive"]);preview=case/"native-F0.png"
        if payload.exists():
            if sha(payload)!=native[0]["archive_sha256"]:raise ValueError("native archive changed")
            if not preview.exists():
                with np.load(payload) as a:
                    x=np.maximum(a["hdr"][...,:3],0);x=x/(1+x)
                    display=255*np.where(x<=.0031308,12.92*x,1.055*x**(1/2.4)-.055)
                Image.fromarray(np.clip(np.rint(display),0,255).astype("u1")).save(preview)
            retention="lossless native archive retained"
        else:
            ledger=json.loads((case/"native-cleanup.json").read_text())
            if ledger[payload.name]["sha256"]!=native[0]["archive_sha256"] or not preview.exists():
                raise ValueError("deleted native payload lacks verified summary/preview provenance")
            retention="large raw HDR deleted after summary; hash and full F0 PNG retained"
        row=dict(bench=bench,config=runtime["config"],case=str(case),scene=scene,camera=camera,
            native=native[0],tracing=runtime["effective_mode"],platform=platform,process_exit=process["exit_code"],
            minimum_available_gib=process["minimum_available_gib"],cost_eligible=False,
            retention=retention,full_F0_png=str(preview),full_F0_png_sha256=sha(preview))
        rows[(bench,runtime["config"])]=row
    missing=sorted(expected-rows.keys())
    save(OUT/"bench-captures.json",dict(complete=not missing,expected=len(expected),
        captured=list(rows.values()),missing=missing,scope="content and quality captures; functional leases, no speed claim"))
    for bench in ("SV","S4","S5","S2G"):
        sheet=Image.new("RGB",(480*5,300*4));draw=ImageDraw.Draw(sheet)
        for index,config in enumerate(ue.CONFIGS):
            row=rows.get((bench,config))
            if row is None:continue
            x,y=(index%5)*480,(index//5)*300
            with Image.open(row["full_F0_png"]) as image:sheet.paste(image.resize((480,270)),(x,y+30))
            draw.text((x+8,y+8),config,fill="white")
        sheet.save(args.root/(bench+"-matrix.png"))
    print(json.dumps(dict(expected=len(expected),captured=len(rows),missing=len(missing))))


def references(args):
    root=OUT/"reference/quarry-final";full=json.loads((OUT/"quarry.json").read_text())
    registered={f["name"]:f for f in full["frames"]};verified={};groups={}
    for group in ("quarry-static","quarry-t1b-witnesses","quarry-unit-sequence"):
        manifest=json.loads((OUT/(group+".json")).read_text());rows=[];missing=[]
        for frame in manifest["frames"]:
            name=frame["name"];source=frame.get("alias_of",name);path=root/(source+".npz")
            if not path.exists():missing.append(name);continue
            if source not in verified:
                meta=json.loads(path.with_suffix(".json").read_text())
                if meta.get("camera_background")!=[0,0,0] or sha(path)!=meta["archive_sha256"]:
                    raise ValueError("reference hash/background mismatch")
                verified[source]=meta
            meta=verified[source]
            for key in ("open","close","fov_y","samples","width","height","sky_only"):
                if frame[key]!=registered[source][key]:raise ValueError("nonidentical reference alias")
            if meta["samples"]!=frame["samples"]:raise ValueError("reference sample budget mismatch")
            rows.append(dict(frame=name,canonical=source,samples=meta["samples"],
                archive_sha256=meta["archive_sha256"],relative_noise_quantiles=meta["noise_relative_quantiles"],
                luminance_noise_rms=meta["noise_luminance_rms"]))
        ranges={}
        for samples in sorted({r["samples"] for r in rows}):
            noise=np.array([r["relative_noise_quantiles"] for r in rows if r["samples"]==samples])
            ranges[str(samples)]=dict(per_frame_median_p95_p99_relative_SE_range=[noise.min(0).tolist(),noise.max(0).tolist()])
        groups[group]=dict(expected=len(manifest["frames"]),ready=len(rows),missing=missing,
            canonical_ready=len({r["canonical"] for r in rows}),noise_ranges_by_spp=ranges,frames=rows)
    save(OUT/"reference-coverage.json",dict(primary_background=[0,0,0],groups=groups,
        noise_definition="absolute split-half luminance difference / 2 divided by mean luminance; quantiles where mean exceeds 1e-4 of image peak; not a confidence interval"))
    print(json.dumps({k:{f:v[f] for f in ("expected","ready","canonical_ready","missing")} for k,v in groups.items()}))


def main():
    p=argparse.ArgumentParser(description=__doc__);s=p.add_subparsers(dest="command",required=True)
    q=s.add_parser("quiet");q.add_argument("--out",type=Path,default=OUT/"quiet-summary.json")
    s.add_parser("annotate-black")
    q=s.add_parser("aliases");q.add_argument("manifest",type=Path)
    q=s.add_parser("floors");q.add_argument("--walk",type=Path,default=OUT/"walk/lumen")
    q.add_argument("--out",type=Path,default=OUT/"physical-floor-summary.json")
    q=s.add_parser("benches");q.add_argument("--root",type=Path,default=OUT/"study/bench-matrix")
    s.add_parser("references")
    a=p.parse_args();globals()[a.command.replace("-","_")](a)


if __name__=="__main__":main()
