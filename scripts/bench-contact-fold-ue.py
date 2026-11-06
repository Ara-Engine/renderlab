"""Private UE preparation and one native-main-view batch (public Python APIs).

RENDERLAB_S1_PHASE=prepare runs once with NullRHI. The render phase runs all
configurations in one offscreen editor-game session, without input or UI actions.
"""
import hashlib
import json
import math
import os
import shutil
from pathlib import Path
import traceback

import unreal

ROOT = Path(os.environ["RENDERLAB_S1_ROOT"])
SCRIPT_SHA = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
SCENE = json.loads((ROOT / "fixture/scene.json").read_text())
LEVEL = "/Game/BenchContactFold/Bench"
OUTPUT = ROOT / ("unreal" if os.environ["RENDERLAB_S1_PHASE"]=="prepare" else os.environ.get("RENDERLAB_S1_CAPTURE","unreal-final"))
OUTPUT.mkdir(exist_ok=True)


def save(name, value):
    (OUTPUT/name).write_text(json.dumps(value,indent=2)+"\n")


def vector(v): return unreal.Vector(-100*v[2],100*v[0],100*v[1])


def prepare():
    levels = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
    if (OUTPUT/"prepared.json").exists():raise RuntimeError("S1 preparation is already complete")
    if unreal.EditorAssetLibrary.does_asset_exist(LEVEL):
        if not levels.load_level(LEVEL):raise RuntimeError("cannot resume S1 map")
    elif not levels.new_level(LEVEL): raise RuntimeError("cannot create S1 map")
    paths=unreal.EditorAssetLibrary.list_assets("/Game/BenchContactFold/Imported",recursive=True)
    if not paths:
        task = unreal.AssetImportTask()
        for key,value in dict(filename=str(ROOT/"fixture/contact-fold.gltf"),destination_path="/Game/BenchContactFold/Imported",
                              automated=True,save=True,replace_existing=False).items(): task.set_editor_property(key,value)
        unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([task])
    meshes = [unreal.load_asset(p) for p in unreal.EditorAssetLibrary.list_assets("/Game/BenchContactFold/Imported",recursive=True)]
    meshes = [m for m in meshes if isinstance(m,unreal.StaticMesh)]
    if len(meshes) != len(SCENE["meshes"]): raise RuntimeError(f"expected {len(SCENE['meshes'])} meshes, got {len(meshes)}")
    actors = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
    def spawn(cls,name,location=unreal.Vector(),rotation=unreal.Rotator()):
        existing=next((a for a in actors.get_all_level_actors() if a.get_actor_label()==name),None)
        if existing is not None:return existing
        a=actors.spawn_actor_from_class(cls,location,rotation);a.set_actor_label(name);return a
    material = unreal.load_asset("/Game/BenchContactFold/M_S1Grey") or unreal.AssetToolsHelpers.get_asset_tools().create_asset("M_S1Grey","/Game/BenchContactFold",unreal.Material,unreal.MaterialFactoryNew())
    material.set_editor_property("two_sided",True)
    lib=unreal.MaterialEditingLibrary
    lib.delete_all_material_expressions(material)
    base=lib.create_material_expression(material,unreal.MaterialExpressionConstant3Vector)
    base.set_editor_property("constant",unreal.LinearColor(.5,.5,.5,1))
    lib.connect_material_property(base,"",unreal.MaterialProperty.MP_BASE_COLOR)
    for value,prop in ((.6,unreal.MaterialProperty.MP_ROUGHNESS),(0.,unreal.MaterialProperty.MP_METALLIC)):
        node=lib.create_material_expression(material,unreal.MaterialExpressionConstant);node.set_editor_property("r",value)
        lib.connect_material_property(node,"",prop)
    lib.recompile_material(material);unreal.EditorAssetLibrary.save_loaded_asset(material)
    subsystem=unreal.get_editor_subsystem(unreal.StaticMeshEditorSubsystem) or unreal.get_default_object(unreal.StaticMeshEditorSubsystem)
    inventory=[]
    expected={m["name"]:[[min(-100*p[2] for p in m["positions"]),min(100*p[0] for p in m["positions"]),min(100*p[1] for p in m["positions"])],
                           [max(-100*p[2] for p in m["positions"]),max(100*p[0] for p in m["positions"]),max(100*p[1] for p in m["positions"])]] for m in SCENE["meshes"]}
    for mesh in sorted(meshes,key=lambda m:m.get_name()):
        nanite=subsystem.get_nanite_settings(mesh);nanite.set_editor_property("enabled",False)
        subsystem.set_nanite_settings(mesh,nanite,True)
        build=subsystem.get_lod_build_settings(mesh,0)
        build.set_editor_property("remove_degenerates",False)
        build.set_editor_property("recompute_normals",False)
        build.set_editor_property("use_high_precision_tangent_basis",True)
        subsystem.set_lod_build_settings(mesh,0,build)
        mesh.set_material(0,material);unreal.EditorAssetLibrary.save_loaded_asset(mesh)
        a=spawn(unreal.StaticMeshActor,"S1_"+mesh.get_name());a.static_mesh_component.set_static_mesh(mesh)
        a.static_mesh_component.set_cast_shadow(True)
        origin,extent=a.get_actor_bounds(False)
        inventory.append(dict(name=mesh.get_name(),triangles=mesh.get_num_triangles(0),
                              bounds_cm=[[origin.x-extent.x,origin.y-extent.y,origin.z-extent.z],
                                         [origin.x+extent.x,origin.y+extent.y,origin.z+extent.z]],nanite=False))
        actual=inventory[-1]["bounds_cm"]
        match=min(expected,key=lambda key:max(abs(actual[i][j]-expected[key][i][j]) for i in range(2) for j in range(3)))
        error=max(abs(actual[i][j]-expected[match][i][j]) for i in range(2) for j in range(3))
        inventory[-1].update(source_lane=match,bounds_error_cm=error)
        if error>.005:raise RuntimeError("imported basis/scale mismatch: "+mesh.get_name())
        del expected[match]
    if sum(m["triangles"] for m in inventory) != SCENE["triangles"]: raise RuntimeError("triangle count changed")
    light=spawn(unreal.DirectionalLight,"S1_Sun",rotation=unreal.MathLibrary.find_look_at_rotation(unreal.Vector(),vector([-v for v in SCENE["to_light"]])))
    lc=light.get_component_by_class(unreal.DirectionalLightComponent);lc.set_mobility(unreal.ComponentMobility.MOVABLE)
    lc.set_intensity(1000);lc.set_light_color(unreal.LinearColor(1,1,1,1),False);lc.set_indirect_lighting_intensity(0)
    for key,value in (("light_source_angle",math.degrees(.08)),("light_source_soft_angle",0.),("contact_shadow_length",0.),("use_temperature",False)):
        lc.set_editor_property(key,value)
    cube=unreal.load_asset("/Game/BenchContactFold/constant")
    if cube is None:
        task=unreal.AssetImportTask()
        for key,value in dict(filename=str(ROOT/"fixture/constant.dds"),destination_path="/Game/BenchContactFold",automated=True,save=True).items():task.set_editor_property(key,value)
        unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([task])
        cube=unreal.load_asset("/Game/BenchContactFold/constant")
    if not isinstance(cube,unreal.TextureCube):raise RuntimeError("constant environment did not import as cube")
    cube.set_editor_property("srgb",False);cube.set_editor_property("compression_settings",unreal.TextureCompressionSettings.TC_HDR_F32)
    unreal.EditorAssetLibrary.save_loaded_asset(cube)
    sky=spawn(unreal.SkyLight,"S1_Environment");sc=sky.get_component_by_class(unreal.SkyLightComponent)
    sc.set_mobility(unreal.ComponentMobility.MOVABLE)
    for key,value in (("source_type",unreal.SkyLightSourceType.SLS_SPECIFIED_CUBEMAP),("cubemap",cube),
                      ("lower_hemisphere_is_black",False),("real_time_capture",False),("cubemap_resolution",16)):
        sc.set_editor_property(key,value)
    sc.set_cast_shadows(False);sc.set_intensity(0);sc.recapture_sky()
    camera=spawn(unreal.CameraActor,"S1_Camera",vector(SCENE["poses"][0]["eye"]))
    camera.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(vector(SCENE["poses"][0]["eye"]),vector(SCENE["poses"][0]["target"])),False)
    camera.set_editor_property("auto_activate_for_player",unreal.AutoReceiveInput.PLAYER0)
    cc=camera.get_component_by_class(unreal.CameraComponent)
    cc.set_editor_property("field_of_view",math.degrees(2*math.atan(math.tan(math.pi/8)*1920/1080)))
    cc.set_editor_property("aspect_ratio",1920/1080);cc.set_editor_property("constrain_aspect_ratio",False)
    post=unreal.PostProcessSettings()
    for key,value in (("auto_exposure_method",unreal.AutoExposureMethod.AEM_MANUAL),("auto_exposure_apply_physical_camera_exposure",False),("auto_exposure_bias",-8.)):
        post.set_editor_property("override_"+key,True);post.set_editor_property(key,value)
    cc.set_editor_property("post_process_settings",post);cc.set_editor_property("post_process_blend_weight",1.)
    factory=unreal.BlueprintFactory();factory.set_editor_property("parent_class",unreal.GameModeBase)
    bp=unreal.load_asset("/Game/BenchContactFold/S1GameMode") or unreal.AssetToolsHelpers.get_asset_tools().create_asset("S1GameMode","/Game/BenchContactFold",unreal.Blueprint,factory)
    unreal.BlueprintEditorLibrary.compile_blueprint(bp);cls=unreal.BlueprintEditorLibrary.generated_class(bp)
    defaults=unreal.get_default_object(cls);defaults.set_editor_property("default_pawn_class",None);defaults.set_editor_property("hud_class",None)
    unreal.EditorAssetLibrary.save_loaded_asset(bp)
    world=unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
    world.get_world_settings().set_editor_property("default_game_mode",cls)
    if not levels.save_current_level():raise RuntimeError("could not save S1 map")
    save("prepared.json",dict(engine=unreal.SystemLibrary.get_engine_version(),meshes=inventory,
         triangles=sum(m["triangles"] for m in inventory),source_sha256=hashlib.sha256((ROOT/"fixture/scene.json").read_bytes()).hexdigest(),
         light=dict(source_angle=lc.get_editor_property("light_source_angle"),intensity=lc.get_editor_property("intensity"))))


