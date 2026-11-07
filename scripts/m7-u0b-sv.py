"""Native SV sky-visibility comparator for G1. All output lives on E:."""
import argparse
import contextlib
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import subprocess
import re
import csv
import time
import numpy as np

spec=importlib.util.spec_from_file_location("u0b",Path(__file__).with_name("m7-u0b.py"))
host=importlib.util.module_from_spec(spec);spec.loader.exec_module(host)
OUT=host.OUT
SOURCE=host.BASE/"scenes/SV/scene.json"


def save(path,value):path.write_text(json.dumps(value,indent=2,allow_nan=False)+"\n")


def poses():
    scene=json.loads(SOURCE.read_text());named={p["name"]:{k:[float(np.float32(x)) for x in p[k]] for k in ("eye","target")} for p in scene["poses"]}
    pan=[]
    for i in range(9):
        p={k:list(v) for k,v in named["gaps"].items()}
        for k in ("eye","target"):p[k][0]=float(np.float32(np.float32(p[k][0])+np.float32(.02)*np.float32(i)))
        pan.append(p)
    return named,pan


def scenes(args):
    scene=json.loads(SOURCE.read_text());named,pan=poses()
    for material in scene["materials"]:material["specular"]=0
    scene["poses"]=[dict(name=n,**p) for n,p in named.items()]+[dict(name=f"pan-{i:03}",**p) for i,p in enumerate(pan) if i]
    scene["reference_control"]=dict(source_scene_sha256=hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        max_depth=2,samples=2048,specular=0,primary_background="visible constant sky, matching G1",motion="gaps + float32(0.02*t) source X, 9 frames")
    root=OUT/"scenes/SVSky";root.mkdir(parents=True,exist_ok=True)
    (root/"scene.json").write_text(json.dumps(scene,separators=(",",":")))
    print("generated SV sky reference poses",len(scene["poses"]))


