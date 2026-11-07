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


def score(args):
    metadata=json.loads(args.reference.with_suffix(".json").read_text())
    if metadata.get("camera_background")!=[0,0,0] or metadata.get("archive_sha256")!=sha(args.reference):
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
                    p99=float(np.sqrt(np.quantile(values,.99))),max=float(np.sqrt(values.max())),signed_mean=float(error[mask].mean()))
    save(args.out,dict(reference_sha256=sha(args.reference),candidate_sha256=sha(args.candidate),
         pre_exposure=args.pre_exposure,exposure_stops=args.stops,
         scope="unexposed physical reference; F0 Reinhard+sRGB codes at registered exposure and +4 stops",measures=results))


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
        selected=[(key,name) for key,name in inputs.items() if key in ("TSR.Output","TSR.History.Color","SceneColor")]
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
    q=s.add_parser("score");q.add_argument("reference",type=Path);q.add_argument("candidate",type=Path)
    q.add_argument("--pre-exposure",type=float,required=True);q.add_argument("--out",type=Path,required=True)
    q.add_argument("--stops",type=int,default=-14)
    q=s.add_parser("decode");q.add_argument("dump",type=Path);q.add_argument("out",type=Path);q.add_argument("--stops",type=int,default=-14)
    a=p.parse_args();globals()[a.command](a)


if __name__=="__main__":main()
