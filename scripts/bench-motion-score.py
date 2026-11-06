"""Streaming S2/S3 native-mask decoding, per-frame film scores and motion crops."""
import argparse
import csv
import hashlib
import importlib.util
import json
from pathlib import Path
import time

import numpy as np
from PIL import Image


def load(name):
    spec=importlib.util.spec_from_file_location(name.replace("-","_"),Path(__file__).with_name(name+".py"))
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module


s1=load("bench-contact-fold-score")


def save(path,data):
    path.write_text(json.dumps(data,indent=2,allow_nan=False)+"\n")


def cut(a,scene):
    x0,y0,x1,y1=scene["crop"];return a[y0:y1,x0:x1]


def old_sun_projection(candidate,current,previous,use):
    delta=(previous-current)[use];denominator=float(np.sum(delta**2))
    return float(np.sum((candidate-current)[use]*delta)/denominator) if denominator else None


def resource(dump,name,event,rect=None):
    desc=dump.resources[name.split(".")[0]]
    if desc["Format"]!="PF_FloatR11G11B10":
        try:values,identity=dump.resource(name,event)
        except FileNotFoundError:
            # A leaf-only capture may omit the producer's pass metadata. A
            # unique captured input version is unambiguous; never choose a
            # lexicographically latest version among several candidates.
            paths=list((dump.root/"Resources").glob(name+".v*.bin"))
            if len(paths)!=1:raise
            path=paths[0];height,width=desc["ExtentY"],desc["ExtentX"];fmt=desc["Format"]
            if fmt=="PF_DepthStencil":values=np.fromfile(path,dtype="<f4").reshape(height,width)
            elif fmt in ("PF_B8G8R8A8","PF_R8G8B8A8"):
                values=np.fromfile(path,dtype="u1").reshape(height,width,4)/255
                if fmt=="PF_B8G8R8A8":values=values[...,[2,1,0,3]]
            elif fmt=="PF_FloatRGBA":values=np.fromfile(path,dtype="<f2").reshape(height,width,4).astype("f4")
            else:raise ValueError("unreviewed filtered input format "+fmt)
            identity=dict(resource=name,version=path.stem.split(".v")[-1],path=str(path),sha256=s1.native.digest(path),description=desc,resolution="unique input version; producer outside filtered tree")
        if rect is not None:
            x0,y0,x1,y1=rect;values=values[y0:y1,x0:x1]
        return values,identity
    writer="00000000000000000000"
    for earlier in dump.passes:
        if earlier["Id"]>=event["Id"]:break
        if name in earlier["OutputResources"]:writer=earlier["Pointer"]
    path=dump.root/"Resources"/f"{name}.v{writer}.bin"
    packed=np.fromfile(path,dtype="<u4").reshape(desc["ExtentY"],desc["ExtentX"])
    if rect is not None:
        x0,y0,x1,y1=rect;packed=packed[y0:y1,x0:x1]
    value=s1.unpack_r11g11b10(packed)
    return value,dict(resource=name,version=writer,path=str(path),sha256=s1.native.digest(path),description=desc)


def reference(directory,index,scene,pose):
    prefix=directory/f"pose-{index}";cache=prefix.with_suffix(".npz")
    if cache.exists():
        with np.load(cache) as a:return {key:a[key] for key in a.files}
    d=s1.film.decode(directory,index);d["depth"]=d["surface_depth"]
    n=np.fromfile(prefix.with_suffix(".normal.f32"),dtype="<f4").reshape(*d["ref"].shape,4)[...,:3].astype(float)
    n*=np.where(np.einsum("ijk,ijk->ij",n,np.array(pose["eye"])-d["world"][...,:3])<0,-1,1)[...,None]
    d["front"]=d["cover"]&(n@np.array(pose["to_light"])>0)
    keep={k:d[k] for k in ("ref","sigma","depth","cover","front","n","hdr","world","hit")}
    temporary=cache.with_suffix(".npz.tmp")
    with temporary.open("wb") as f:np.savez_compressed(f,**keep)
    temporary.replace(cache)
    for suffix in ("film.bin","world.f32","normal.f32","hit.u32"):
        path=prefix.with_suffix("."+suffix);path.unlink()
    return keep