def capture(args):
    if args.async_off and not args.timing:raise ValueError("async attribution is a separate timing process")
    u=host.ue(allow_game=not args.timing);root=OUT/"sv-sky"/args.label;root.mkdir(parents=True,exist_ok=True)
    u.WAIT_FOR_FOREIGN_CPU_CLEAR=args.timing
    u.PACKAGE_DIR=OUT/"ue-svsky-package";u.PACKAGE=u.PACKAGE_DIR/"Windows/Engine/Binaries/Win64/UnrealGame.exe"
    u.UE_FLAGS += ["-UseFixedTimeStep","-FPS=60"]
    named,pan=poses()
    for mode in args.configs:
        config="sv-sky-"+mode
        backend="hw" if mode=="hw" else (None if mode=="off" else "sw")
        u.CONFIGS[config]=u.spec(backend,3,shadows="hrt" if mode=="hw" else "vsm",reflections="none",
            mesh_sdfs=1 if mode=="sw-mesh" else None,
            ini=["-ini:Engine:[/Script/Engine.RendererSettings]:r.RayTracing=False"] if mode in ("sw-mesh","off") else [])
        if mode in ("sw-mesh","off"):u.CONFIGS[config]["required"] += [("r.RayTracing","0")]
        if mode=="off":
            u.CONFIGS[config]["commands"] += ["r.DistanceFieldAO 0"]
            u.CONFIGS[config]["required"] += [("r.DistanceFieldAO","0")]
        u.CONFIGS[config]["commands"] += ["r.LumenScene.Radiosity 0"]
        u.CONFIGS[config]["required"] += [("r.LumenScene.Radiosity","0")]
        if args.async_off:
            u.CONFIGS[config]["commands"] += ["r.RDG.AsyncCompute 0","r.Lumen.AsyncCompute 0"]
            u.CONFIGS[config]["required"] += [("r.RDG.AsyncCompute","0"),("r.Lumen.AsyncCompute","0")]
        for start in ("overview","gaps"):
            case=root/(mode+"-"+start);case.mkdir(exist_ok=True)
            if (case/"complete.json").exists():continue
            other="gaps" if start=="overview" else "overview"
            macros=OUT/"m"/("sv"+hashlib.sha256(str(case).encode()).hexdigest()[:8]);macros.mkdir(parents=True,exist_ok=True)
            commands={}
            def event(frame,lines):commands.setdefault(frame,[]).extend(lines)
            if args.async_off:event(70,["r.RDG.AsyncCompute 0","r.Lumen.AsyncCompute 0"])
            extra=[u.macro(macros,"initial",host.pose_commands(named[start])),
                "set DirectionalLightComponent LightSourceAngle 0",*u.READBACK,
                "getall StaticMeshComponent OverrideMaterials","r.MotionBlurQuality 0"]
            if not args.timing:
                u.CONFIGS[config]["required"] += [("r.OneFrameThreadLag","0")]
                extra += ["r.OneFrameThreadLag 0","r.DumpGPU.Upload 0","r.DumpGPU.Explore 0","r.DumpGPU.Stream 0",
                    "r.DumpGPU.Texture 2","r.DumpGPU.Buffer 2","r.DumpGPU.PassParameters 1"]
                event(75,["r.DumpGPU.Root BasePass*",f"r.DumpGPU.Directory {(case/'material').as_posix()}","r.DumpGPU.FrameCount 1"])
                event(76,["DumpGPU"])
                event(92,["r.DumpGPU.Root PostProcessing*",f"r.DumpGPU.Directory {(case/'hdr-cut').as_posix()}","r.DumpGPU.FrameCount 3"])
                event(95,["DumpGPU"])
                event(192,["r.DumpGPU.Root Tonemap*",f"r.DumpGPU.Directory {(case/'hdr-pan').as_posix()}","r.DumpGPU.FrameCount 9"])
                event(195,["DumpGPU"])
            event(97,["getall PlayerCameraManager CameraCachePrivate",*host.pose_commands(named[other]),
                "set PlayerCameraManager bGameCameraCutThisFrame true","getall PlayerCameraManager bGameCameraCutThisFrame"])
            event(98,["getall PlayerCameraManager CameraCachePrivate","set PlayerCameraManager bGameCameraCutThisFrame false"])
            event(99,["getall PlayerCameraManager CameraCachePrivate"])
            if other!="gaps":event(128,host.pose_commands(named["gaps"])+["set PlayerCameraManager bGameCameraCutThisFrame true"])
            for i,p in enumerate(pan):event(196+i,(["getall PlayerCameraManager CameraCachePrivate"] if i else [])+host.pose_commands(p))
            event(205,["getall PlayerCameraManager CameraCachePrivate"])
            events=[(f,u.macro(macros,"m"+str(f),lines)) for f,lines in commands.items()]
            expected=[named[start],named[other],named[other],*pan]
            expected=[dict(p,fov_y_radians=float(np.float32(math.pi/4))) for p in expected]
            with u.lease("sv-sky-"+args.label+"-"+mode+"-"+start,3) as owner:
                if args.timing:u.schedule_line(f"[m7-u0b QUIET window] SV {args.label}/{mode}/{start} token={owner['token']}; exclusive GPU, wallpaper paused, no compiler/heavy build until owned release.")
                with (host.quiet_desktop(u,case) if args.timing else contextlib.nullcontext()) as quiet_window:
                    u.launch_ue(case,config,"static",extra=extra,events=events,frames=230,level="/Game/M7U0b/SVSky/Bench")
                    if not u.readback(case,config,u.scalability_expectations()):raise RuntimeError("SV sky readback failed")
                    camera=host.check_camera((case/"ue.log").read_text(encoding="utf-8-sig",errors="replace"),expected)
                    save(case/"camera-readback.json",dict(frames=camera,all_ok=True))
            save(case/"recipe.json",dict(config=config,poses=expected,start=start,cut_to=other,
                capture_only=not args.timing,async_compute=not args.async_off,fixed_delta_seconds=1/60,primary_background=[4000]*3,
                static_csv=96,cut_csv=97,pan_csv=list(range(196,205)),source_scene_sha256=u.sha(SOURCE),
                package_sha256=u.sha(u.PACKAGE),settings=u.CONFIGS[config],quiet_window=str(quiet_window) if quiet_window else None))
            if not args.timing:
                for stage in ("cut","pan"):
                    with (case/("decode-"+stage+".log")).open("w") as log:
                        subprocess.run(["py","-3","-B",str(Path(__file__).with_name("m7-u0b-score.py")),"decode",
                            str(case/("hdr-"+stage)),str(case/("native-"+stage+".npz")),"--stops","-11"],
                            stdout=log,check=True,creationflags=subprocess.CREATE_NO_WINDOW)
            save(case/"complete.json",dict(runtime_and_camera_pass=True,timing=args.timing))
            print("SV sky captured",mode,start,flush=True)
            time.sleep(3)


