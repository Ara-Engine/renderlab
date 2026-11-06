"""One UE session per S2/S3 bench; native VSM/HRT and uninterrupted timed tracks.

Uses only public Unreal Python/console APIs. Native dumps are compacted by the
supervisor, so raw full-resolution resources do not accumulate on the shared disk.
"""
import json
import math
import os
from pathlib import Path
import shutil
import traceback
import time

import unreal

ROOT=Path(os.environ["RENDERLAB_MOTION_ROOT"])
SCENE=json.loads((ROOT/"fixture/scene.json").read_text())
BENCH=SCENE["bench"]
os.environ["RENDERLAB_S1_ROOT"]=str(ROOT)
os.environ["RENDERLAB_S1_PHASE"]=os.environ["RENDERLAB_MOTION_PHASE"]
# Reuse the retained S1 import, basis/bounds/triangle checks and camera setup.
prefix=Path(__file__).with_name("bench-contact-fold-ue.py").read_text().split("\ndef render_batch():")[0]
prefix=prefix.replace("BenchContactFold","BenchMotion"+BENCH)
# The black-world run can reuse S1's own authored cube; avoid a redundant
# import and the installed commandlet's post-save Slate assertion.
prefix=prefix.replace('cube=unreal.load_asset("/Game/BenchMotion'+BENCH+'/constant")',
                      'cube=unreal.load_asset("/Game/BenchContactFold/constant")',1)
exec(compile(prefix,str(Path(__file__)),"exec"),globals())
OUTPUT=ROOT/"unreal";OUTPUT.mkdir(exist_ok=True)


def prepare_motion():
    prepare()
    actors=unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
    inventory=json.loads((OUTPUT/"prepared.json").read_text())
    source={m["name"]:m for m in SCENE["meshes"]}
    for item in inventory["meshes"]:
        g=source[item["source_lane"]]
        actor=next(a for a in actors.get_all_level_actors() if a.get_actor_label()=="S1_"+item["name"])
        if g.get("moving"):actor.static_mesh_component.set_mobility(unreal.ComponentMobility.MOVABLE)
        if g["roughness"]!=.6:
            lib=unreal.MaterialEditingLibrary
            name="M_Roughness"+str(g["roughness"]).replace(".","_")
            path="/Game/BenchMotion"+BENCH
            material=unreal.load_asset(path+"/"+name) or unreal.AssetToolsHelpers.get_asset_tools().create_asset(name,path,unreal.Material,unreal.MaterialFactoryNew())
            material.set_editor_property("two_sided",True);lib.delete_all_material_expressions(material)
            base=lib.create_material_expression(material,unreal.MaterialExpressionConstant3Vector);base.set_editor_property("constant",unreal.LinearColor(.5,.5,.5,1))
            lib.connect_material_property(base,"",unreal.MaterialProperty.MP_BASE_COLOR)
            rough=lib.create_material_expression(material,unreal.MaterialExpressionConstant);rough.set_editor_property("r",g["roughness"])
            lib.connect_material_property(rough,"",unreal.MaterialProperty.MP_ROUGHNESS)
            lib.recompile_material(material);unreal.EditorAssetLibrary.save_loaded_asset(material)
            actor.static_mesh_component.set_material(0,material)
    camera=next(a for a in actors.get_all_level_actors() if isinstance(a,unreal.CameraActor))
    cc=camera.get_component_by_class(unreal.CameraComponent)
    cc.set_editor_property("field_of_view",math.degrees(2*math.atan(math.tan(math.radians(SCENE["fov_y_degrees"])/2)*1920/1080)))
    if not unreal.get_editor_subsystem(unreal.LevelEditorSubsystem).save_current_level():raise RuntimeError("motion map save failed")
    save("prepared-motion.json",dict(bench=BENCH,triangles=SCENE["triangles"],nanite=False,engine=unreal.SystemLibrary.get_engine_version(),materials="per-mesh registered roughness"))
    unreal.SystemLibrary.execute_console_command(unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world(),"DumpConsoleCommands r.Shadow.Virtual")


