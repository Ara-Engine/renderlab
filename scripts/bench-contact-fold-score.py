"""S1 film/native-mask scoring; immutable film populations, explicit missing data.

Decode raw resources before optional cleanup. No alignment, bias fitting or replacement
of missing receivers. All arrays/PNGs stay in the caller's private evidence directory.
"""
import argparse
import csv
import hashlib
import importlib.util
import json
import math
import re
from pathlib import Path
import sys

import numpy as np
from PIL import Image,ImageDraw

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/"support"))
import fresh_level_b_metrics as metrics


def load(name):
    spec=importlib.util.spec_from_file_location(name.replace("-","_"),Path(__file__).with_name(name+".py"))
    m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m


film=load("m6-film-score")
native=load("ue-shadow-dump")
profiler=load("ue-quarry-profile")


def save(path,value):path.write_text(json.dumps(value,indent=2,allow_nan=False)+"\n")


def unpack_r11g11b10(packed):
    channels=[]
    for shift,mantissa in ((0,6),(11,6),(22,5)):
        bits=(packed>>shift)&((1<<(mantissa+5))-1)
        exponent=bits>>mantissa;fraction=bits&((1<<mantissa)-1)
        value=np.where(exponent==0,np.ldexp(fraction.astype(float),-14-mantissa),
                       np.ldexp(1+fraction/(1<<mantissa),exponent.astype(int)-15))
        value=np.where(exponent==31,np.inf,value)
        channels.append(value)
    return np.stack(channels,-1)


def reference(directory,pose,scene):
    if (directory/"reference.npz").exists():
        with np.load(directory/"reference.npz") as cached:d={k:cached[k] for k in cached.files}
        d["info"]=json.loads((directory/f"pose-{pose}.json").read_text())
        return d
    d=film.decode(directory,pose)
    d["depth"]=d["surface_depth"]
    n=np.fromfile(directory/f"pose-{pose}.normal.f32",dtype="<f4").reshape(*d["ref"].shape,4)[...,:3].astype(float)
    n*=np.where(np.einsum("ijk,ijk->ij",n,np.array(d["info"]["eye"][:3])-d["world"][...,:3])<0,-1,1)[...,None]
    d["front"]=d["cover"] & (n@np.array(scene["to_light"])>0)
    return d


