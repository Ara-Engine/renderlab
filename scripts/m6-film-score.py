"""CPU-only F0 decoding, frozen static Level-F metrics, inspection artifacts.

py -3 -B scripts/m6-film-score.py --root D:/Coding/renderlab-work/m6-exact-quality/f0
No reference inputs are rewritten. Math-library threads should be set to two.
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import sys
import time

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "support"))
import fresh_level_b_metrics as lb
import fresh_level_b as level_b

FROZEN = Path("D:/Coding/renderlab-work/m6-level-b-probe")
NATIVE = Path("D:/Coding/renderlab-work/ue-m5/Quarry/native-shadow")
SPREAD = Path("D:/Coding/renderlab-work/m6-primary-prod/runs/accept-b12/quarry")
COORDS = [(1203,669),(928,325),(1388,529),(1326,607),(945,276),(1577,595),(1254,549)]
DTYPE = np.dtype([("counts", "<u4", (4,)), ("moments", "<f4", (4,)), ("radiance", "<f4", (4,))])


def decode(directory, pose):
    p = directory / f"pose-{pose}"
    info = json.loads(p.with_suffix(".json").read_text())
    w,h = info["size"]
    a = np.fromfile(p.with_suffix(".film.bin"), dtype=DTYPE).reshape(h,w)
    c = a["counts"].astype(float)
    n = c[...,2] + c[...,3]
    ref = np.divide(c[...,0]+c[...,1],256*n,out=np.ones(n.shape),where=n>0)
    halves = [np.divide(c[...,i],256*c[...,i+2],out=np.ones(n.shape),where=c[...,i+2]>0) for i in range(2)]
    # Variance of the weighted full estimate from the two independent halves.
    sigma = np.abs(halves[0]-halves[1])*np.sqrt(c[...,2]*c[...,3])/np.maximum(n,1)
    variance = np.maximum(0, np.divide(a["moments"][...,0],n,out=np.zeros(n.shape),where=n>0)-ref**2)
    world = np.fromfile(p.with_suffix(".world.f32"),dtype="<f4").reshape(h,w,4)
    normal = np.fromfile(p.with_suffix(".normal.f32"),dtype="<f4").reshape(h,w,4)
    hit = np.fromfile(p.with_suffix(".hit.u32"),dtype="<u4").reshape(h,w,4)
    cover = hit[...,0]!=0
    # Population construction uses the complete film reference before native availability.
    eye=np.asarray(info["eye"][:3]);forward=np.asarray(info["forward"][:3])
    depth=np.einsum("ijk,k->ij",world[...,:3]-eye,forward)
    facing = normal[...,:3].astype(float)
    facing *= np.where(np.einsum("ijk,ijk->ij",facing,eye-world[...,:3])<0,-1,1)[...,None]
    front=cover & (facing @ lb.TO_LIGHT>0)
    # Keep the frozen metric input (stored reversed-Z depth) distinct from the
    # physical depth used by the reference's 2% surface acceptance policy.
    return dict(ref=ref,sigma=sigma,depth=world[...,3],surface_depth=depth,cover=cover,front=front,n=n,variance=variance,
                hdr=a["radiance"].astype(float)/256,world=world,hit=hit,info=info)


def populations(d, point):
    p=lb.populations(d)
    p["point-aliased"]=p["partial"] & ((point<=1e-9)|(point>=1-1e-9))
    p["micro"]=d["cover"] & (d["variance"]>0.01)
    return p


def summary(e, noise):
    s=lb.summary(e)
    if len(e):
        s["noise_rms"]=float(np.sqrt(np.mean(noise**2)))
        s["rms_noise_subtracted"]=float(np.sqrt(max(0,np.mean(e**2)-np.mean(noise**2))))
        s["se"]=float(np.std(e,ddof=1)/np.sqrt(len(e))) if len(e)>1 else 0.
    return s


def statistics(candidate,d,pops,valid,domains=("all","front")):
    e=candidate-d["ref"]
    result={}
    for domain,keep in (("all",d["cover"]),("front",d["front"])):
        if domain not in domains:continue
        rows={}
        for name,pop in pops.items():
            target=pop & keep
            m=target & valid & (d["n"]>0)
            rows[name]=summary(e[m],d["sigma"][m]) | dict(reference_n=int(target.sum()),missing_n=int((target&~m).sum()))
        part=pops["partial"] & keep
        segments,centres=lb.contour_segments(e,d,part)
        from scipy.ndimage import maximum_filter
        # Count missing outputs on the complete 9-px support, never insert zero error.
        supported=~maximum_filter((keep&~valid).astype(np.uint8),size=19).astype(bool)
        segvalid=supported[centres]
        seg=segments[segvalid]
        rows["contour"]=lb.summary(seg) | dict(reference_n=int(len(segments)),missing_n=int((~segvalid).sum()))
        rows["rho"]=rows["contour"].get("rms",0)/(rows["partial"].get("rms",0)/3) if rows["partial"].get("rms",0)>0 else 0.
        k=keep & valid & (d["n"]>0)
        rows["missed_shadows"]=int((k&(d["ref"]<=.9)&(candidate>=.99)).sum())
        rows["false_shadows"]=int((k&(d["ref"]>=.99)&(candidate<=.9)).sum())
        rows["complete_domain"]=bool(not (keep&~valid).any() and not (keep&(d["n"]==0)).any())
        result[domain]=rows
    return result


def vis(a,cover):
    rgb=np.repeat(np.rint(np.clip(a,0,1)*255).astype("u1")[...,None],3,-1)
    rgb[~cover]=[20,27,35]
    return Image.fromarray(rgb)


def error(a,cover,valid):
    v=np.clip(a*8,-1,1)
    rgb=np.stack([np.maximum(v,0),np.zeros_like(v),np.maximum(-v,0)],-1)
    rgb[~cover]=np.array([20,27,35])/255
    rgb[cover&~valid]=[0,1,0]
    return Image.fromarray(np.rint(rgb*255).astype("u1"))


def tone(a,stops=0):
    v=np.maximum(0,a[...,:3]*(2**stops));v=v/(1+v)
    v=np.where(v<=.0031308,12.92*v,1.055*v**(1/2.4)-.055)
    return Image.fromarray(np.rint(np.clip(v,0,1)*255).astype("u1"))


def sheet(panels,columns=3,width=640):
    height=int(panels[0][1].height*width/panels[0][1].width)
    out=Image.new("RGB",(width*columns,(height+24)*math.ceil(len(panels)/columns)),(20,27,35))
    draw=ImageDraw.Draw(out)
    for i,(name,im) in enumerate(panels):
        x=i%columns*width;y=i//columns*(height+24)
        draw.text((x+5,y+5),name,fill="white")
        out.paste(im.resize((width,height)),(x,y+24))
    return out


def write_crops(images,pose,policy):
    """Frozen 33-pixel probes and five whole-scene feature regions, at 3x nearest."""
    coords=COORDS+[(720,600),(1400,600),(1210,330),(1040,650),(1575,500)]
    rows=[("Film",f"pose-{pose}-{policy}-reference.png")]+[(k,f"pose-{pose}-{policy}-{k}-visibility.png") for k in ("Exact","Interactive","Unreal-VSM","Unreal-HRT")]+[("Exact error x8",f"pose-{pose}-{policy}-Exact-error-x8.png")]
    crop=Image.new("RGB",(len(coords)*140,6*128),(20,27,35));draw=ImageDraw.Draw(crop)
    for row,(name,path) in enumerate(rows):
        im=Image.open(images/path)
        for col,(x,y) in enumerate(coords):
            draw.text((col*140,row*128),name,fill="white")
            draw.text((col*140,row*128+12),f"{x},{y}",fill="white")
            crop.paste(im.crop((x-16,y-16,x+17,y+17)).resize((99,99),Image.Resampling.NEAREST),(col*140,row*128+28))
    crop.save(images/f"pose-{pose}-{policy}-crops.png")


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root",type=Path,required=True)
    a=parser.parse_args();start=time.perf_counter()
    fixture_validation=json.loads((a.root/"fixture-validation.json").read_text())
    assert fixture_validation["passed"] and len(fixture_validation["cases"])==4,"both fixtures and policies must pass first"
    out=a.root/"scores";images=a.root/"images"
    out.mkdir(exist_ok=True);images.mkdir(exist_ok=True)
    lb.R=FROZEN/"runs/quarry-b6";lb.REFERENCE=FROZEN/"gate-final/reference";lb.NATIVE=NATIVE
    results={};csvrows=[];spread={};policy_difference={}
    for pose in range(3):
        point=np.load(lb.REFERENCE/f"pose-{pose}-reference.npz")["ref"]
        old_exact=lb.counts(pose,0)/256
        hit=np.fromfile(lb.R/f"pose-{pose}-seed-0-hits.u32",dtype="<u4").reshape(*lb.SHAPE,8)
        prefix=hit[...,0].copy().view("u1").reshape(*lb.SHAPE,4)
        blocked=np.unpackbits(prefix,axis=-1,bitorder="little")[...,:8].sum(-1)
        candidates={"Exact":old_exact,"Interactive":(8-blocked.astype(float))/8}
        validity={k:np.ones(lb.SHAPE,bool) for k in candidates}
        for case,name in (("vsm","Unreal-VSM"),("hrt1","Unreal-HRT")):
            native=np.load(lb.native_dir(case,pose)/"native.npz")
            candidates[name]=native["visibility"].astype(float);validity[name]=native["valid"]
        data={}
        for policy in ("policy","minimal"):
            directory=a.root/f"quarry-{policy}-p{pose}"
            d=decode(directory,pose);data[policy]=d
            assert np.array_equal(np.fromfile(directory/f"pose-{pose}.exact.u32",dtype="<u4").reshape(lb.SHAPE)/256,old_exact),"centre Exact changed"
            pops=populations(d,point)
            # Store the reference as a portable CPU artifact with explicit accepted counts.
            np.savez_compressed(out/f"pose-{pose}-{policy}-reference.npz",ref=d["ref"],sigma=d["sigma"],n=d["n"],variance=d["variance"],cover=d["cover"],depth=d["depth"])
            key=f"{pose}/{policy}";result={}
            result["reference"]={"covered":int(d["cover"].sum()),"no_samples":int((d["cover"]&(d["n"]==0)).sum()),"accepted_quantiles":np.quantile(d["n"][d["cover"]],[0,.01,.5,1]).tolist(),"partial_sigma_rms":float(np.sqrt(np.mean(d["sigma"][pops["partial"]]**2)))}
            panels=[("Film visibility",vis(d["ref"],d["cover"])),("Film radiance",tone(d["hdr"])),("Film radiance +4 stops",tone(d["hdr"],4))]
            panels[0][1].save(images/f"pose-{pose}-{policy}-reference.png")
            for label,im in panels[1:]:
                im.save(images/f"pose-{pose}-{policy}-{'hdr+4' if '+4' in label else 'hdr'}.png")
            for name,candidate in candidates.items():
                valid=validity[name]
                result[name]=statistics(candidate,d,pops,valid)
                e=candidate-d["ref"]
                im=vis(candidate,d["cover"]&valid);im.save(images/f"pose-{pose}-{policy}-{name}-visibility.png")
                err=error(e,d["cover"],valid);err.save(images/f"pose-{pose}-{policy}-{name}-error-x8.png")
                panels.append((name+" error x8 (red bright / blue dark)",err))
                missing=d["cover"]&valid&(d["n"]>0)&(d["ref"]<=.9)&(candidate>=.99)
                false=d["cover"]&valid&(d["n"]>0)&(d["ref"]>=.99)&(candidate<=.9)
                with (out/f"pose-{pose}-{policy}-{name}-defects.csv").open("w",newline="") as f:
                    writer=csv.writer(f);writer.writerow(["kind","x","y","reference","candidate","error","sigma"])
                    for kind,m in (("missed",missing),("false",false)):
                        for y,x in zip(*np.nonzero(m)):
                            writer.writerow([kind,int(x),int(y),d["ref"][y,x],candidate[y,x],e[y,x],d["sigma"][y,x]])
                for domain,s in result[name].items():
                    for pop,stat in s.items():
                        if isinstance(stat,dict) and "n" in stat:
                            csvrows.append(dict(pose=pose,policy=policy,method=name,domain=domain,population=pop,**stat))
            sheet(panels).save(images/f"pose-{pose}-{policy}-overview.png")
            litpanels=[]
            for label,hdr in [("Film",d["hdr"])]+[(label,np.fromfile(FROZEN/f"runs/accept-b7/pose-{pose}-{suffix}.rgba16f",dtype="<f2").reshape(*lb.SHAPE,4).astype(float)) for label,suffix in (("Exact","exact"),("Interactive","seed-0-interactive"))]:
                for stops in (0,4):
                    im=tone(hdr,stops);litpanels.append((f"{label} +{stops} stops",im))
            sheet(litpanels,columns=2,width=960).save(images/f"pose-{pose}-{policy}-lit.png")
            write_crops(images,pose,policy)
            # Freeze F1/F3/F7 static calibration from independent production seeds 65..80.
            assert np.array_equal(np.fromfile(SPREAD/f"pose-{pose}-seed-0-production.u32",dtype="<u4").reshape(lb.SHAPE)/256,old_exact)
            draws=[statistics(np.fromfile(SPREAD/f"pose-{pose}-seed-{seed}-production.u32",dtype="<u4").reshape(lb.SHAPE)/256,d,pops,validity["Exact"],domains=("all",))["all"] for seed in range(65,81)]
            calibration={}
            for pop in pops:
                if not result["Exact"]["all"][pop]["n"]:continue
                values=np.array([s[pop]["rms"] for s in draws]);mean=values.mean()
                spread2=float(2*values.std(ddof=1)/mean) if mean else 0.
                calibration[pop]=dict(median_rms=float(np.median(values)),spread_2sd_relative=spread2,F1_exact_seed0_limit=result["Exact"]["all"][pop]["rms"]*(1+spread2),F3_abs_mean_base=abs(result["Exact"]["all"][pop]["mean"]))
            rho0=float(np.median([s["rho"] for s in draws]))
            share=level_b.COHERENT_SHARE
            calibration["F7"]=dict(rho0=rho0,S4_limit=math.sqrt((1-share)*rho0*rho0+9*share))
            calibration["solid_max_bound"]=.002
            spread[key]=calibration;results[key]=result
            print(key,{name:(s["all"]["partial"].get("rms"),s["all"]["missed_shadows"]) for name,s in result.items() if name!="reference"},flush=True)
        diff=data["policy"]["ref"]-data["minimal"]["ref"]
        policy_difference[str(pose)]={}
        for basis in ("policy","minimal"):
            p=populations(data[basis],point);rows={}
            for pop,m in p.items():
                m=m&data["policy"]["cover"]&data["minimal"]["cover"]&(data["policy"]["n"]>0)&(data["minimal"]["n"]>0)
                value=lb.summary(diff[m]);base=results[f"{pose}/{basis}"]["Exact"]["all"][pop]
                value["exceeds_exact_rms"]=bool(value.get("rms",0)>base.get("rms",0))
                rows[pop]=value
            policy_difference[str(pose)][basis]=rows
    fields=["pose","policy","method","domain","population","reference_n","n","missing_n","rms","rms_noise_subtracted","mean","se","p99","max","noise_rms"]
    with (out/"population-scores.csv").open("w",newline="") as f:
        writer=csv.DictWriter(f,fields,extrasaction="ignore");writer.writeheader();writer.writerows(csvrows)
    policy_errors=[results[f"{p}/policy"]["Exact"]["all"]["partial"]["rms"] for p in range(3)]
    verdict="finding_wrong_stop_Exact_8.4-10.9_ms" if any(e<.02 for e in policy_errors) else "finding_holds" if all(e>=.05 for e in policy_errors) else "intermediate_recompute_budget"
    receiver_defect=any(s["exceeds_exact_rms"] for p in policy_difference.values() for s in p["policy"].values())
    result=dict(cases=results,policy_difference=policy_difference,static_calibration=spread,partial_policy_errors=policy_errors,verdict=verdict,receiver_policy_unit_first=receiver_defect,seconds=time.perf_counter()-start,
                limits=["Static independently warmed poses only: no dynamic F2/F5/F6/F8/F9 certification.","Split halves include spatial integration error; noise-subtracted scores are diagnostic.","Native masks with missing geometry remain incomplete; no complete-domain native pass.","F4 native persistence is not established by a single saved mask."])
    (out/"summary.json").write_text(json.dumps(result,indent=2,allow_nan=False))
    hashes=[]
    for directory in (out,images):
        for path in sorted(directory.iterdir()):
            hashes.append(f"{hashlib.file_digest(path.open('rb'),'sha256').hexdigest()}  {path}")
    (a.root/"identities.sha256").write_text("\n".join(hashes)+"\n")
    print(verdict,"receiver policy unit first",receiver_defect,"seconds",result["seconds"],flush=True)


if __name__=="__main__":
    main()