def render_motion():
    world=unreal.load_object(None,LEVEL+".Bench")
    if world is None:raise RuntimeError("main game world missing")
    actors=unreal.GameplayStatics.get_all_actors_of_class(world,unreal.Actor)
    camera,=[a for a in actors if isinstance(a,unreal.CameraActor)]
    light,=[a for a in actors if isinstance(a,unreal.DirectionalLight)]
    lc=light.get_component_by_class(unreal.DirectionalLightComponent)
    inventory=json.loads((OUTPUT/"prepared.json").read_text())
    moving={item["source_lane"]:next(a for a in actors if a.get_name().startswith("StaticMeshActor") and a.static_mesh_component.get_editor_property("static_mesh").get_name()==item["name"])
            for item in inventory["meshes"] if item["source_lane"] in ("reveal-plate","shadow-blocker")}
    def command(text):unreal.SystemLibrary.execute_console_command(world,text)
    common=["r.ProfileGPU.ShowUI 0","r.VSync 0","t.MaxFPS 0","t.IdleWhenNotForeground 0","r.SetNearClipPlane 1",
        "r.ShadowQuality 5","r.Shadow.Virtual.Enable 1","r.Shadow.Virtual.Cache 1","CsvCategory VSM enable",
        "r.Shadow.Virtual.SMRT.RayCountDirectional 8","r.Shadow.Virtual.SMRT.SamplesPerRayDirectional 8","r.Shadow.Virtual.SMRT.AdaptiveRayCount 1",
        "r.ContactShadows 0","r.AntiAliasingMethod 4","r.ScreenPercentage 100","r.TSR.History.ScreenPercentage 200",
        "sg.AntiAliasingQuality 3","r.EyeAdaptation.PreExposureOverride 1",
        "ShowFlag.GlobalIllumination 0","ShowFlag.LumenGlobalIllumination 0","ShowFlag.LumenReflections 0","ShowFlag.ScreenSpaceReflections 0",
        "ShowFlag.AmbientOcclusion 0","ShowFlag.DistanceFieldAO 0","ShowFlag.Fog 0","ShowFlag.Atmosphere 0","ShowFlag.Bloom 0","ShowFlag.MotionBlur 0","ShowFlag.DepthOfField 0",
        "r.DumpGPU.Upload 0","r.DumpGPU.Explore 0","r.DumpGPU.Stream 0","r.DumpGPU.FrameCount 1","r.DumpGPU.PassParameters 1","r.DumpGPU.Texture 2","r.DumpGPU.Buffer 0"]
    for text in common:command(text)
    configs=[(track,radius,mode) for track in SCENE["tracks"] for radius in SCENE["radii"] for mode in ("VSM","HRT1","HRT256")]
    state=dict(config=-1,tick=0,stage="warm",repeat=0,index=0,engine_tick=0,records=[])
    def pose(p):
        camera.set_actor_location(vector(p["eye"]),False,False)
        camera.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(vector(p["eye"]),vector(p["target"])),False)
        light.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(unreal.Vector(),vector([-v for v in p["to_light"]])),False)
        for name,actor in moving.items():actor.set_actor_location(vector([p["plate_x"] if name=="reveal-plate" else p["blocker_x"],0,0]),False,False)
    def advance():
        state["config"]+=1
        if state["config"]==len(configs):
            save("complete.json",dict(configurations=len(configs),engine_ticks=state["engine_tick"]));command("quit");return False
        track,radius,mode=configs[state["config"]]
        case=f"{track['name']}-r{radius:g}";directory=OUTPUT/case/mode
        if (directory/"config-complete.json").exists():return advance()
        directory.mkdir(parents=True,exist_ok=True)
        # The 256-spp mask has no denoiser history. Export bounded subsets from
        # full input replays so a host-memory stop does not discard 385 frames.
        # Its final post-TSR HDR is always one uninterrupted full-track replay.
        capture_specs=(["depth"] if mode=="VSM" else ["parameters"] if mode=="HRT1" else [])+["native","hdr"]
        if BENCH=="S3" and mode=="HRT256":capture_specs=[f"native-{i:04}" for i in (0,64,128,192,256,320)]+["hdr"]
        captured=all((directory/("stage-"+kind)/"complete.done").exists() for kind in capture_specs if kind!="depth")
        state.update(track=track,radius=radius,mode=mode,directory=directory,stage="warm",tick=0,index=0,repeat=0,
                     captures=[] if captured else capture_specs)
        state["csv_name"]=f"motion-{BENCH}-{state['config']}-{os.environ['RENDERLAB_MOTION_SESSION']}.csv"
        lc.set_visibility(False)
        lc.set_editor_property("light_source_angle",math.degrees(2*radius))
        lc.set_visibility(True)
        for text in (f"r.RayTracing.Shadows {int(mode!='VSM')}",f"r.RayTracing.Shadows.SamplesPerPixel {256 if mode=='HRT256' else 1}",f"r.Shadow.Denoiser {int(mode!='HRT256')}","r.Shadow.Denoiser.TemporalAccumulation 1"):
            command(text)
        pose(track["frames"][0]);save(str(directory.relative_to(OUTPUT)/"recipe.json"),dict(case=case,mode=mode,radius=radius,commands=common,track_frames=len(track["frames"]),repeats=8,
            dump_scope="Full 60 Hz input replays. S3 HRT256 memoryless masks use bounded subsets; final HDR and all production-mode masks are uninterrupted tracks."))
        unreal.log("MOTION_BEGIN "+str(directory));return True
    def tick(delta):
        try:
            # Read the camera after the scheduled frame has rendered, before
            # changing actors for the next frame. This works on either side of
            # the engine tick and avoids attributing the previous camera cache
            # to the newly scheduled pose.
            pending=state.pop("pending_view",None)
            if pending is not None:
                unreal.log("MOTION_VIEW "+str(pending));command("getall PlayerCameraManager CameraCachePrivate")
            state["engine_tick"]+=1;state["tick"]+=1
            n=state["tick"];frames=state["track"]["frames"];directory=state["directory"]
            if state["stage"]=="warm":
                if n==64:
                    command("csvprofile startfile="+state["csv_name"])
                    command("csvprofile start")
                    state.update(stage="timing",tick=0,index=0,repeat=0)
                return True
            if state["stage"]=="timing":
                i=state["index"];pose(frames[i])
                state["records"].append(dict(config=state["config"],stage="timing",engine_tick=state["engine_tick"],repeat=state["repeat"],frame=i))
                state["index"]+=1
                if state["index"]==len(frames):
                    state["repeat"]+=1;state["index"]=0
                    if state["repeat"]==8:state.update(stage="timing-stop",tick=0)
                return True
            if state["stage"]=="timing-stop":
                command("csvprofile stop");state.update(stage="capture-start" if state["captures"] else "finish",tick=0);return True
            if state["stage"]=="capture-start":
                spec=state["captures"].pop(0)
                kind="native" if spec.startswith("native-") else spec
                offset=int(spec.split("-")[1]) if spec.startswith("native-") else 0
                destination=directory.parent/"stage-depth" if kind=="depth" else directory/("stage-"+spec)
                if (destination/"complete.done").exists():
                    state.update(stage="capture-start",tick=0);return True
                destination.mkdir(exist_ok=False)
                count=1 if kind=="parameters" else len(frames)
                exported=(65 if offset==320 else 64) if spec.startswith("native-") else count
                save(str(destination.relative_to(OUTPUT)/"recipe.json"),dict(stage=kind,mode=state["mode"],frames=exported,frame_offset=offset,session=os.environ["RENDERLAB_MOTION_SESSION"]))
                unreal.log("MOTION_STAGE "+spec+" "+str(directory))
                state.update(stage="capture-warm",tick=0,index=0,capture_kind=kind,capture_spec=spec,destination=destination,capture_count=count,dump_offset=offset,dump_count=exported)
                command("r.ResetViewState");pose(frames[0]);return True
            if state["stage"]=="capture-warm":
                pose(frames[0])
                if n==64:state.update(stage="capture",tick=0,index=0)
                return True
            if state["stage"]=="capture":
                i=state["index"];p=frames[i];pose(p)
                kind=state["capture_kind"]
                exporting=state["dump_offset"]<=i<state["dump_offset"]+state["dump_count"]
                frame_dir=(directory.parent/"depth" if kind=="depth" else directory)/f"{i:04}";frame_dir.mkdir(parents=True,exist_ok=True)
                if kind!="hdr" and exporting:save(str(frame_dir.relative_to(OUTPUT)/"recipe.json"),dict(frame=i,pose=p,mode=state["mode"],radius=state["radius"],engine_tick=state["engine_tick"],ticker_delta_seconds=delta,
                    world_delta_seconds=unreal.GameplayStatics.get_world_delta_seconds(world)))
                if kind in ("native","hdr") and exporting:state["pending_view"]=frame_dir
                if i==state["dump_offset"]:
                    scopes={"VSM":"CompositeVirtualShadowMapMask*","HRT1":"*SpatialAccumulation*","HRT256":"RayTracedShadow*"}
                    if kind=="native" and state["mode"]=="HRT1":
                        parameters=json.loads((directory/"stage-parameters/effective-parameters.json").read_text())
                        if parameters["consumed_mask"]=="RayTracingOcclusion":scopes["HRT1"]="RayTracedShadow*"
                    scope="*Pre?ass*" if kind=="depth" else "Tonemap*" if kind=="hdr" else "Lights" if kind=="parameters" else scopes[state["mode"]]
                    command("r.DumpGPU.Texture "+("1" if kind=="parameters" else "2"))
                    command("r.DumpGPU.Root "+scope);command("r.DumpGPU.FrameCount "+str(state["dump_count"]))
                    command("r.DumpGPU.Directory "+(state["destination"]/"dump").as_posix());command("DumpGPU")
                if kind=="hdr" and i in (0,15,16,17,19,23,31,32,63,64,95,96,127,159,191,319,351,352,353,355,359,367,384):
                    command("Shot filename="+(frame_dir/"main.png").as_posix())
                state["records"].append(dict(config=state["config"],stage=state["stage"],engine_tick=state["engine_tick"],frame=i))
                state["index"]+=1
                if state["index"]==state["capture_count"]:state.update(stage="capture-wait",tick=0,wait_start=time.monotonic())
                return True
            if state["stage"]=="capture-wait":
                if (state["destination"]/"complete.done").exists():state.update(stage="capture-start" if state["captures"] else "finish",tick=0)
                elif time.monotonic()-state["wait_start"]>600:raise RuntimeError("batch decoder timeout "+str(state["destination"]))
                return True
            if state["stage"]=="finish" and n==32:
                csv=Path(unreal.Paths.project_saved_dir())/"Profiling/CSV"/state["csv_name"]
                if not csv.exists():raise RuntimeError("missing native timing CSV "+str(csv))
                shutil.copyfile(csv,directory/"profile.csv");save("timeline.json",state["records"])
                save(str(directory.relative_to(OUTPUT)/"config-complete.json"),dict(session=os.environ["RENDERLAB_MOTION_SESSION"],mode=state["mode"],frames=len(frames)))
                if not advance():unreal.unregister_ticker_callback(handle)
            return True
        except Exception:
            save("failure.json",dict(traceback=traceback.format_exc(),state={k:str(v) if isinstance(v,Path) else v for k,v in state.items() if k not in ("track","records")}));command("quit");return True
    if advance():handle=unreal.register_ticker_callback(tick)