def decode_native(directory,scene):
    cached=directory/"decoded.npz"
    if cached.exists():
        with np.load(cached) as a:return {k:a[k] for k in a.files}
    dump=native.Dump(directory/"native" if (directory/"native").exists() else directory)
    with (dump.root/"Base/ConsoleVariables.csv").open(encoding="utf-8-sig",newline="") as f:
        controls={r["CVar"]:r for r in csv.DictReader(f)}
    mode=json.loads((directory/"recipe.json").read_text())["mode"]
    expected={"r.AntiAliasingMethod":"0","r.ScreenPercentage":"100","r.EyeAdaptation.PreExposureOverride":"1",
              "r.RayTracing.Shadows":"0" if mode=="VSM" else "1",
              "r.RayTracing.Shadows.SamplesPerPixel":"256" if mode=="HRT256" else "1"}
    for key,value in expected.items():
        if key not in controls or float(controls[key]["Value"])!=float(value):raise ValueError("effective control differs: "+key)
    light,=[p for p in dump.passes if p["EventName"].startswith("Light::StandardDeferred: S1_Sun")]
    inputs=dump.inputs(light)
    depth,depth_id=dump.resource(inputs["SceneDepthZ"],light)
    names=[k for k in inputs if k in ("ShadowMaskTexture","RayTracingOcclusion") or k.startswith("Shadow.Denoiser.DenoiserOutput")]
    if len(names)!=1:raise ValueError("ambiguous native mask: "+str(names))
    attenuation,mask_id=dump.resource(inputs[names[0]],light)
    visibility=attenuation[...,0]**2*attenuation[...,2]**2
    recipe=json.loads((directory/"recipe.json").read_text())
    rays=[p for p in dump.passes if p["EventName"].startswith("RayTracedShadow")]
    ray_values=dump.scalar_parameters(rays[0]) if rays else {}
    if recipe["mode"]!="VSM":
        if abs(ray_values["Light.SourceRadius"]-math.sin(scene["sun_radius"]))>1e-8:raise ValueError("native source proxy angle mismatch")
        if ray_values["SamplesPerPixel"] != (256 if recipe["mode"]=="HRT256" else 1):raise ValueError("native SPP mismatch")
    # Pre-tonemap SceneColor holds physical radiance because PreExposureOverride=1.
    hdr_dump=native.Dump(directory/"hdr") if (directory/"hdr").exists() else dump
    tone=[p for p in hdr_dump.passes if "Tonemap" in p["EventName"]]
    hdr=None;hdr_id=None
    for event in tone:
        for key,name in hdr_dump.inputs(event).items():
            desc=hdr_dump.resources[name.split(".")[0]]
            if "SceneColor" in key and desc.get("ExtentX")==scene["size"][0] and desc.get("ExtentY")==scene["size"][1]:
                if desc["Format"]=="PF_FloatR11G11B10":
                    writer="00000000000000000000"
                    for earlier in hdr_dump.passes:
                        if earlier["Id"]>=event["Id"]:break
                        if name in earlier["OutputResources"]:writer=earlier["Pointer"]
                    path=hdr_dump.root/"Resources"/f"{name}.v{writer}.bin"
                    packed=np.fromfile(path,dtype="<u4").reshape(desc["ExtentY"],desc["ExtentX"])
                    hdr=unpack_r11g11b10(packed)
                    hdr_id=dict(resource=name,version=writer,path=str(path),sha256=native.digest(path),description=desc)
                else:hdr,hdr_id=hdr_dump.resource(name,event)
                break
        if hdr is not None:break
    if hdr is None:raise ValueError("no full-resolution pre-tonemap SceneColor; inspect dump")
    valid=(depth>0)&np.isfinite(visibility)
    data=dict(visibility=visibility,valid=valid,depth=depth,hdr=hdr*(2**scene["exposure_stops"]))
    np.savez_compressed(cached,**data)
    save(directory/"effective-controls.json",dict(mask=mask_id,depth=depth_id,radiance=hdr_id,ray_parameters=ray_values,
                      recipe=recipe,pre_exposure_override=1,exposure_scale=2**scene["exposure_stops"]))
    save(directory/"confirmed-cvars.json",{key:controls_row for key,controls_row in controls.items()
         if key in expected or key.startswith(("r.Shadow.Virtual.","r.Shadow.Denoiser","r.RayTracing.Shadows."))})
    return data


def verify_camera(directory,scene,log):
    block=log.split("S1_BEGIN ")
    block=next(b for b in block if b.splitlines()[0].replace("\\","/").endswith("/"+directory.name))
    line=next(line for line in block.splitlines() if "CameraCachePrivate =" in line)
    values=re.search(r"Location=\(X=([^,]+),Y=([^,]+),Z=([^\)]+)\),Rotation=\(Pitch=([^,]+),Yaw=([^,]+),Roll=([^\)]+)\),FOV=([^,]+)",line)
    if values is None:raise ValueError("runtime camera readback missing")
    x,y,z,pitch,yaw,roll,fov=map(float,values.groups())
    pose=json.loads((directory/"recipe.json").read_text())["pose"];p=scene["poses"][pose]
    eye=np.array([y,z,-x])/100
    forward=np.array(p["target"])-p["eye"];forward/=np.linalg.norm(forward)
    a,b=np.radians([pitch,yaw]);ue=np.array([np.cos(a)*np.cos(b),np.cos(a)*np.sin(b),np.sin(a)])
    actual=ue[[1,2,0]]*[1,1,-1]
    expected=math.degrees(2*math.atan(math.tan(math.pi/8)*1920/1080))
    if np.max(np.abs(eye-p["eye"]))>1e-6 or np.max(np.abs(actual-forward))>2e-6 or abs(fov-expected)>1e-5 or abs(roll)>1e-6:
        raise ValueError("runtime camera differs from registration")
    result=dict(eye=eye.tolist(),forward=actual.tolist(),horizontal_fov=fov,readback=line)
    save(directory/"confirmed-camera.json",result)
    return result


