"""Lossless reference archives and independent-reference scores. CPU only; no image fitting.

Archives hold unexposed cd/m^2, real split halves, world/normal/centre identity, centre-
conditioned sun visibility and registered geometric populations. A candidate in pre-exposed
HDR must supply its measured pre-exposure; no inferred gain is accepted by the scorer.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import zipfile
import re

import numpy as np
from PIL import Image

sys.path.insert(0,str(Path(__file__).resolve().parent.parent/"support"))
import fresh_level_b_metrics as metrics


def save(path,value):
    path.write_text(json.dumps(value,indent=2,allow_nan=False)+"\n")


def sha(path):
    with path.open("rb") as stream:return hashlib.file_digest(stream,"sha256").hexdigest()


def codes(x,stops=0):
    x=np.maximum(x*2**stops,0);x=x/(1+x)
    return 255*np.where(x<=.0031308,12.92*x,1.055*x**(1/2.4)-.055)


def archive(path,arrays):
    temporary=path.with_suffix(path.suffix+".tmp")
    with zipfile.ZipFile(temporary,"w",compression=zipfile.ZIP_DEFLATED,compresslevel=1) as z:
        for key,value in arrays.items():
            with z.open(key+".npy","w",force_zip64=True) as stream:
                np.lib.format.write_array(stream,np.asarray(value),allow_pickle=False)
    temporary.replace(path)


def compact(args):
    root,name=args.root,args.name
    meta=json.loads((root/(name+".json")).read_text());h,w=meta["height"],meta["width"];n=meta["samples"]
    raw=[root/f"{name}-{suffix}" for suffix in ("mean.f32","se.f32","half0.f32","half1.f32","geometry.bin")]
    halves=[np.fromfile(root/f"{name}-half{k}.f32","<f4").reshape(h,w,3) for k in (0,1)]
    L=(halves[0]+halves[1])/2
    se=(halves[0]-halves[1])/2
    arrays={"L0":halves[0],"L1":halves[1]}
    pops={"image":np.ones((h,w),bool)}
    if raw[-1].exists():
        dtype=np.dtype([("world","<f4",4),("normal","<f4",4),("hit","<u4",4),("direct0","<f4",4),("direct1","<f4",4)])
        a=np.fromfile(raw[-1],dtype).reshape(h,w)
        count=a["hit"][...,3];counts=[a["world"][...,3],count-a["world"][...,3]]
        visibility=np.divide(a["direct0"][...,3]+a["direct1"][...,3],count,out=np.zeros((h,w),"f4"),where=count>0)
        cover=a["hit"][...,0]>0
        if np.any(count>n) or np.any(counts[0]>(n+1)//2) or np.any(counts[1]>n//2):raise ValueError("invalid accepted-sample counts")
        if not np.isfinite(a["world"]).all() or not np.isfinite(a["normal"]).all():raise ValueError("nonfinite geometry")
        pose=meta["frame"]["open"];eye=np.array(pose["eye"]);forward=np.array(pose["target"])-eye;forward/=np.linalg.norm(forward)
        depth=((a["world"][...,:3]-eye)*forward).sum(-1)
        d=dict(ref=visibility,depth=depth,cover=cover)
        pops=metrics.populations(d)
        pops|={"backlit":cover&(a["normal"][...,3]<=0),"all-covered":cover,"image":np.ones((h,w),bool),
               "multi-surface":cover&(count<n),"background-centre":~cover,
               "solid-lit":pops["lit"]&(count==n),"solid-occluded":pops["occluded"]&(count==n),"single-surface":cover&(count==n)}
        placement=a["hit"][...,0];pad=np.pad(placement,1,mode="edge");edge=np.zeros((h,w),bool)
        for dy in (-1,0,1):
            for dx in (-1,0,1):edge|=pad[1+dy:1+dy+h,1+dx:1+dx+w]!=placement
        pops["edge"]=edge
        arrays|={"world":a["world"][...,:3],"geometric_normal":a["normal"][...,:3],"hit":a["hit"][...,:3],"n":count,
                 "visibility":visibility,"depth":depth.astype("f4")}
        for k in (0,1):
            arrays[f"D{k}"]=a[f"direct{k}"][...,:3]/((n+1-k)//2)
            arrays[f"V{k}"]=np.divide(a[f"direct{k}"][...,3],counts[k],out=np.zeros((h,w),"f4"),where=counts[k]>0)
    arrays|={"pop_"+k:v for k,v in pops.items()}
    target=root/(name+".npz");archive(target,arrays)
    stops=-14 if "frame" in meta else -9
    if "bench" in meta:
        source=Path("E:/renderlab-work/m7-u0/scenes")/meta["bench"]/"scene.json"
        if source.exists():stops=json.loads(source.read_text()).get("exposure_stops",stops)
    stops=meta.get("exposure_stops",stops)
    Image.fromarray(np.rint(np.clip(codes(L,stops),0,255)).astype("u1")).save(root/(name+".png"))
    lum=(L*np.array([.2126,.7152,.0722])).sum(-1)
    noise=np.abs((se*np.array([.2126,.7152,.0722])).sum(-1))
    lit=lum>max(float(lum.max())*1e-4,1e-8)
    relative=np.divide(noise,np.maximum(lum,1e-20))[lit]
    meta|=dict(archive_sha256=sha(target),radiance_units="unexposed cd/m^2",noise_relative_quantiles=np.quantile(relative,[.5,.95,.99]).tolist() if relative.size else [],
               noise_luminance_rms=float(np.sqrt((noise**2).mean())),populations={k:int(v.sum()) for k,v in pops.items()},
               deleted_raw={p.name:dict(sha256=sha(p),bytes=p.stat().st_size) for p in raw if p.exists()},
               direct_definition="physical sun-only direct radiance; includes disk integration (not the old delta-BRDF T1b estimand)",
               visibility_definition="disk visibility conditioned on centre placement and 2% forward-depth band; single-surface mask has every sample accepted")
    save(root/(name+".json"),meta)
    for p in raw:
        if p.exists():p.unlink()
    print(json.dumps(dict(reference=name,samples=n,noise=meta["noise_relative_quantiles"],bytes=target.stat().st_size)),flush=True)


def combine(args):
    """Combine independent even-sample batches without averaging duplicate seeds."""
    fields=("bench","pose","scene_sha256","smith","max_depth","camera_background","width","height")
    recipe=None;seeds=set();total=0;halves=None;inputs=[]
    for path in args.inputs:
        meta=json.loads(path.with_suffix(".json").read_text());n=meta["samples"]
        if (meta.get("accepted_for_scoring") is False or meta.get("camera_background")!=[0,0,0]
            or meta.get("archive_sha256")!=sha(path) or n<2 or n%2):
            raise ValueError("batch must have verified black-primary metadata, hash and even sample count")
        identity={k:meta[k] for k in fields}
        if recipe is None:recipe=identity
        elif identity!=recipe:raise ValueError("batch scene/settings mismatch")
        if meta["seed"] in seeds:raise ValueError("duplicate seed would not add samples")
        seeds.add(meta["seed"])
        with np.load(path) as arrays:
            values=[arrays[k].astype("f8") for k in ("L0","L1")]
            if any(v.shape!=(meta["height"],meta["width"],3) for v in values):raise ValueError("batch shape mismatch")
            if any(not np.isfinite(v).all() for v in values):raise ValueError("nonfinite batch")
            if halves is None:halves=[v*n for v in values]
            else:
                for i,v in enumerate(values):halves[i]+=v*n
        total+=n;inputs.append(dict(path=str(path),archive_sha256=meta["archive_sha256"],seed=meta["seed"],samples=n))
    if halves is None:raise ValueError("no batches")
    args.root.mkdir(parents=True,exist_ok=True)
    for i,values in enumerate(halves):(values/total).astype("<f4").tofile(args.root/f"{args.name}-half{i}.f32")
    save(args.root/(args.name+".json"),dict(recipe,samples=total,independent_seed_batches=inputs,
        accumulation="sample-weighted actual even/odd batch means, accumulated in f64 then rounded once to f32; no duplicate seeds",
        accepted_for_scoring=True))
    compact(argparse.Namespace(root=args.root,name=args.name))


def score(args):
    metadata=json.loads(args.reference.with_suffix(".json").read_text())
    if metadata.get("accepted_for_scoring") is False or metadata.get("camera_background")!=[0,0,0] or metadata.get("archive_sha256")!=sha(args.reference):
        raise ValueError("reference needs verified black-primary metadata and archive hash")
    with np.load(args.reference) as r:
        ref=(r["L0"]+r["L1"])/2
        se=(r["L0"]-r["L1"])/2
        with np.load(args.candidate) as candidate:
            if float(candidate["pre_exposure"])!=args.pre_exposure:
                raise ValueError("requested pre-exposure differs from captured native value")
            cand=candidate["hdr"][...,:3]/args.pre_exposure
        if cand.shape!=ref.shape or not np.isfinite(cand).all():raise ValueError("invalid candidate")
        results={}
        for stops in (0,4):
            reference=codes(ref,args.stops+stops);estimate=codes(cand,args.stops+stops)
            error=estimate-reference
            # Actual split-half display uncertainty, with the same frozen F0 transform.
            noise=(codes(ref+se,args.stops+stops)-codes(ref-se,args.stops+stops))/2
            e2=(error**2).mean(-1);v2=(noise**2).mean(-1)
            results[str(stops)]={}
            for key in r.files:
                if not key.startswith("pop_"):continue
                mask=r[key]
                if not mask.any():continue
                values=e2[mask]
                results[str(stops)][key[4:]]=dict(n=int(mask.sum()),rms=float(np.sqrt(values.mean())),
                    rms_noise_subtracted=float(np.sqrt(max(0,float(values.mean()-v2[mask].mean())))),
                    reference_noise_rms=float(np.sqrt(v2[mask].mean())),
                    p99=float(np.sqrt(np.quantile(values,.99))),max=float(np.sqrt(values.max())),signed_mean=float(error[mask].mean()))
    save(args.out,dict(reference_sha256=sha(args.reference),candidate_sha256=sha(args.candidate),
         pre_exposure=args.pre_exposure,exposure_stops=args.stops,
         statistics_definitions=dict(rms="RGB component RMS over the population",p99="p99 of per-pixel RGB RMS",max="maximum per-pixel RGB RMS"),
         scope="unexposed physical reference; F0 Reinhard+sRGB codes at registered exposure and +4 stops",measures=results))


def static_floor(args):
    """Score retained static native HDR only after camera and runtime readbacks pass."""
    rows=[]
    for directory in sorted(args.root.iterdir()):
        if not directory.is_dir() or not (directory/"camera-readback.json").exists():continue
        runtime=json.loads((directory/"readback.json").read_text())
        camera=json.loads((directory/"camera-readback.json").read_text())
        process=json.loads((directory/"process.json").read_text())
        if not runtime["all_ok"] or not camera["all_ok"] or process["exit_code"]:
            raise ValueError("static capture readback failed: "+str(directory))
        output=directory/"native.npz"
        if not output.with_suffix(".json").exists():
            decode(argparse.Namespace(dump=directory/"hdr",out=output,stops=args.stops))
        native=json.loads(output.with_suffix(".json").read_text())["frames"]
        if len(native)!=1 or native[0]["archive_sha256"]!=sha(output):
            raise ValueError("static native archive identity/coverage failed")
        result=directory/"physical-score.json"
        score(argparse.Namespace(reference=args.reference,candidate=output,
              pre_exposure=native[0]["pre_exposure"],stops=args.stops,out=result))
        with np.load(args.reference) as reference,np.load(output) as candidate:
            ref=codes((reference["L0"]+reference["L1"])/2,args.stops)
            estimate=codes(candidate["hdr"][...,:3]/native[0]["pre_exposure"],args.stops)
            pixels=np.concatenate([ref,estimate,8*np.abs(estimate-ref)],axis=1)
            Image.fromarray(np.rint(np.clip(pixels,0,255)).astype("u1")).save(directory/"physical.png")
        rows.append(dict(config=runtime["config"],effective_mode=runtime["effective_mode"],
                         case=str(directory),**json.loads(result.read_text())))
        print("scored static",runtime["config"],flush=True)
    save(args.root/"physical-static-floor.json",dict(
        scope="physical combined material/lighting/reconstruction error; black primary; visible-sky silhouette behavior untested",
        frames=rows))


def floor(args):
    """Physical combined-error floors, with explicit missing cameras. Internal TSR ages
    are unavailable; this is not an isolated reconstruction or visibility comparison."""
    def key(p):return np.asarray(p["eye"]+p["target"]+[p.get("fov_y",p.get("fov_y_radians",.85))],"<f4").tobytes()
    selection=getattr(args,"selection",None)
    selected=None if selection is None else {
        int(f["name"].removeprefix("q-walk-")) for f in json.loads(selection.read_text())["frames"]
        if f["name"].startswith("q-walk-") and not f["name"].endswith("-shutter")}
    refs={}
    for path in args.reference.glob("*.json"):
        meta=json.loads(path.read_text());frame=meta.get("frame")
        archive_path=path.with_suffix(".npz")
        if not frame or not archive_path.exists() or frame["open"]!=frame["close"] or frame.get("sky_only"):continue
        if meta.get("accepted_for_scoring") is False or meta.get("camera_background")!=[0,0,0] or sha(archive_path)!=meta["archive_sha256"]:
            raise ValueError("unverified reference "+str(path))
        pose=dict(frame["open"],fov_y=frame["fov_y"]);k=key(pose)
        if k not in refs or meta["samples"]>refs[k][1]["samples"]:refs[k]=(archive_path,meta)
    def measures(error,noise,populations):
        e2=(error**2).mean(-1);v2=(noise**2).mean(-1);result={}
        for name,mask in populations.items():
            if not mask.any():continue
            result[name]=dict(n=int(mask.sum()),rms=float(np.sqrt(e2[mask].mean())),
                rms_noise_subtracted=float(np.sqrt(max(0,float((e2-v2)[mask].mean())))),
                reference_noise_rms=float(np.sqrt(v2[mask].mean())),
                p99=float(np.sqrt(np.quantile(e2[mask],.99))),maximum=float(np.abs(error[mask]).max()),
                signed_mean=float(error[mask].mean()))
        return result
    for directory in sorted(args.root.iterdir()):
        if not directory.is_dir() or not (directory/"native.json").exists():continue
        recipe=json.loads((directory/"recipe.json").read_text())
        if getattr(args,"configs",None) and recipe["config"] not in args.configs:continue
        camera=json.loads((directory/"camera-readback.json").read_text())
        runtime=json.loads((directory/"readback.json").read_text())
        native=json.loads((directory/"native.json").read_text())["frames"]
        if not camera["all_ok"] or not runtime["all_ok"] or len(native)!=len(recipe["frames"]):
            raise ValueError("capture coverage/readback failed: "+str(directory))
        if selected is not None and not selected.issubset(range(len(native))):
            raise ValueError("requested witness outside captured sequence")
        rows=[];residuals=[];missing=[];previous=None
        for index,(pose,capture) in enumerate(zip(recipe["frames"],native,strict=True)):
            if selected is not None and index not in selected:previous=None;continue
            source=refs.get(key(pose))
            if source is None:missing.append(index);previous=None;continue
            reference_path,meta=source
            candidate_path=Path(capture["archive"])
            if sha(candidate_path)!=capture["archive_sha256"]:raise ValueError("candidate changed")
            with np.load(candidate_path) as candidate:
                exposure=float(candidate["pre_exposure"])
                if exposure!=capture["pre_exposure"] or exposure!=2.**args.stops:raise ValueError("pre-exposure drift")
                hdr=candidate["hdr"][...,:3]/exposure
            with np.load(reference_path) as reference:
                halves=[reference[k] for k in ("L0","L1")];mean=(halves[0]+halves[1])/2
                if mean.shape!=hdr.shape or not np.isfinite(hdr).all():raise ValueError("invalid native HDR")
                populations={k[4:]:reference[k] for k in reference.files if k.startswith("pop_")}
                hit=reference["hit"][...,0]
            row=dict(frame=index,phase=pose.get("phase"),reference=str(reference_path),
                     reference_sha256=meta["archive_sha256"],samples=meta["samples"],
                     candidate_sha256=capture["archive_sha256"],measures={})
            for offset in (0,4):
                ref=codes(mean,args.stops+offset);estimate=codes(hdr,args.stops+offset)
                split=[codes(h,args.stops+offset) for h in halves]
                row["measures"][str(offset)]=measures(estimate-ref,(split[0]-split[1])/2,populations)
                if previous is not None and previous["frame"]==index-1:
                    old=previous["display"][offset]
                    common=populations["all-covered"]&previous["covered"]&(hit==previous["hit"])
                    mask={name:value&common for name,value in populations.items()}
                    residual=estimate-old[0]-(ref-old[1])
                    noise=((split[0]-old[2][0])-(split[1]-old[2][1]))/2
                    residuals.append(dict(frames=[index-1,index],offset=offset,phase=pose.get("phase"),
                        static_camera=key(pose)==key(recipe["frames"][index-1]),
                        measures=measures(residual,noise,mask)))
                if offset==0 and index in (0,48,64,120,128,160):
                    pixels=np.concatenate([ref,estimate,8*np.abs(estimate-ref)],axis=1)
                    Image.fromarray(np.rint(np.clip(pixels,0,255)).astype("u1")).save(directory/f"physical-{index:03}.png")
            previous=dict(frame=index,covered=populations["all-covered"],hit=hit,
                          display={o:(codes(hdr,args.stops+o),codes(mean,args.stops+o),
                              [codes(h,args.stops+o) for h in halves]) for o in (0,4)})
            rows.append(row)
        result=dict(config=recipe["config"],effective_mode=runtime["effective_mode"],
            statistics_definitions=dict(rms="RGB component RMS over the population",p99="p99 of per-pixel RGB RMS",maximum="maximum absolute RGB component"),
            scope="physical combined foreground/material/lighting/reconstruction error; black primary before accumulation; visible-sky silhouette behavior untested",
            motion_definition="consecutive F0 candidate step minus physical-reference step; common covered placement; no internal-age filter; split-half residual noise reported/subtracted",
            age_filter_available=False,sequence_frames=len(native),
            native_manifest_sha256=sha(directory/"native.json"),recipe_sha256=sha(directory/"recipe.json"),
            selection=str(selection) if selection else "all sequence frames",selection_sha256=sha(selection) if selection else None,
            complete=not missing,missing_frames=missing,frames=rows,motion=residuals)
        save(directory/"physical-floor.json",result)
        print(json.dumps(dict(config=recipe["config"],scored=len(rows),missing=len(missing),motion_rows=len(residuals))),flush=True)


def decode(args):
    """Decode the actual Tonemap input and its versioned EyeAdaptationBuffer. Reject
    adaptation, local-exposure or tint drift instead of fitting an image-domain gain."""
    def load(name):
        spec=importlib.util.spec_from_file_location(name.replace("-","_"),Path(__file__).with_name(name+".py"))
        m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
    native=load("ue-shadow-dump");s1=load("bench-contact-fold-score")
    dump=native.Dump(args.dump)
    def version(name,consumer):
        writer="00000000000000000000"
        for p in dump.passes:
            if p["Id"]>=consumer["Id"]:break
            if name in p["OutputResources"]:writer=p["Pointer"]
        path=dump.root/"Resources"/f"{name}.v{writer}.bin"
        if not path.exists():
            candidates=list((dump.root/"Resources").glob(name+".v*.bin"))
            if len(candidates)!=1:raise ValueError("ambiguous/missing input version "+name)
            path=candidates[0]
        return path
    results=[]
    for event in dump.passes:
        if "Tonemap" not in event["EventName"]:continue
        inputs=dump.inputs(event)
        selected=[(key,name) for key,name in inputs.items() if key in
                  ("TSR.Output","TSR.History.Color","SceneColor","PostDOFTranslucency.SceneColor")]
        if len(selected)!=1:raise ValueError("ambiguous Tonemap colour input "+str(selected))
        key,name=selected[0];desc=dump.resources[name.split(".")[0]];path=version(name,event)
        h,w=desc["ExtentY"],desc["ExtentX"]
        if desc["Format"]=="PF_FloatRGBA":
            values=np.fromfile(path,"<f2").reshape(-1,h,w,4).astype("f4")
        elif desc["Format"]=="PF_FloatR11G11B10":
            values=s1.unpack_r11g11b10(np.fromfile(path,"<u4").reshape(-1,h,w))
        else:raise ValueError("unreviewed HDR format "+desc["Format"])
        if len(values)>1:
            match=re.search(r"(?:array|slice)(\d+)",name,re.I)
            if match is None:raise ValueError("array input needs an explicit captured slice: "+name)
            values=values[int(match.group(1))]
        else:values=values[0]
        eye_name=inputs["EyeAdaptationBuffer"];eye_path=version(eye_name,event)
        eye=np.fromfile(eye_path,"<f4")
        params=dump.scalar_parameters(event)
        manual=params["Tonemap.EyeAdaptation.ExposureCompensationSettings"]
        tint=params["Tonemap.ColorScale0"][:3]
        if eye.size!=8 or not np.isfinite(eye).all() or not np.isfinite(values).all():raise ValueError("nonfinite native readback")
        if manual!=2.0**args.stops or eye[0]!=manual or eye[4]!=1 or tint!=[1,1,1]:
            raise ValueError(f"fixed manual exposure proof failed: {manual}, {eye.tolist()}, {tint}")
        pre_exposure=float(manual*eye[4])
        frame=next((int(m.group(1)) for scope in event["ParentEventScopes"] if (m:=re.match(r"Frame (\d+)",scope))),0)
        target=args.out if len([p for p in dump.passes if "Tonemap" in p["EventName"]])==1 else args.out.with_name(args.out.stem+f"-{frame}"+args.out.suffix)
        archive(target,dict(hdr=values,pre_exposure=np.float32(pre_exposure)))
        results.append(dict(engine_frame=frame,archive=str(target),archive_sha256=sha(target),resource_name=key,
             consumed_resource=name,resource_sha256=sha(path),format=desc["Format"],shape=list(values.shape),
             pre_exposure=pre_exposure,eye_adaptation_buffer=eye.tolist(),eye_buffer_sha256=sha(eye_path),
             manual_exposure=manual,tonemap_color_scale=tint,
             proof="fixed manual FViewInfo::UpdatePreExposure factors: manual global exposure, unit scene tint, prior local exposure 1; captured EyeAdaptationBuffer and Tonemap parameters"))
    if not results:raise ValueError("no Tonemap captured")
    frames=[r["engine_frame"] for r in results]
    if len(frames)>1 and frames!=list(range(frames[0],frames[0]+len(frames))):
        raise ValueError("native HDR sequence must contain ordered consecutive engine frames")
    save(args.out.with_suffix(".json"),dict(frames=results,dump=str(dump.root)))
    removed={}
    for path in (dump.root/"Resources").glob("*.bin"):
        if path.stat().st_size>5*1024**2:
            removed[path.name]=dict(bytes=path.stat().st_size,sha256=sha(path));path.unlink()
    save(args.out.with_suffix(".deleted.json"),removed)
    print(json.dumps(results,indent=1))


def main():
    p=argparse.ArgumentParser(description=__doc__);s=p.add_subparsers(dest="command",required=True)
    q=s.add_parser("compact");q.add_argument("root",type=Path);q.add_argument("name")
    q=s.add_parser("combine");q.add_argument("root",type=Path);q.add_argument("name");q.add_argument("inputs",nargs="+",type=Path)
    q=s.add_parser("score");q.add_argument("reference",type=Path);q.add_argument("candidate",type=Path)
    q.add_argument("--pre-exposure",type=float,required=True);q.add_argument("--out",type=Path,required=True)
    q.add_argument("--stops",type=int,default=-14)
    q=s.add_parser("decode");q.add_argument("dump",type=Path);q.add_argument("out",type=Path);q.add_argument("--stops",type=int,default=-14)
    q=s.add_parser("floor");q.add_argument("root",type=Path);q.add_argument("reference",type=Path);q.add_argument("--stops",type=int,default=-14)
    q.add_argument("--selection",type=Path)
    q.add_argument("--configs",nargs="+")
    q=s.add_parser("static_floor");q.add_argument("root",type=Path);q.add_argument("reference",type=Path);q.add_argument("--stops",type=int,default=-14)
    a=p.parse_args();globals()[a.command](a)


if __name__=="__main__":main()
