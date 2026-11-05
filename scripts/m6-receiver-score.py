"""R1 frozen-population policy differences, sample replay, and inspection artifacts."""
import argparse
import csv
import importlib.util
import json
from pathlib import Path
import time

import numpy as np
from PIL import Image, ImageDraw

spec=importlib.util.spec_from_file_location("film_score",Path(__file__).with_name("m6-film-score.py"))
f0=importlib.util.module_from_spec(spec);spec.loader.exec_module(f0)
DT=f0.DTYPE

def radiance(directory,pose,direct=False):
    info=json.loads((directory/f"pose-{pose}.json").read_text());w,h=info["size"]
    name=f"pose-{pose}.{'direct' if direct else 'film'}.bin"
    return np.fromfile(directory/name,dtype=DT).reshape(h,w)["radiance"].astype(np.float64)/256

def codes(hdr,stops):
    x=np.maximum(0,hdr[...,:3]*(2**stops));x=x/(1+x)
    return 255*np.where(x<=.0031308,x*12.92,1.055*x**(1/2.4)-.055)

def stats(delta,exact,mask):
    d=delta[mask];e=exact[mask]
    r=f0.lb.summary(d.reshape(-1));r["pixels"]=int(mask.sum())
    r["exact_rms"]=float(np.sqrt(np.mean(e**2))) if e.size else 0
    r["exceeds_exact"]=bool(r.get("rms",0)>r["exact_rms"])
    return r

def replay(directory,pose,reference,direct,mask,exact_vis,exact_hdr,exact_sun):
    h,w=mask.shape;vis_sum=np.zeros((h,w));vis_n=np.zeros((h,w));full=np.zeros((h,w,3));sun=full.copy()
    vis_domain=mask&reference["cover"]
    curves=[]
    for i in range(16):
        raw=np.fromfile(directory/f"samples/{i:02}.visibility.u16",dtype="<u2").reshape(h,w)
        valid=raw!=65535;vis_sum+=np.where(valid,raw/256,0);vis_n+=valid
        full+=np.fromfile(directory/f"samples/{i:02}.rgba16f",dtype="<f2").reshape(h,w,4)[...,:3]
        sun+=np.fromfile(directory/f"samples/{i:02}.sun.rgba16f",dtype="<f2").reshape(h,w,4)[...,:3]
        v=np.divide(vis_sum,vis_n,out=np.ones((h,w)),where=vis_n>0)
        m=vis_domain&(vis_n>0)
        curves.append(dict(N=i+1,visibility_rms=float(np.sqrt(np.mean((v[m]-reference["ref"][m])**2))),
                           radiance_rms=float(np.sqrt(np.mean((full[mask]/(i+1)-reference["hdr"][mask,:3])**2))),
                           sun_rms=float(np.sqrt(np.mean((sun[mask]/(i+1)-direct[mask,:3])**2))),
                           missing=int((vis_domain&~(vis_n>0)).sum())))
    fits={};betas={}
    for key,centre,film in (("visibility_rms",exact_vis,reference["ref"]),("radiance_rms",exact_hdr[...,:3],reference["hdr"][...,:3]),("sun_rms",exact_sun[...,:3],direct[...,:3])):
        slope,intercept=np.polyfit(np.log(np.arange(1,17)),np.log([max(r[key],1e-30) for r in curves]),1)
        beta=float(-slope);amplitude=float(np.exp(intercept));betas[key]=beta
        domain=vis_domain if key=="visibility_rms" else mask
        exact_error=float(np.sqrt(np.mean((centre[domain]-film[domain])**2)))
        fits[key]=dict(beta=beta,amplitude=amplitude,exact_point_rms=exact_error,
                       predicted_frame8=amplitude*8**(-beta),predicted_frame32=amplitude*32**(-beta),
                       observed_first_below_exact=next((r["N"] for r in curves if r[key]<=exact_error),None),
                       model_frames_to_exact=(amplitude/max(exact_error,1e-30))**(1/beta) if beta>0 else None)
    return dict(curve=curves,beta=betas,fits=fits,recompute_predictions=betas["visibility_rms"]<.45 or betas["radiance_rms"]<betas["visibility_rms"])