def scored(candidate,d,pops,valid):
    errors=candidate-d["ref"];rows={}
    for name,pop in pops.items():
        selected=pop&valid
        rows[name]=metrics.summary(errors[selected])|dict(reference_n=int(pop.sum()),missing_n=int((pop&~valid).sum()))
    contact=pops["contact"]&valid;umbra=pops["occluded"]&valid;solid=pops["solid"]&valid
    lost=int((contact&(errors>.5)).sum())
    flips=(pops["lit"]&valid&(errors<-.5)) | (umbra&(errors>.5))
    rows.update(contact_lost=lost,contact_n=int(contact.sum()),contact_loss_rate=lost/int(contact.sum()) if contact.any() else None,
                leaks=int((umbra&(errors>.1)).sum()),
                solid_flips=int(flips.sum()),
                solid_error_gt_002=int((solid&(np.abs(errors)>.002)).sum()),
                solid_max=float(np.max(np.abs(errors[solid]))) if solid.any() else None,
                missing_covered=int((d["cover"]&~valid).sum()),
                missed_shadows=int((d["cover"]&valid&(d["ref"]<=.9)&(candidate>=.99)).sum()),
                false_shadows=int((d["cover"]&valid&(d["ref"]>=.99)&(candidate<=.9)).sum()))
    one_surface=solid&(d["n"]==256)
    rows["all_256_primaries_accepted_solid"]=metrics.summary(errors[one_surface])|dict(error_gt_002=int((one_surface&(np.abs(errors)>.002)).sum()))
    contour,centres=metrics.contour_segments(errors,d,pops["partial"])
    from scipy.ndimage import maximum_filter
    good=~maximum_filter((d["cover"]&~valid).astype("u1"),size=19).astype(bool)
    rows["contour"]=metrics.summary(contour[good[centres]])|dict(missing_n=int((~good[centres]).sum()))
    return rows


def radiance(candidate,reference):
    rows={"linear_hdr":metrics.summary((candidate[...,:3]-reference[...,:3]).ravel())}
    for stops in (0,4):
        a=np.asarray(film.tone(candidate,stops),dtype=float);b=np.asarray(film.tone(reference,stops),dtype=float)
        rows[f"display_{stops:+}_codes"]=metrics.summary((a-b).ravel())
    return rows