def compact(args):
    root=OUT/"reference/sv-sky";scene=json.loads((OUT/"scenes/SVSky/scene.json").read_text())
    for pose in scene["poses"]:
        name="SVSky-"+pose["name"];path=root/(name+".json");meta=json.loads(path.read_text())
        if meta["max_depth"]!=2 or meta["samples"]!=2048:raise ValueError("G1 reference settings mismatch")
        meta.update(exposure_stops=-11,source_scene_sha256=hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
            frame=dict(name=pose["name"],open={k:pose[k] for k in ("eye","target")},close={k:pose[k] for k in ("eye","target")},
                fov_y=math.pi/4,sky_only=True),reference_model="G1 sky visibility, Lambert specular0, max_depth2, constant4000 sky")
        save(path,meta)
        if not path.with_suffix(".npz").exists():
            subprocess.run(["py","-3","-B",str(Path(__file__).with_name("m7-u0b-score.py")),"compact",str(root),name],check=True,creationflags=subprocess.CREATE_NO_WINDOW)


def populations(scene,arrays,pose):
    hit=arrays["hit"];placement=hit[...,0];covered=placement!=0
    normal=np.zeros((*covered.shape,3),"f4")
    for i,mesh in enumerate(scene["meshes"]):
        positions=np.asarray(mesh["positions"],"f4");tri=positions[np.asarray(mesh["indices"]).reshape(-1,3)]
        n=np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0]);length=np.linalg.norm(n,axis=1)
        n=np.divide(n,length[:,None],out=np.zeros_like(n),where=length[:,None]>0)
        mask=placement==i+1;normal[mask]=n[hit[...,1][mask]-1]
    h,w=covered.shape;edge=np.zeros_like(covered)
    p=np.pad(placement,1,mode="edge");n=np.pad(normal,((1,1),(1,1),(0,0)),mode="edge")
    for y in (-1,0,1):
        for x in (-1,0,1):edge|=(p[1+y:1+y+h,1+x:1+x+w]!=placement)|((n[1+y:1+y+h,1+x:1+x+w]*normal).sum(-1)<np.float32(.999))
    valid=covered&~edge;result={"frame":valid,"edges":covered&edge}
    world=arrays["world"]
    ground=next(i+1 for i,m in enumerate(scene["meshes"]) if m["name"]=="ground")
    for name,meshes in scene["populations"].items():
        ids=[i+1 for i,m in enumerate(scene["meshes"]) if m["name"] in meshes]
        result[name]=valid&np.isin(placement,ids)
        if name!="open":
            xyz=np.concatenate([np.asarray(m["positions"],"f4") for m in scene["meshes"] if m["name"] in meshes])
            lo,hi=xyz.min(0),xyz.max(0)
            # The SV ground is at y=0, matching G1's ground-ray intersection.
            result["ground-near-"+name]=valid&(placement==ground)&(world[...,0]>lo[0]-.3)&(world[...,0]<hi[0]+.3)&(world[...,2]>lo[2]-.3)&(world[...,2]<hi[2]+.3)
    return result