def crop_sheet(images,centres,path):
    size=65;scale=3;cell=size*scale
    out=Image.new("RGB",(cell*len(centres),(cell+32)*len(images)),(20,27,35));draw=ImageDraw.Draw(out)
    for row,(label,im) in enumerate(images):
        for col,(x,y) in enumerate(centres):
            draw.text((col*cell+3,row*(cell+32)+2),f"{label} [{x},{y}]",fill="white")
            out.paste(im.crop((x-32,y-32,x+33,y+33)).resize((cell,cell),Image.Resampling.NEAREST),(col*cell,row*(cell+32)+32))
    out.save(path)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument("--root",type=Path,default=Path("D:/Coding/renderlab-work/m6-exact-quality/r1"));a=p.parse_args();start=time.perf_counter()
    images=a.root/"images";images.mkdir(exist_ok=True)
    scores=a.root/"scores";scores.mkdir(exist_ok=True)
    result={};rows=[]
    for pose in (0,1):
        old=f0.decode(a.root.parent/f"f0/quarry-minimal-p{pose}",pose)
        baseline=f0.decode(a.root/f"quarry-minimal-p{pose}",pose)
        base_direct=radiance(a.root/f"quarry-minimal-p{pose}",pose,True)
        point=np.load(f0.FROZEN/f"gate-final/reference/pose-{pose}-reference.npz")["ref"]
        pops=f0.populations(old,point)
        pops["backlit"]=old["cover"]&~old["front"]
        pops["all-covered"]=old["cover"]
        pops["image"]=np.ones_like(old["cover"])
        pops["multi-surface"]=old["cover"]&(old["n"]<256)
        pops["background-centre"]=~old["cover"]
        exact_vis=np.fromfile(a.root/f"quarry-minimal-p{pose}/pose-{pose}.exact.u32",dtype="<u4").reshape(old["cover"].shape)/256
        exact_hdr=np.fromfile(a.root/f"quarry-minimal-p{pose}/pose-{pose}.exact.rgba16f",dtype="<f2").reshape(*old["cover"].shape,4).astype(float)
        exact_sun=np.fromfile(a.root/f"quarry-minimal-p{pose}/pose-{pose}.exact.sun.rgba16f",dtype="<f2").reshape(*old["cover"].shape,4).astype(float)
        # The new minimal run adds missing R_D. Its R_F and R_L are independently
        # checked against the immutable F0 before using it as a sun-only baseline.
        identity=dict(visibility_max=float(np.max(np.abs(baseline["ref"]-old["ref"]))),hdr_max=float(np.max(np.abs(baseline["hdr"]-old["hdr"]))),count_mismatches=int((baseline["n"]!=old["n"]).sum()))
        assert identity["visibility_max"]==0 and identity["count_mismatches"]==0,"fixed-10um replay must preserve frozen F0 visibility and accepted counts"
        identity["centre_exact_identity"]={}
        for suffix in ("exact.u32","exact.rgba16f"):
            frozen=(a.root.parent/f"f0/quarry-minimal-p{pose}/pose-{pose}.{suffix}").read_bytes()
            identity["centre_exact_identity"][suffix]=all((a.root/f"quarry-{policy}-p{pose}/pose-{pose}.{suffix}").read_bytes()==frozen for policy in ("minimal","magnitude","viewed"))
            assert identity["centre_exact_identity"][suffix],"original centre Exact changed; stop rather than rebaseline"
        pose_result=dict(minimal_replay=identity,policies={})
        data={};directs={}
        for policy in ("magnitude","viewed"):
            directory=a.root/f"quarry-{policy}-p{pose}"
            d=f0.decode(directory,pose);direct=radiance(directory,pose,True)
            data[policy]=d;directs[policy]=direct
            tables={}
            for channel,candidate,ref,exact in (("R_F",d["ref"],old["ref"],exact_vis),
                                              ("R_L_HDR",d["hdr"][...,:3],old["hdr"][...,:3],exact_hdr[...,:3]),
                                              ("R_D_HDR",direct[...,:3],base_direct[...,:3],exact_sun[...,:3])):
                tables[channel]={}
                transforms=[("",lambda x:x)] if channel=="R_F" else [("",lambda x:x),("_codes",lambda x:codes(x,0)),("_codes_plus4",lambda x:codes(x,4))]
                for suffix,transform in transforms:
                    delta=transform(candidate)-transform(ref);e=transform(exact)-transform(ref)
                    table={}
                    for pop,mask in pops.items():
                        if channel=="R_F":mask=mask&old["cover"]&(old["n"]>0)&(d["n"]>0)
                        row=stats(delta,e,mask);table[pop]=row
                        rows.append(dict(pose=pose,policy=policy,channel=channel+suffix,population=pop,**row))
                    tables[channel+suffix]=table
            convergence={pop:replay(directory,pose,d,direct,pops[pop],exact_vis,exact_hdr,exact_sun) for pop in ("partial","all-covered","image")}
            pose_result["policies"][policy]=dict(populations=tables,convergence=convergence)
            panels=[]
            for label,im in (("R_F",f0.vis(d["ref"],d["cover"])),("R_L",f0.tone(d["hdr"])),("R_L+4",f0.tone(d["hdr"],4)),("R_D",f0.tone(direct)),("R_D+4",f0.tone(direct,4)),("R_F minus minimal x8",f0.error(d["ref"]-old["ref"],d["cover"],d["n"]>0))):
                im.save(images/f"pose-{pose}-{policy}-{label.replace(' ','-')}.png");panels.append((label,im))
            f0.sheet(panels,columns=2,width=960).save(images/f"pose-{pose}-{policy}-overview.png")
        # Choose separated maximum sun-radiance changes within the centre-backlit
        # domain, plus the seven frozen contact probes. Selection is recorded.
        importance=np.sqrt(np.mean((directs["viewed"][...,:3]-directs["magnitude"][...,:3])**2,-1))
        importance=np.where(pops["backlit"],importance,-1)
        centres=[]
        for _ in range(5):
            y,x=np.unravel_index(np.argmax(importance),importance.shape);centres.append((int(x),int(y)))
            importance[max(0,y-65):y+66,max(0,x-65):x+66]=-1
        cropimages=[]
        for channel in ("R_L","R_D"):
            for policy in ("magnitude","viewed"):
                for stops in (0,4):
                    source=data[policy]["hdr"] if channel=="R_L" else directs[policy]
                    cropimages.append((f"{policy} {channel} +{stops}",f0.tone(source,stops)))
        crop_sheet(cropimages,centres,images/f"pose-{pose}-viewed-side-decision-crops.png")
        crop_sheet(cropimages,f0.COORDS,images/f"pose-{pose}-frozen-contact-crops.png")
        pose_result["viewed_crop_coordinates"]=centres
        # The side-rule comparison is R1b minus R1a, isolating sidedness.
        pose_result["side_rule_backlit"]={}
        for channel,v,b,e,frozen in (("R_F",data["viewed"]["ref"],data["magnitude"]["ref"],exact_vis,old["ref"]),
                              ("R_L",data["viewed"]["hdr"][...,:3],data["magnitude"]["hdr"][...,:3],exact_hdr[...,:3],old["hdr"][...,:3]),
                              ("R_D",directs["viewed"][...,:3],directs["magnitude"][...,:3],exact_sun[...,:3],base_direct[...,:3])):
            pose_result["side_rule_backlit"][channel]=stats(v-b,e-frozen,pops["backlit"])
        result[str(pose)]=pose_result
    result["identity"]={}
    for policy in ("minimal","magnitude","viewed"):
        result["identity"][policy]={suffix:(a.root/f"quarry-{policy}-p0/pose-0.{suffix}").read_bytes()==(a.root/f"quarry-{policy}-p2/pose-2.{suffix}").read_bytes() for suffix in ("film.bin","direct.bin","exact.u32","exact.rgba16f","exact.sun.rgba16f")}
    result["seconds"]=time.perf_counter()-start
    (scores/"summary.json").write_text(json.dumps(result,indent=2))
    fields=list(dict.fromkeys(k for row in rows for k in row))
    with (scores/"population-scores.csv").open("w",newline="") as f:
        writer=csv.DictWriter(f,fieldnames=fields);writer.writeheader();writer.writerows(rows)
    print("scored",len(rows),"rows in",result["seconds"],"seconds",flush=True)

if __name__=="__main__":main()