def sheet(panels,path,width=480):
    film.sheet(panels,columns=4,width=width).save(path)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--root",type=Path,required=True)
    parser.add_argument("--unreal",default="unreal-final")
    parser.add_argument("--unreal-log",type=Path)
    args=parser.parse_args();root=args.root
    scene=json.loads((root/"fixture/scene.json").read_text());images=root/"images";images.mkdir(exist_ok=True)
    timings=json.loads((root/"renderlab/timings.json").read_text())
    result={};table=[]
    for pose in (0,1):
        for environment in (0,1):
            folder=root/f"renderlab/p{pose}-{'black' if environment==0 else 'environment'}"
            d=reference(folder,pose,scene);pops=metrics.populations(d)
            np.savez_compressed(folder/"reference.npz",ref=d["ref"],sigma=d["sigma"],cover=d["cover"],n=d["n"],depth=d["depth"],hdr=d["hdr"],hit=d["hit"])
            candidates={};panels=[("Film visibility",film.vis(d["ref"],d["cover"]))]
            radiance_panels=[("Film",film.tone(d["hdr"])),("Film +4",film.tone(d["hdr"],4))]
            masks={}
            for mode in ("hard","Exact","Interactive","Temporal"):
                hdr=np.fromfile(folder/f"{mode}-n{128 if mode=='Temporal' else 32}.rgba16f",dtype="<f2").reshape(*d["ref"].shape,4).astype(float)
                if mode=="Temporal":
                    counts=sum(np.fromfile(folder/f"{mode}-visibility-{half}.u32",dtype="<u4").reshape(*d["ref"].shape,2).astype(float) for half in (0,1))
                    valid=counts[...,1]>0
                    v=np.divide(counts[...,0],256*counts[...,1],out=np.ones(d["ref"].shape),where=valid)
                else:
                    v=np.fromfile(folder/f"{mode}-visibility.rgba16f",dtype="<f2").reshape(*d["ref"].shape,4)[...,0].astype(float)
                    valid=np.ones(d["ref"].shape,bool)
                candidates["RL "+mode]=dict(visibility=v,valid=valid,hdr=hdr)
            for mode in ("VSM","HRT1","HRT256"):
                directory=root/args.unreal/f"p{pose}-e{environment}-{mode}"
                if not directory.exists():continue
                recipe=json.loads((directory/"recipe.json").read_text())
                log=args.unreal_log or root/"runs"/(recipe.get("session","unreal-batch-3")+".log")
                verify_camera(directory,scene,log.read_text(encoding="utf-8-sig"))
                candidates["UE "+mode]=decode_native(directory,scene)
            # Preserve a common-domain comparison in addition to complete-domain missing counts.
            common=d["cover"].copy()
            for value in candidates.values():common &= value["valid"]
            case={"reference":dict(populations={k:int(v.sum()) for k,v in pops.items()},
                                   partial_sigma_rms=float(np.sqrt(np.mean(d["sigma"][pops["partial"]]**2))),
                                   missing_samples=int((d["cover"]&(d["n"]==0)).sum())),"modes":{}}
            for label,c in candidates.items():
                row=scored(c["visibility"],d,pops,c["valid"])
                row["common_domain"]=scored(c["visibility"],d,pops,c["valid"]&common)
                row["radiance"]=radiance(c["hdr"],d["hdr"])
                if label.startswith("RL"):
                    timing=next(t for t in timings if t["pose"]==pose and t["environment"]==environment and t["mode"]==label[3:])
                    row["timing"]=dict(gpu_ms=metrics.summary(np.array(timing["gpu_ms"])),gpu_median_ms=float(np.median(timing["gpu_ms"])),
                                       cpu_service_median_ms=float(np.median(timing["cpu_service_ms"])),scope=timing["scope"])
                else:
                    directory=root/args.unreal/f"p{pose}-e{environment}-{label[3:]}"
                    files=list(directory.glob("*.csv"))
                    if files:
                        summary=profiler.summarize(files[0]);save(directory/"profile-summary.json",summary)
                        row["timing"]=summary
                case["modes"][label]=row
                im=film.vis(c["visibility"],d["cover"]);im.save(images/f"p{pose}-e{environment}-{label.replace(' ','-')}.png")
                panels.extend([(label,im),(label+" error x8",film.error(c["visibility"]-d["ref"],d["cover"],c["valid"]))])
                masks[label]=im
                radiance_panels.extend([(label,film.tone(c["hdr"])),(label+" +4",film.tone(c["hdr"],4))])
                timing=row.get("timing",{})
                gpu=timing.get("statistics",{}).get("GPUTime",{})
                table.append(dict(pose=pose,environment=environment,mode=label,contact_lost=row["contact_lost"],contact_n=row["contact_n"],
                                  leaks=row["leaks"],partial_rms=row["partial"].get("rms"),solid_flips=row["solid_flips"],missing=row["missing_covered"],
                                  gpu_median_ms=timing.get("gpu_median_ms",gpu.get("median"))))
            result[f"p{pose}-e{environment}"]=case
            sheet(panels,images/f"p{pose}-e{environment}-visibility.png")
            sheet(radiance_panels,images/f"p{pose}-e{environment}-radiance.png")
            # Lane crops are registered by projecting the generated lane coordinates.
            eye=np.array(scene["poses"][pose]["eye"]);forward=np.array(scene["poses"][pose]["target"])-eye;forward/=np.linalg.norm(forward)
            right=np.cross(forward,[0,1,0]);right/=np.linalg.norm(right);up=np.cross(right,forward)
            f=scene["size"][1]/(2*math.tan(math.pi/8))
            points=[("strips",[-.15,0,-.55]),("stack",[-.5,0,-.15]),("concave",[.57,.07,.1]),("convex",[.57,.1,.6]),("shell",[-.45,.06,.65])]
            regions=[]
            for name,p in points:
                delta=np.array(p)-eye;z=delta@forward
                x=round(960+f*(delta@right)/z);y=round(540-f*(delta@up)/z)
                regions.append((name,x,y))
            crop=Image.new("RGB",(len(regions)*210,(len(masks)+1)*230),(20,27,35));draw=ImageDraw.Draw(crop)
            for row,(label,im) in enumerate([("Film",film.vis(d["ref"],d["cover"]))]+list(masks.items())):
                for col,(name,x,y) in enumerate(regions):
                    draw.text((col*210+4,row*230+4),f"{label} / {name} ({x},{y})",fill="white")
                    crop.paste(im.crop((x-50,y-50,x+51,y+51)).resize((202,202),Image.Resampling.NEAREST),(col*210,row*230+25))
            crop.save(images/f"p{pose}-e{environment}-crops.png")
    save(root/"scores.json",result)
    with (root/"comparison.csv").open("w",newline="") as f:
        w=csv.DictWriter(f,fieldnames=list(table[0]));w.writeheader();w.writerows(table)
    print(json.dumps(table,indent=2))


if __name__ == "__main__":main()