def frame_score(candidate,d,pops,valid,hdr=None):
    row=s1.scored(candidate,d,pops,valid)
    if hdr is not None:row["radiance"]=s1.radiance(hdr,d["hdr"])
    row["reference_partial_sigma_rms"]=float(np.sqrt(np.mean(d["sigma"][pops["partial"]]**2))) if pops["partial"].any() else None
    return row


def read_hdr(path,shape):return np.fromfile(path,dtype="<f2").reshape(*shape,4).astype("f4")


def score_case(root,case,renderlab_only=False):
    started=time.monotonic()
    scene=json.loads((root/"fixture/scene.json").read_text());track=next(t for t in scene["tracks"] if case.startswith(t["name"]+"-r"))
    directory=root/"renderlab"/case;images=root/"images";images.mkdir(exist_ok=True)
    full_reference=directory/"film-full.rgba32f"
    if full_reference.exists():
        full=np.fromfile(full_reference,dtype="<f4").reshape(scene["size"][1],scene["size"][0],4)
        s1.film.tone(full).save(directory/"film-full.png");s1.film.tone(full,4).save(directory/"film-full-plus4.png")
        save(directory/"film-full-identity.json",dict(sha256=hashlib.file_digest(full_reference.open("rb"),"sha256").hexdigest(),scope="Frame 0 full S1 footprint reference, independent of crop-only scoring"));full_reference.unlink()
    x0,y0,x1,y1=scene["crop"];shape=(y1-y0,x1-x0)
    result=[];history=[];snapshots={};previous={};jump={}
    for pose in track["frames"]:
        index=pose["frame"];d=reference(directory/"reference",index,scene,pose);pops=s1.metrics.populations(d)
        refs=d["ref"];panels=[("Film",s1.film.tone(d["hdr"])),("Film visibility",s1.film.vis(refs,d["cover"]))]
        modes={}
        for mode in ("hard","Exact","Interactive","Temporal","Virtual"):
            path=directory/mode
            if not path.is_dir():continue
            hdr=read_hdr(path/f"{index:04}.rgba16f",shape)
            if mode=="Temporal" and not (path/f"{index:04}-visibility.rgba16f").exists():
                info=json.loads((path/f"{index:04}.json").read_text())
                if info["reset"]:history=[]
                packed=read_hdr(path/f"{index:04}-sample.rgba16f",shape)
                sample=packed[...,0];sample_depth=packed[...,1]+packed[...,2]-packed[...,1]/1024
                hit=np.fromfile(path/f"{index:04}-hit.u32",dtype="<u4").reshape(*shape,4)
                history.append((sample,hit[...,0].copy(),sample_depth))
                accepted=np.zeros(shape,dtype="u4");total=np.zeros(shape)
                for visibility,placement,depth in history:
                    use=(placement==d["hit"][...,0])&(placement!=0)&(np.abs(depth-d["depth"])<=.02*d["depth"])
                    accepted+=use;total+=np.where(use,visibility,0)
                valid=accepted>0;v=np.divide(total,accepted,out=np.ones(shape),where=valid)
            else:
                v=read_hdr(path/f"{index:04}-visibility.rgba16f",shape)[...,0];valid=np.ones(shape,bool)
            modes["RL "+mode]=(v,valid,hdr)
            full=path/f"{index:04}-full.rgba16f"
            if full.exists():
                im=read_hdr(full,(scene["size"][1],scene["size"][0]));s1.film.tone(im).save(path/f"{index:04}-full.png");s1.film.tone(im,4).save(path/f"{index:04}-full-plus4.png");full.unlink()
        if not renderlab_only:
            for mode in ("VSM","HRT1","HRT256"):
                path=root/"unreal"/case/mode/f"{index:04}"
                if (path/"decoded.npz").exists():
                    with np.load(path/"decoded.npz") as a:modes["UE "+mode]=(a["visibility"],a["valid"],a["hdr"] if "hdr" in a.files else None)
        common=d["cover"].copy()
        for v,valid,hdr in modes.values():common &= valid
        for label,(v,valid,hdr) in modes.items():
            row=frame_score(v,d,pops,valid,hdr)
            row.update(case=case,mode=label,frame=index,phase=pose["phase"],seconds=pose["seconds"],partial_reference_n=int(pops["partial"].sum()),common_partial_rms=s1.metrics.summary((v-refs)[pops["partial"]&common]).get("rms"))
            if label=="RL Temporal":row.update(reset=json.loads((directory/"Temporal"/f"{index:04}.json").read_text())["reset"],visibility_condition="S1 placement and 2%-depth acceptance")
            if label in previous:
                oldv,oldref,old_hdr_ref=previous[label]
                row["residual_change_rms"]=float(np.sqrt(np.mean(((v-refs)-(oldv-oldref))[valid&d["cover"]]**2))) if (valid&d["cover"]).any() else None
                changed=np.abs(refs-oldref)>.05
                row["changed_pixels"]=int(changed.sum())
                row["ghost_rms"]=s1.metrics.summary((v-refs)[changed&valid&d["cover"]]).get("rms")
                row["previous_reference_rms"]=s1.metrics.summary((v-oldref)[changed&valid&d["cover"]]).get("rms")
                if pose["phase"]=="jump-recovery" and pose.get("phase_age")==0:
                    jump[label]=(oldref.copy(),changed.copy(),old_hdr_ref)
            if label in jump and pose["phase"]=="jump-recovery":
                before,changed,before_hdr=jump[label];use=changed&valid&d["cover"]
                row["jump_age"]=pose["phase_age"]
                row["jump_ghost_rms"]=s1.metrics.summary((v-refs)[use]).get("rms")
                row["old_sun_projection"]=old_sun_projection(v,refs,before,use)
                if hdr is not None:
                    current_hdr=d["hdr"][...,:3];rgb=hdr[...,:3]
                    row["jump_hdr_ghost_rms"]=float(np.sqrt(np.mean((rgb-current_hdr)[use]**2))) if use.any() else None
                    row["hdr_old_sun_projection"]=old_sun_projection(rgb,current_hdr,before_hdr,use)
            previous[label]=(v.copy(),refs.copy(),d["hdr"][...,:3]);result.append(row)
            if index in (0,15,16,17,19,23,31,32,63,64,95,96,127,159,191,319,351,352,353,355,359,367,384):
                panels.extend([(label,s1.film.tone(hdr) if hdr is not None else s1.film.vis(v,d["cover"])),(label+" error x8",s1.film.error(v-refs,d["cover"],valid))])
        if len(panels)>2:
            s1.film.sheet(panels,columns=4,width=512).save(images/f"{case}-f{index:04}.png")
            if index in ((16,95) if scene["bench"]=="S2" else (31,352,353,359,384)):
                rect=(128,32,384,224) if scene["bench"]=="S2" else (96,40,288,184)
                details=[(label,im.crop(rect).resize((512,384),Image.Resampling.NEAREST)) for label,im in panels]
                s1.film.sheet(details,columns=4,width=512).save(images/f"{case}-f{index:04}-details.png")
        if index%32==0:print(case,"score",index,flush=True)
    save(directory/("scores-renderlab.json" if renderlab_only else "scores.json"),result)
    save(directory/("scoring-renderlab.json" if renderlab_only else "scoring-paired.json"),dict(seconds=time.monotonic()-started,frames=len(track["frames"]),rows=len(result),scope="CPU decode/score/PNG/NPZ compaction; not frame time"))
    # Preserve small source samples for the subsequent paired score. The large
    # reference dumps and full-frame HDR readbacks are now compacted to NPZ/PNG.
    return result