def probe_motion():
    """Inspect the installed build's two-frame dump layout in the same session.

    This setup hold is outside all measured tracks. A session-scoped file lets
    the supervisor continue with the reviewed controller without reopening UE.
    """
    world=unreal.load_object(None,LEVEL+".Bench")
    session=os.environ["RENDERLAB_MOTION_SESSION"]
    directory=OUTPUT/"probe"/session;directory.mkdir(parents=True)
    state=dict(tick=0)
    def command(text):unreal.SystemLibrary.execute_console_command(world,text)
    for text in ("r.RayTracing.Shadows 0","r.Shadow.Virtual.Enable 1","r.DumpGPU.Upload 0","r.DumpGPU.Explore 0","r.DumpGPU.Stream 0",
                 "r.DumpGPU.Buffer 0","r.DumpGPU.Texture 2","r.DumpGPU.PassParameters 1","r.DumpGPU.FrameCount 2","t.MaxFPS 30"):
        command(text)
    def tick(delta):
        state["tick"]+=1
        if state["tick"]==16:
            command("r.DumpGPU.Root Light::StandardDeferred*");command("r.DumpGPU.Directory "+directory.as_posix());command("DumpGPU")
            save("probe-session.json",dict(session=session,directory=str(directory),world_delta_seconds=unreal.GameplayStatics.get_world_delta_seconds(world)))
        control=ROOT/"continue-native.json"
        if state["tick"]>24 and control.exists():
            message=json.loads(control.read_text())
            if message.get("session")==session and message.get("continue"):
                control.unlink();unreal.unregister_ticker_callback(handle);os.environ.pop("RENDERLAB_MOTION_PROBE",None)
                command("r.DumpGPU.FrameCount 1")
                exec(compile(Path(__file__).read_text(),str(Path(__file__)),"exec"),globals())
                return False
        return True
    handle=unreal.register_ticker_callback(tick)


try:
    if not unreal.SystemLibrary.get_engine_version().startswith("5.7.4-51494982"):raise RuntimeError("UE build changed")
    if os.environ["RENDERLAB_MOTION_PHASE"]=="prepare":prepare_motion()
    elif os.environ.get("RENDERLAB_MOTION_PROBE")=="1":probe_motion()
    else:render_motion()
except Exception:
    save("failure.json",dict(traceback=traceback.format_exc()))
    if os.environ["RENDERLAB_MOTION_PHASE"]=="render":
        unreal.SystemLibrary.execute_console_command(unreal.load_object(None,LEVEL+".Bench"),"quit")
    raise
