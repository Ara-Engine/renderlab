"""Lossless reference archives and independent-reference scores. CPU only; no image fitting.

Archives hold unexposed cd/m^2, real split halves, world/normal/centre identity, centre-
conditioned sun visibility and registered geometric populations. A candidate in pre-exposed
HDR must supply its measured pre-exposure; no inferred gain is accepted by the scorer.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import zipfile

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
    with zipfile.ZipFile(path,"w",compression=zipfile.ZIP_DEFLATED,compresslevel=1) as z:
        for key,value in arrays.items():
            with z.open(key+".npy","w",force_zip64=True) as stream:
                np.lib.format.write_array(stream,np.asarray(value),allow_pickle=False)


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
    with np.load(args.reference) as r:
        ref=(r["L0"]+r["L1"])/2
        se=(r["L0"]-r["L1"])/2
        cand=np.load(args.candidate)["hdr"][...,:3]/args.pre_exposure
        results={}
        for stops in (0,4):
            reference=codes(ref,-14+stops);estimate=codes(cand,-14+stops)
            error=estimate-reference
            # Actual split-half display uncertainty, with the same frozen F0 transform.
            noise=(codes(ref+se,-14+stops)-codes(ref-se,-14+stops))/2
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
         pre_exposure=args.pre_exposure,scope="unexposed physical reference; F0 Reinhard+sRGB codes at -14 and -10 stops",measures=results))


def main():
    p=argparse.ArgumentParser(description=__doc__);s=p.add_subparsers(dest="command",required=True)
    q=s.add_parser("compact");q.add_argument("root",type=Path);q.add_argument("name")
    q=s.add_parser("score");q.add_argument("reference",type=Path);q.add_argument("candidate",type=Path)
    q.add_argument("--pre-exposure",type=float,required=True);q.add_argument("--out",type=Path,required=True)
    a=p.parse_args();globals()[a.command](a)


if __name__=="__main__":main()