def native_profile(path,scene,track):
    with path.open(encoding="utf-8-sig",newline="") as f:raw=list(csv.reader(f))
    header=next(row for row in reversed(raw) if row and row[0]=="EVENTS")
    rows=[]
    for row in raw:
        if not row or row[0]=="EVENTS" or row[0].startswith("["):continue
        row=row+["0"]*max(0,len(header)-len(row))
        try:rows.append([float(v) if i else v for i,v in enumerate(row[:len(header)])])
        except ValueError:continue
    frames=track["frames"];count=len(frames)*8
    if len(rows)<count:raise ValueError(f"native profile truncated: {len(rows)} < {count}")
    # CSV contains the renderer's own view position. Validate the transport and
    # frame association independently of intended Python tick ordinals.
    alignment=[]
    for offset in range(min(5,len(rows)-count+1)):
        errors=[]
        for i,row in enumerate(rows[offset:offset+count]):
            eye=np.array([row[header.index("View/PosY")],row[header.index("View/PosZ")],-row[header.index("View/PosX")]])/100
            errors.append(float(np.max(np.abs(eye-np.array(frames[i%len(frames)]["eye"])))))
        alignment.append((max(errors),abs(offset-(1 if len(rows)>count else 0)),offset))
    error,_,offset=min(alignment)
    # Some UE CSV view positions use centimetres while exposing float rounding.
    if error>2e-5:raise ValueError(f"native CSV camera/timestamp mismatch: {error} metres")
    fields=[name for name in header if name in ("GPUTime","FrameTime","RenderThreadTime","GameThreadTime","RHIThreadTime","RenderThreadTime_CriticalPath","GameThreadTime_CriticalPath") or name.startswith(("GPU/","VSM/")) or "Page" in name or "Invalidat" in name]
    result=[]
    for i,row in enumerate(rows[offset:offset+count]):
        result.append(dict(frame=i%len(frames),repeat=i//len(frames),**{field:row[header.index(field)] for field in fields}))
    save(path.with_name("profile-frames.json"),dict(rows=result,csv_rows=len(rows),alignment_offset=offset,maximum_camera_error_m=error,
        phase_alignment_limit="Static-camera S3 CSV phase boundaries have up to one engine frame of scheduling uncertainty; exclude boundary frames for steady phase costs.",
        invalidation_counters=[name for name in fields if "Page" in name or "Invalidat" in name]))
    return result


def comparison(root):
    scene=json.loads((root/"fixture/scene.json").read_text());timings=json.loads((root/"renderlab/timings.json").read_text())
    scores=[];table=[];invalidation=[]
    for directory in (root/"renderlab").iterdir():
        if not directory.is_dir():continue
        path=directory/"scores.json"
        if not path.exists():path=directory/"scores-renderlab.json"
        scores.extend(json.loads(path.read_text()))
        track=next(t for t in scene["tracks"] if directory.name.startswith(t["name"]+"-r"))
        for mode in ("VSM","HRT1","HRT256"):
            path=root/"unreal"/directory.name/mode/"profile.csv"
            if path.exists():
                rows=native_profile(path,scene,track)
                timings.extend(dict(case=directory.name,mode="UE "+mode,frame=row["frame"],repeat=row["repeat"],gpu_ms=row["GPUTime"],
                    shadow_depths_ms=row.get("GPU/ShadowDepths"),projection_ms=row.get("GPU/ShadowProjection"),cpu_service_ms=row["GameThreadTime"],frame_ms=row["FrameTime"],
                    **{key:value for key,value in row.items() if "VSM" in key or "Page" in key or "Invalidat" in key}) for row in rows)
                if mode=="VSM":
                    for phase in dict.fromkeys(p["phase"] for p in track["frames"]):
                        phase_frames=[p["frame"] for p in track["frames"] if p["phase"]==phase]
                        # Native fixed-camera phase transport has one-frame
                        # uncertainty. Attribute steady rotation costs away
                        # from both boundaries; retain all frames separately.
                        selected=[r for r in rows if r["frame"] in phase_frames[2:-2]]
                        fields=[key for key in rows[0] if "VSM" in key or "Page" in key or "Invalidat" in key or key in ("GPUTime","GPU/ShadowDepths","GPU/ShadowProjection")]
                        invalidation.append(dict(case=directory.name,phase=phase,scope="Steady phase, first/last two frames excluded",samples=len(selected),
                            fields={key:dict(mean=float(np.mean([r[key] for r in selected])),p50=float(np.median([r[key] for r in selected])),p95=float(np.quantile([r[key] for r in selected],.95))) for key in fields} if selected else {}))
    for row in timings:
        if not row["mode"].startswith("UE "):row["mode"]="RL "+row["mode"]
    for case in sorted({row["case"] for row in scores}):
        track=next(t for t in scene["tracks"] if case.startswith(t["name"]+"-r"))
        phases=list(dict.fromkeys(p["phase"] for p in track["frames"]))
        for phase in phases:
            frames={p["frame"] for p in track["frames"] if p["phase"]==phase}
            for mode in sorted({row["mode"] for row in scores if row["case"]==case}):
                selected=[r for r in scores if r["case"]==case and r["mode"]==mode and r["frame"] in frames]
                timing=[r for r in timings if r["case"]==case and r["mode"]==mode and r["frame"] in frames]
                parts=[r["partial"] for r in selected if r["partial"].get("n",0)]
                row=dict(case=case,phase=phase,mode=mode,frames=len(selected),
                    partial_rms=float(np.sqrt(sum(p["rms"]**2*p["n"] for p in parts)/sum(p["n"] for p in parts))) if parts else None,
                    contact_lost=sum(r["contact_lost"] for r in selected),contact_n=sum(r["contact_n"] for r in selected),
                    leaks_max=max(r["leaks"] for r in selected),solid_flips_max=max(r["solid_flips"] for r in selected),
                    solid_error_gt_002_max=max(r["solid_error_gt_002"] for r in selected),missing_max=max(r["missing_covered"] for r in selected))
                hdr_errors=[r["radiance"]["linear_hdr"]["rms"] for r in selected if "radiance" in r]
                row["hdr_rms"]=float(np.sqrt(np.mean(np.square(hdr_errors)))) if hdr_errors else None
                for key in ("gpu_ms","shadow_depths_ms","projection_ms","cpu_service_ms","frame_ms"):
                    values=[r[key] for r in timing if r.get(key) is not None]
                    if values:
                        row.update({key+"_"+suffix:float(value) for suffix,value in zip(("p50","p95","p99"),np.quantile(values,[.5,.95,.99]))})
                table.append(row)
    lag=[]
    for mode in sorted({row["mode"] for row in scores}):
        recovery=sorted((r for r in scores if r["mode"]==mode and "jump_age" in r),key=lambda r:r["jump_age"])
        if recovery:
            first=next((r["jump_age"] for i,r in enumerate(recovery) if all(x.get("old_sun_projection") is not None and abs(x["old_sun_projection"])<=.1 for x in recovery[i:])),None)
            hdr_first=next((r["jump_age"] for i,r in enumerate(recovery) if all(x.get("hdr_old_sun_projection") is not None and abs(x["hdr_old_sun_projection"])<=.1 for x in recovery[i:])),None)
            lag.append(dict(mode=mode,lag90_diagnostic_frames=first,hdr_lag90_diagnostic_frames=hdr_first,ages=[{key:r.get(key) for key in ("jump_age","jump_ghost_rms","old_sun_projection","jump_hdr_ghost_rms","hdr_old_sun_projection","leaks","solid_flips")} for r in recovery]))
    save(root/"comparison.json",table);save(root/"per-frame.json",scores);save(root/"timing-frames.json",timings);save(root/"jump-recovery.json",lag)
    save(root/"vsm-invalidation.json",invalidation)
    fields=list(dict.fromkeys(key for row in table for key in row))
    with (root/"comparison.csv").open("w",newline="") as f:
        writer=csv.DictWriter(f,fieldnames=fields);writer.writeheader();writer.writerows(table)
    return table


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--root",type=Path,required=True)
    parser.add_argument("--case");parser.add_argument("--renderlab-only",action="store_true");parser.add_argument("--comparison-only",action="store_true");args=parser.parse_args()
    if args.comparison_only:comparison(args.root);return
    cases=[args.case] if args.case else [p.name for p in (args.root/"renderlab").iterdir() if p.is_dir()]
    for case in cases:score_case(args.root,case,args.renderlab_only)


if __name__=="__main__":main()