def score(args):
    ds=host.load("ue-shadow-dump");sc=host.load("m7-u0b-score")
    scene=json.loads(SOURCE.read_text());named,pan=poses();refs={}
    for path in (OUT/"reference/sv-sky").glob("*.npz"):
        meta=json.loads(path.with_suffix(".json").read_text())
        if meta["max_depth"]!=2 or meta["archive_sha256"]!=sc.sha(path):raise ValueError("reference identity/settings")
        with np.load(path) as a:
            rgb=(a["L0"]+a["L1"])/2;lum=(rgb*np.array([.2126,.7152,.0722])).sum(-1)
            noise=(np.abs(a["L0"]-a["L1"])*np.array([.2126,.7152,.0722])).sum(-1)/2
            masks=populations(scene,a,meta["frame"]["open"])
        refs[meta["pose"]]=dict(rgb=rgb,lum=lum,se=noise,masks=masks,sha256=meta["archive_sha256"])
    if len(refs)!=10:raise ValueError("need both poses and all nine registered pan references")
    refs["pan-000"]=refs["gaps"]
    result=[]
    def p90(values):
        values=np.sort(values);return float(values[len(values)*9//10])
    def metrics(native,ref):
        lum=(native*np.array([.2126,.7152,.0722])).sum(-1);r=ref["lum"];se=ref["se"];masks=ref["masks"]
        opened=p90(r[masks["open"]]);keep=(r>1e-6)&(se<=.02*r);e=(lum-r)/np.maximum(r,.05*opened)
        rows={}
        for name,mask in masks.items():
            good=mask&keep;n=int(good.sum())
            if not n:continue
            crevice=good&(r<.7*opened);op=good&(r>=.95*opened)
            rows[name]=dict(n=n,excluded_dark_or_noisy=int((mask&~keep).sum()),rms=float(np.sqrt((e[good]**2).mean())),
                bias=float(e[good].mean()),p99=float(np.quantile(abs(e[good]),.99,method="lower")),
                reference_noise_rms=float(np.sqrt(((se[good]/r[good])**2).mean())),
                crevice_n=int(crevice.sum()),crevice_rms=float(np.sqrt((e[crevice]**2).mean())) if crevice.any() else None,
                open_n=int(op.sum()),over_occluded_open=int((op&(lum<.95*r)).sum()))
        return rows
    for case in sorted((OUT/"sv-sky"/args.label).iterdir()):
        if not (case/"complete.json").exists():continue
        recipe=json.loads((case/"recipe.json").read_text());runtime=json.loads((case/"readback.json").read_text())
        if not runtime["all_ok"]:raise ValueError("runtime settings failed")
        if 'shaderplatform="PCD3D_SM6"' not in (case/"ue.log").read_text(encoding="utf-8-sig",errors="replace"):
            raise ValueError("actual D3D12 SM6 platform proof failed")
        if not (case/"material-audit.json").exists():raise ValueError("diffuse-only material audit missing")
        cut=json.loads((case/"native-cut.json").read_text())["frames"];motion=json.loads((case/"native-pan.json").read_text())["frames"]
        if len(cut)!=3 or len(motion)!=9:raise ValueError("capture coverage mismatch")
        def hdr(record):
            p=Path(record["archive"])
            if sc.sha(p)!=record["archive_sha256"] or record["pre_exposure"]!=2**-11:raise ValueError("native identity/exposure")
            with np.load(p) as a:return a["hdr"][...,:3]/record["pre_exposure"]
        static=hdr(cut[0]);first=hdr(cut[1]);frames=[hdr(p) for p in motion]
        dump=ds.Dump(case/"hdr-cut");proof=[]
        for event in dump.passes:
            if "RejectShading" not in event["EventName"]:continue
            params=dump.scalar_parameters(event)
            proof.append(dict(camera_cut=params.get("CommonParameters.bCameraCut"),input_jitter=params.get("CommonParameters.InputJitter")))
        if [p["camera_cut"] for p in proof]!=[0,1,0]:raise ValueError("first-cut shader proof failed")
        record=dict(case=str(case),config=recipe["config"],start=recipe["start"],cut_to=recipe["cut_to"],
            runtime=runtime["effective_mode"],platform="D3D12 PCD3D_SM6",tsr_cut_proof=proof,
            static=metrics(static,refs[recipe["start"]]),first_cut=metrics(first,refs[recipe["cut_to"]]))
        # G1 uses the final pan frame's non-edge placement mask for every frame.
        mask=refs["pan-008"]["masks"]["frame"];r0=refs["pan-000"]["lum"]
        floor=.05*p90(r0[mask]);changes=[]
        for i in range(1,9):
            a,b=refs[f"pan-{i-1:03}"]["lum"],refs[f"pan-{i:03}"]["lum"]
            good=mask&(a>1e-6)&(b>1e-6)
            l0=(frames[i-1]*np.array([.2126,.7152,.0722])).sum(-1);l1=(frames[i]*np.array([.2126,.7152,.0722])).sum(-1)
            delta=(l1-b)/np.maximum(b,floor)-(l0-a)/np.maximum(a,floor)
            changes.append(float(np.sqrt((delta[good]**2).mean())))
        record["motion"]=dict(frames=changes,mean=float(np.mean(changes)),max=max(changes),pan_scores=[metrics(f,refs[f"pan-{i:03}"]) for i,f in enumerate(frames)])
        save(case/"g1-scores.json",record);result.append(record)
        ref=refs[recipe["start"]]["rgb"];pixels=np.concatenate([sc.codes(ref,-11),sc.codes(static,-11),8*abs(sc.codes(ref,-11)-sc.codes(static,-11))],axis=1)
        from PIL import Image
        Image.fromarray(np.rint(np.clip(pixels,0,255)).astype("u1")).save(case/"comparison.png")
    save(OUT/"sv-sky"/args.label/"g1-comparator.json",dict(reference="regenerated G1 estimator: same registered SV, 2048 spp, depth2/specular0/sky4000",
        original_reference_caches="deleted at G1 unit close; regenerated, not byte-identity claimed",scope="G1 floored linear luminance metric; edges separate; candidate is shipping TSR Epic",cases=result))
    print(json.dumps([{k:r[k] for k in ("config","start","cut_to")} | {"static_rms":r["static"]["frame"]["rms"],"cut_rms":r["first_cut"]["frame"]["rms"],"flicker":r["motion"]["mean"]} for r in result],indent=1))


def audit(args):
    """Retain actual GBuffer material bytes and source identities before raw cleanup."""
    ds=host.load("ue-shadow-dump");sc=host.load("m7-u0b-score");root=OUT/"sv-sky"/args.label
    rows={}
    for case in sorted(root.iterdir()):
        if not (case/"complete.json").exists():continue
        target=case/"material-audit.json"
        if target.exists():rows[case.name]=json.loads(target.read_text());continue
        recipe=json.loads((case/"recipe.json").read_text())
        if recipe["capture_only"] is False:continue
        dump=ds.Dump(case/"material")
        resource=next(r for r in dump.resources.values() if r["Name"]=="GBufferB")
        prefix=resource["UniqueResourceName"]
        writes=[(p,n) for p in dump.passes for n in p["OutputResources"] if n.split(".")[0]==prefix]
        event,name=writes[-1];path=dump.root/"Resources"/f"{name}.v{event['Pointer']}.bin"
        if resource["Format"]!="PF_B8G8R8A8":raise ValueError("unreviewed GBuffer material format")
        values=np.fromfile(path,"u1").reshape(resource["ExtentY"],resource["ExtentX"],4)[...,[2,1,0,3]]
        with np.load(OUT/"reference/sv-sky"/("SVSky-"+recipe["start"]+".npz")) as ref:
            mask=ref["hit"][...,0]!=0
        padded=np.pad(mask,1);safe=mask.copy();h,w=mask.shape
        for y in range(3):
            for x in range(3):safe &= padded[y:y+h,x:x+w]
        selected=values[safe,:3];lo=selected.min(0);hi=selected.max(0);median=float(np.median(selected[:,2])/255)
        if lo[0]!=0 or hi[0]!=0 or lo[1]!=0 or hi[1]!=0 or abs(median-.8)>1/255:
            raise ValueError("diffuse-only material readback failed")
        retained=case/"material-readback.npz";sc.archive(retained,dict(gbuffer_b=values,covered_interior=safe))
        row=dict(n=int(safe.sum()),metallic_minmax=[int(lo[0]),int(hi[0])],
            specular_minmax=[int(lo[1]),int(hi[1])],roughness_median=median,
            resource_sha256=sc.sha(path),archive_sha256=sc.sha(retained),source_scene_sha256=recipe["source_scene_sha256"])
        deleted={}
        for raw in dump.root.rglob("*.bin"):
            if raw.stat().st_size>5_000_000:
                deleted[str(raw.relative_to(dump.root))]=dict(bytes=raw.stat().st_size,sha256=sc.sha(raw));raw.unlink()
        row["deleted_raw"]=deleted;save(target,row);rows[case.name]=row
    save(root/"material-audits.json",rows)
    print("material audits",list(rows),flush=True)


def costs(args):
    """CSV GPU groups at the exact static, first-cut and pan frames, without rebinning."""
    root=OUT/"sv-sky"/args.label;result=[]
    fields=("GPUTime","GPU/UpdateLumenSceneBuffers","GPU/LumenSceneUpdate",
            "GPU/LumenSceneLighting","GPU/LumenScreenProbeGather","GPU/TemporalSuperResolution",
            "View/PosX","View/PosY","View/PosZ")
    for case in sorted(root.iterdir()):
        if not (case/"complete.json").exists():continue
        recipe=json.loads((case/"recipe.json").read_text());rows=[]
        cvars=host.ue().cvar_rows(case/"cvars-timed.csv")
        lag=int(cvars["r.OneFrameThreadLag"]["value"])
        if lag!=(0 if recipe["capture_only"] else 1):raise ValueError("capture-only render-thread synchronization leaked into cost process")
        if any(int(cvars[k]["value"])!=int(recipe.get("async_compute",True))
               for k in ("r.RDG.AsyncCompute","r.Lumen.AsyncCompute")):
            raise ValueError("async attribution state mismatch")
        with (case/"profile.csv").open(newline="",encoding="utf-8-sig") as stream:
            reader=csv.DictReader(stream);absent=[key for key in fields if key not in reader.fieldnames]
            if any(not key.startswith("GPU/") for key in absent):raise ValueError("missing GPU CSV frame/camera columns")
            for row in reader:
                try:row={key:float(row.get(key,0)) for key in fields}
                except (ValueError,KeyError,TypeError):continue
                rows.append(row)
        # CSV profiling drains several GPU query frames after the requested 230.
        if len(rows)<230:raise ValueError("CSV frame coverage")
        selected=[96,97,*range(196,205)]
        expected=[recipe["poses"][0],recipe["poses"][1],*recipe["poses"][3:]]
        for frame,pose in zip(selected,expected,strict=True):
            x,y,z=pose["eye"];target=[-100*z,100*x,100*y]
            actual=[rows[frame]["View/Pos"+axis] for axis in "XYZ"]
            if max(abs(a-b) for a,b in zip(actual,target))>1e-3:
                raise ValueError(f"GPU CSV camera/frame identity failed at {frame}")
        value=dict(config=recipe["config"],start=recipe["start"],capture_instrumented=recipe["capture_only"],one_frame_thread_lag=lag,async_compute=recipe.get("async_compute",True),
            timing_eligible_for_claim=False,
            timing_scope="functional exclusive slot, not a quiet-window speed claim; group values separately, no inclusive summation",
            static=rows[96],first_cut=rows[97],pan=[rows[i] for i in range(196,205)],
            camera_csv_proof="all selected GPU rows match registered source eye within 0.001 cm",
            absent_csv_groups_reported_as_zero=absent,captured_csv_rows=len(rows),source_csv_sha256=host.ue().sha(case/"profile.csv"),
            warm_profile_async_off=host.ue().pass_costs(case))
        save(case/"g1-costs.json",value);result.append(value)
    save(root/"g1-costs.json",result)
    print("SV frame costs",len(result),flush=True)


def calibration(args):
    """Lumen/DFAO-off constant sky must give Lambert .5 * 4000 on every ground pixel.
    Compare to the analytic unoccluded value, not the physically occluded reference."""
    scene=json.loads(SOURCE.read_text());ground=next(i+1 for i,m in enumerate(scene["meshes"]) if m["name"]=="ground")
    root=OUT/"sv-sky"/args.label;result={}
    for pose in ("overview","gaps"):
        case=root/("off-"+pose);record=json.loads((case/"native-cut.json").read_text())["frames"][0]
        with np.load(record["archive"]) as a:rgb=a["hdr"][...,:3]/record["pre_exposure"]
        with np.load(OUT/"reference/sv-sky"/("SVSky-"+pose+".npz")) as a:
            mask=populations(scene,a,{})["frame"]&(a["hit"][...,0]==ground)
        values=rgb[mask];expected=2000.
        result[pose]=dict(n=int(mask.sum()),analytic_unoccluded_cd_m2=expected,
            RGB_quantiles=np.quantile(values,[0,.01,.5,.99,1],axis=0).tolist(),mean=values.mean(0).tolist(),
            relative_rgb_rms=float(np.sqrt(((values/expected-1)**2).mean())),
            native_archive_sha256=record["archive_sha256"],gain_fitted=False,
            population="all covered nonedge ground; lighting control deliberately has no occlusion")
    save(root/"unoccluded-calibration.json",result);print("SV analytic calibration",result,flush=True)


def main():
    p=argparse.ArgumentParser(description=__doc__);s=p.add_subparsers(dest="command",required=True)
    s.add_parser("scenes")
    s.add_parser("compact")
    q=s.add_parser("score");q.add_argument("label")
    q=s.add_parser("audit");q.add_argument("label")
    q=s.add_parser("costs");q.add_argument("label")
    q=s.add_parser("calibration");q.add_argument("label")
    q=s.add_parser("capture");q.add_argument("label");q.add_argument("--configs",nargs="+",choices=("sw","hw","sw-mesh","off"),default=["sw","hw"])
    q.add_argument("--timing",action="store_true");q.add_argument("--async-off",action="store_true")
    a=p.parse_args();globals()[a.command](a)


if __name__=="__main__":main()
