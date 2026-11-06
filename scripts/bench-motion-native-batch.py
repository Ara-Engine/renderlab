"""Compact a completed multi-frame RDG dump, preserving frame and writer identity."""
import csv
import importlib.util
import json
from pathlib import Path
import re

import numpy as np

spec=importlib.util.spec_from_file_location("motion_score",Path(__file__).with_name("bench-motion-score.py"))
score=importlib.util.module_from_spec(spec);spec.loader.exec_module(score)
WITNESSES=(0,15,16,17,19,23,31,32,63,64,95,96,127,159,191,319,351,352,353,355,359,367,384)


def consume(stage,scene):
    recipe=json.loads((stage/"recipe.json").read_text())
    root=next((stage/"dump").glob("M5Calibration-*"))
    if (root/"Base/Status.txt").read_text().strip()!="ok":raise FileNotFoundError("dump writing")
    dump=score.s1.native.Dump(root)
    groups={}
    for event in dump.passes:
        frame=next((int(m.group(1)) for scope in event["ParentEventScopes"] if (m:=re.search(r"^Frame (\d+)",scope))),None)
        if frame is None:raise ValueError("pass has no frame scope")
        groups.setdefault(frame,[]).append(event)
    if len(groups)!=recipe["frames"]:raise ValueError(f"frame coverage {len(groups)} != {recipe['frames']}")
    frame_ids=sorted(groups)
    if frame_ids!=list(range(frame_ids[0],frame_ids[0]+len(frame_ids))):raise ValueError("native dump has discontinuous engine frames")
    for event in dump.passes:
        scope=next(s for s in event["ParentEventScopes"] if s.startswith("Frame "))
        delta=re.search(r"Delta=([0-9.]+)s",scope)
        if delta is None or abs(float(delta.group(1))-1/60)>1e-6:raise ValueError("native RDG simulation delta differs: "+scope)
    case=stage.parent.parent if recipe["stage"]!="depth" else stage.parent
    controls={r["CVar"]:r for r in csv.DictReader((dump.root/"Base/ConsoleVariables.csv").open(encoding="utf-8-sig",newline=""))}
    expected={"r.AntiAliasingMethod":4,"r.ScreenPercentage":100,"r.EyeAdaptation.PreExposureOverride":1}
    expected|={"r.Shadow.Virtual.Enable":1,"r.Shadow.Virtual.Cache":1,"r.ShadowQuality":5,
               "r.Shadow.Virtual.SMRT.RayCountDirectional":8,"r.Shadow.Virtual.SMRT.SamplesPerRayDirectional":8,
               "r.Shadow.Virtual.SMRT.AdaptiveRayCount":1,"r.TSR.History.ScreenPercentage":200}
    if recipe["stage"]!="depth":expected|={"r.RayTracing.Shadows":int(recipe["mode"]!="VSM"),"r.RayTracing.Shadows.SamplesPerPixel":256 if recipe["mode"]=="HRT256" else 1}
    if recipe["stage"]!="depth":expected|={"r.Shadow.Denoiser":int(recipe["mode"]!="HRT256"),"r.Shadow.Denoiser.TemporalAccumulation":1}
    for key,value in expected.items():
        if float(controls[key]["Value"])!=value:raise ValueError("effective control mismatch "+key)
    identities=[]
    for local_index,(_,events) in enumerate(sorted(groups.items())):
        index=local_index+recipe.get("frame_offset",0)
        target=stage.parent/f"{index:04}"
        if recipe["stage"]=="depth":target=case/"depth"/f"{index:04}"
        target.mkdir(parents=True,exist_ok=True)
        frame_recipe=json.loads((target/"recipe.json").read_text()) if (target/"recipe.json").exists() else None
        if recipe["stage"]=="parameters":
            rays,=[p for p in events if p["EventName"].startswith("RayTracedShadow")]
            params=dump.scalar_parameters(rays)
            if abs(params["Light.SourceRadius"]-np.sin(frame_recipe["radius"]))>1e-7 or params["SamplesPerPixel"]!=1:raise ValueError("HRT1 shader parameters differ")
            direction=np.array(frame_recipe["pose"]["to_light"])[[2,0,1]]*[-1,1,1]
            if np.max(np.abs(np.array(params["Light.Direction"])-direction))>2e-6:raise ValueError("HRT1 light timestamp differs")
            light,=[p for p in events if p["EventName"].startswith("Light::StandardDeferred: ") and "Sun" in p["EventName"]]
            masks=[key for key in dump.inputs(light) if key=="RayTracingOcclusion" or key.startswith("Shadow.Denoiser.DenoiserOutput")]
            if len(masks)!=1:raise ValueError("ambiguous HRT1 consumed mask "+str(masks))
            identity=dict(ray_parameters=params,consumed_mask=masks[0])
            score.save(stage/"effective-parameters.json",identity)
        elif recipe["stage"]=="depth":
            candidates=[(p,n) for p in events for n in p["OutputResources"] if dump.resources[n.split(".")[0]]["Name"]=="SceneDepthZ" and n.endswith(".mip0")]
            if not candidates:raise ValueError("prepass has no main depth output")
            producer,name=candidates[-1];depth,identity=score.resource(dump,name,dict(Id=producer["Id"]+1))
            np.savez_compressed(target/"depth.npz",depth=score.cut(depth,scene))
            score.save(target/"identity.json",identity)
        elif recipe["stage"]=="native":
            mode=recipe["mode"]
            names={"VSM":"ShadowMaskTexture","HRT1":"Shadow.Denoiser.DenoiserOutput0","HRT256":"RayTracingOcclusion"}
            if mode=="HRT1":names[mode]=json.loads((stage.parent/"stage-parameters/effective-parameters.json").read_text())["consumed_mask"]
            candidates=[(p,n) for p in events for n in p["OutputResources"] if dump.resources[n.split(".")[0]]["Name"]==names[mode]]
            if len(candidates)!=1:raise ValueError("ambiguous native producer "+str([(p["EventName"],n) for p,n in candidates]))
            producer,name=candidates[0];mask,identity=score.resource(dump,name,dict(Id=producer["Id"]+1))
            visibility=mask[...,0]**2*mask[...,2]**2
            with np.load(case/"depth"/f"{index:04}"/"depth.npz") as a:depth=a["depth"]
            params={};depth_check=None
            if names[mode]=="RayTracingOcclusion":
                rays,=[p for p in events if p["EventName"].startswith("RayTracedShadow")]
                params=dump.scalar_parameters(rays)
                if abs(params["Light.SourceRadius"]-np.sin(frame_recipe["radius"]))>1e-7 or params["SamplesPerPixel"]!=(256 if mode=="HRT256" else 1):raise ValueError("ray parameters differ")
                direction=np.array(frame_recipe["pose"]["to_light"])[[2,0,1]]*[-1,1,1]
                if np.max(np.abs(np.array(params["Light.Direction"])-direction))>2e-6:raise ValueError("ray timestamp differs")
                name,=[n for n in rays["InputResources"] if dump.resources[n.split(".")[0]]["Name"]=="SceneDepthZ" and n.endswith(".mip0")]
                actual,depth_id=score.resource(dump,name,rays,scene["crop"])
                depth_check=dict(equal=bool(np.array_equal(actual,depth)),maximum=float(np.max(np.abs(actual-depth))),
                                 coverage_equal=bool(np.array_equal(actual>0,depth>0)),all_covered=bool((actual>0).all() and (depth>0).all()),identity=depth_id)
                # Only availability uses the shared prepass. Its covered versus
                # background mask must match the actual ray input every frame;
                # prepass depth values need not equal the final base-pass depth.
                if not depth_check["coverage_equal"]:raise ValueError("shared native coverage is not certified")
                depth=actual
            valid=(depth>0)&np.isfinite(score.cut(visibility,scene))
            np.savez_compressed(target/"decoded.npz",visibility=score.cut(visibility,scene).astype("f4"),valid=valid,depth=depth)
            score.save(target/"effective.json",dict(mask=identity,ray_parameters=params,depth_check=depth_check,controls={k:controls[k] for k in expected},depth="Actual ray input when available; separate prepass used only for coverage, checked against actual ray input every frame. Values can differ from final base-pass depth."))
            if index in WITNESSES:score.s1.film.vis(score.cut(visibility,scene),valid).save(target/"native-visibility.png")
        else:
            candidates=[]
            for event in events:
                for name in event["InputResources"]:
                    desc=dump.resources[name.split(".")[0]]
                    if ("SceneColor" in desc["Name"] or desc["Name"]=="TSR.Output") and desc.get("ExtentX")==1920 and desc.get("ExtentY")==1080:candidates.append((event,name))
            if not candidates:raise ValueError("no native-size HDR")
            event,name=candidates[0];witness=index in WITNESSES
            hdr,identity=score.resource(dump,name,event,None if witness else scene["crop"])
            with np.load(target/"decoded.npz") as a:data={k:a[k] for k in a.files}
            data["hdr"]=(score.cut(hdr,scene) if witness else hdr).astype("f4")*2**scene["exposure_stops"]
            np.savez_compressed(target/"decoded.npz",**data)
            score.save(target/"effective-hdr.json",dict(radiance=identity,scope="Contiguous reset-view-state identical-track post-TSR HDR replay"))
            if witness:
                score.s1.film.tone(hdr*2**scene["exposure_stops"]).save(target/"final-radiance.png")
                score.s1.film.tone(hdr*2**scene["exposure_stops"],4).save(target/"final-radiance-plus4.png")
        identities.append(dict(frame=index,resource=identity))
    if recipe["stage"] in ("native","hdr"):
        log=(case.parent.parent/"unreal-render.log").read_text(encoding="utf-8-sig",errors="replace")
        token="MOTION_STAGE "+stage.name.removeprefix("stage-")+" "+str(stage.parent)
        if token not in log:raise ValueError("capture-stage camera marker missing")
        block=log.rsplit(token,1)[1];lines=[]
        for view in block.split("MOTION_VIEW ")[1:]:
            directory=Path(view.splitlines()[0].strip())
            if directory.parent!=stage.parent:continue
            camera=next((line for line in view.splitlines() if "CameraCachePrivate =" in line),None)
            if camera:lines.append("MOTION_VIEW "+str(directory)+"\n"+camera+"\n")
        if len(lines)!=len(groups):raise FileNotFoundError("camera log not fully flushed")
        if stage.name.startswith("stage-native-"):
            (stage.parent/("camera-native-"+stage.name.rsplit("-",1)[1]+".log")).write_text("".join(lines),encoding="utf-8")
            (stage.parent/"camera-native.log").write_text("".join(p.read_text(encoding="utf-8") for p in sorted(stage.parent.glob("camera-native-[0-9][0-9][0-9][0-9].log"))),encoding="utf-8")
        else:(stage.parent/("camera-"+recipe["stage"]+".log")).write_text("".join(lines),encoding="utf-8")
    deleted=[]
    for path in dump.root.rglob("*.bin"):
        deleted.append(dict(path=str(path),bytes=path.stat().st_size));path.unlink()
    for path in (dump.root/"Base").glob("*.log"):path.unlink()
    score.save(stage/"summary.json",dict(frames=len(groups),frame_offset=recipe.get("frame_offset",0),session=recipe.get("session"),engine_frame_first=frame_ids[0],engine_frame_last=frame_ids[-1],simulation_delta_seconds=1/60,identities=identities,deleted_bytes=sum(r["bytes"] for r in deleted)))
    (stage/"complete.done").write_text("complete\n")
    return len(groups)