def render_batch():
    world=unreal.load_object(None,LEVEL+".Bench")
    if world is None:raise RuntimeError("S1 main game world unavailable")
    camera,=unreal.GameplayStatics.get_all_actors_of_class(world,unreal.CameraActor)
    sky,=unreal.GameplayStatics.get_all_actors_of_class(world,unreal.SkyLight)
    all_configs=[(pose,environment,mode) for pose in (0,1) for environment in (0,1) for mode in ("VSM","HRT1","HRT256")]
    # A runtime memory abort preserves every completed configuration. Resume only
    # unfinished cases, using the same effective render settings and warmup.
    def completed(c):
        directory=OUTPUT/f"p{c[0]}-e{c[1]}-{c[2]}"
        return (directory/"profile.csv").exists() and (directory/"main.png").exists() and (directory/"hdr").exists()
    configs=[c for c in all_configs if not completed(c)]
    state=dict(index=-1,tick=0)
    def command(text):unreal.SystemLibrary.execute_console_command(world,text)
    command("r.ProfileGPU.ShowUI 0")
    common=["r.VSync 0","t.MaxFPS 0","t.IdleWhenNotForeground 0","r.SetNearClipPlane 1",
            "r.ShadowQuality 5","r.Shadow.Virtual.Enable 1","r.Shadow.Virtual.SMRT.RayCountDirectional 8",
            "r.Shadow.Virtual.SMRT.SamplesPerRayDirectional 8","r.Shadow.Virtual.SMRT.AdaptiveRayCount 1",
            "r.ContactShadows 0","r.AntiAliasingMethod 0","r.ScreenPercentage 100","r.EyeAdaptation.PreExposureOverride 1",
            "ShowFlag.GlobalIllumination 0","ShowFlag.LumenGlobalIllumination 0","ShowFlag.LumenReflections 0",
            "ShowFlag.ScreenSpaceReflections 0","ShowFlag.AmbientOcclusion 0","ShowFlag.DistanceFieldAO 0",
            "ShowFlag.Fog 0","ShowFlag.Atmosphere 0","ShowFlag.Bloom 0","ShowFlag.MotionBlur 0",
            "ShowFlag.DepthOfField 0",
            "r.DumpGPU.Upload 0","r.DumpGPU.Explore 0","r.DumpGPU.Stream 0","r.DumpGPU.FrameCount 1",
            "r.DumpGPU.PassParameters 1","r.DumpGPU.Texture 2","r.DumpGPU.Buffer 1"]
    for text in common:command(text)
    command("getall DirectionalLightComponent LightSourceAngle")
    command("getall DirectionalLightComponent LightColor")
    def advance():
        state["index"]+=1;state["tick"]=0
        if state["index"]>=len(configs):
            save("complete.json",dict(configurations=all_configs,resumed_completed=len(all_configs)-len(configs)));command("quit");return False
        pose,environment,mode=configs[state["index"]]
        directory=OUTPUT/f"p{pose}-e{environment}-{mode}";directory.mkdir(exist_ok=False)
        state["directory"]=directory
        p=SCENE["poses"][pose];camera.set_actor_location(vector(p["eye"]),False,False)
        camera.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(vector(p["eye"]),vector(p["target"])),False)
        sky.get_component_by_class(unreal.SkyLightComponent).set_intensity(float(environment))
        for text in (f"r.RayTracing.Shadows {int(mode!='VSM')}",f"r.RayTracing.Shadows.SamplesPerPixel {256 if mode=='HRT256' else 1}",
                     f"r.Shadow.Denoiser {0 if mode=='HRT256' else 1}","r.Shadow.Denoiser.TemporalAccumulation 1",
                     "r.Shadow.CacheWPOPrimitives 1","r.Shadow.CacheWPOPrimitives 0"):
            command(text)
        save(f"p{pose}-e{environment}-{mode}/recipe.json",dict(pose=pose,environment=environment,mode=mode,
             camera=p,script_sha256=SCRIPT_SHA,session=os.environ.get("RENDERLAB_S1_SESSION","unreal-batch-3"),
             timing="CSV 160 frames, measured rows 96-159; dumps happen after profiling, excluded"))
        unreal.log("S1_BEGIN "+str(directory))
        return True
    advance()
    def tick(delta):
        try:
            state["tick"]+=1;n=state["tick"];directory=state["directory"]
            if n==128:
                command("csvprofile startfile=s1-contact-fold-"+directory.name+".csv")
                command("csvprofile frames=160")
            if n==300:
                command("getall PlayerCameraManager CameraCachePrivate")
                command("r.DumpGPU.Root Lights")
                command("r.DumpGPU.Directory "+(directory/"native").as_posix());command("DumpGPU")
            if n==308:
                command("r.DumpGPU.Root Tonemap*")
                command("r.DumpGPU.Directory "+(directory/"hdr").as_posix());command("DumpGPU")
            if n==316:command("HighResShot 1920x1080 filename="+(directory/"main.png").as_posix())
            if n==328:
                csv=Path(unreal.Paths.project_saved_dir())/"Profiling/CSV"/("s1-contact-fold-"+directory.name+".csv")
                if not csv.exists():raise RuntimeError("native CSV was not written: "+str(csv))
                shutil.copyfile(csv,directory/"profile.csv")
                if not advance():unreal.unregister_ticker_callback(handle)
        except Exception:
            save("failure.json",dict(traceback=traceback.format_exc()));command("quit")
        return True
    handle=unreal.register_ticker_callback(tick)


try:
    if not unreal.SystemLibrary.get_engine_version().startswith("5.7.4-51494982"):raise RuntimeError("UE engine build changed")
    if os.environ["RENDERLAB_S1_PHASE"]=="prepare":prepare()
    else:render_batch()
except Exception:
    save("failure.json",dict(traceback=traceback.format_exc()));raise
