"""Private UE quarry import and direct-light capture, using public scripting APIs.

Run only through the guarded runner, in the assigned shared GPU window.
SceneCapture CPU/export spans are deliberately not reported as GPU frame times.
"""
import hashlib
import json
import math
import os
from pathlib import Path
import time

import unreal

ROOT = Path("D:/Coding/renderlab-work/ue-m5/Quarry")
MANIFEST = json.loads((ROOT / "manifest.json").read_text())
LEVEL = "/Game/M6Quarry/Quarry"
MODE = os.environ["RENDERLAB_UE_QUARRY_MODE"]
RUN = ROOT / "runs" / os.environ["RENDERLAB_UE_QUARRY_RUN"]
RUN.mkdir(parents=True, exist_ok=False)


def save(name, data):
    (RUN / name).write_text(json.dumps(data, indent=2, allow_nan=False) + "\n")


def xyz(v):
    return [v.x, v.y, v.z]


def position(v):
    return unreal.Vector(-100 * v[2], 100 * v[0], 100 * v[1])


def spawn(cls, name, location=unreal.Vector(), rotation=unreal.Rotator()):
    actor = unreal.get_editor_subsystem(unreal.EditorActorSubsystem).spawn_actor_from_class(cls, location, rotation)
    actor.set_actor_label("M6Quarry_" + name)
    return actor


def mesh_subsystem():
    return unreal.get_editor_subsystem(unreal.StaticMeshEditorSubsystem) or unreal.get_default_object(unreal.StaticMeshEditorSubsystem)


def import_quarry():
    for name, wanted in MANIFEST["staged_files_sha256"].items():
        if hashlib.sha256((ROOT / name).read_bytes()).hexdigest() != wanted:
            raise RuntimeError("staged input changed: " + name)
    levels = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
    start = time.perf_counter()
    if MODE == "assemble":
        if not levels.load_level(LEVEL):
            raise RuntimeError("cannot resume the saved quarry map")
        existing = unreal.get_editor_subsystem(unreal.EditorActorSubsystem).get_all_level_actors()
        if any(isinstance(a, unreal.StaticMeshActor) and not a.get_actor_label().startswith("M6Quarry_") for a in existing):
            raise RuntimeError("foreign scene actors; refusing to modify the map")
        paths = list(unreal.EditorAssetLibrary.list_assets("/Game/M6Quarry/Imported", recursive=True))
    else:
        if unreal.EditorAssetLibrary.does_asset_exist(LEVEL):
            raise RuntimeError("quarry map already exists; use assemble/capture rather than replacing it")
        if not levels.new_level(LEVEL):
            raise RuntimeError("cannot create private quarry level")
        task = unreal.AssetImportTask()
        for key, value in (("filename", str(ROOT / "quarry.gltf")), ("destination_path", "/Game/M6Quarry/Imported"),
                           ("automated", True), ("save", True), ("replace_existing", False), ("async_", False)):
            task.set_editor_property(key, value)
        unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([task])
        paths = list(task.get_editor_property("imported_object_paths"))
    objects = [unreal.load_asset(path) for path in paths]
    # Some Interchange versions return only the scene import record in task paths.
    if not any(isinstance(obj, unreal.StaticMesh) for obj in objects):
        objects = [unreal.load_asset(path) for path in unreal.EditorAssetLibrary.list_assets("/Game/M6Quarry/Imported", recursive=True)]
    meshes = sorted([obj for obj in objects if isinstance(obj, unreal.StaticMesh)], key=lambda m: m.get_name())
    if len(meshes) != 12:
        save("import-inventory.json", {"task_paths": paths, "objects": [str(o) for o in objects]})
        raise RuntimeError(f"expected 12 quarry meshes, got {len(meshes)}")
    records = []
    subsystem = mesh_subsystem()
    for index, mesh in enumerate(meshes):
        if mesh.get_name() != f"Quarry_{index:02d}":
            raise RuntimeError("importer mesh naming/order differs: " + mesh.get_name())
        build = subsystem.get_lod_build_settings(mesh, 0)
        nanite = subsystem.get_nanite_settings(mesh)
        if not nanite.get_editor_property("enabled"):
            raise RuntimeError("import did not enable Nanite")
        actor = next((a for a in unreal.get_editor_subsystem(unreal.EditorActorSubsystem).get_all_level_actors()
                      if a.get_actor_label() == "M6Quarry_" + mesh.get_name()), None)
        if actor is None:
            actor = spawn(unreal.StaticMeshActor, mesh.get_name())
        actor.static_mesh_component.set_static_mesh(mesh)
        actor.static_mesh_component.set_cast_shadow(True)
        origin, extent = actor.get_actor_bounds(False)
        records.append(dict(name=mesh.get_name(), asset=mesh.get_path_name(), bounds_cm=[xyz(origin - extent), xyz(origin + extent)],
                            materials=[str(x.material_interface) for x in mesh.get_editor_property("static_materials")],
                            nanite={k: str(nanite.get_editor_property(k)) for k in ("enabled", "keep_percent_triangles", "trim_relative_error", "fallback_percent_triangles", "fallback_relative_error")},
                            build_settings={k: str(build.get_editor_property(k)) for k in ("recompute_normals", "recompute_tangents", "remove_degenerates", "use_high_precision_tangent_basis", "use_full_precision_u_vs")}))
    levels.save_current_level()
    textures = []
    for texture in (o for o in objects if isinstance(o, unreal.Texture2D)):
        for key, value in (("srgb", True), ("compression_settings", unreal.TextureCompressionSettings.TC_EDITOR_ICON),
                           ("never_stream", True), ("max_texture_size", 4096), ("filter", unreal.TextureFilter.TF_DEFAULT)):
            texture.set_editor_property(key, value)
        unreal.EditorAssetLibrary.save_loaded_asset(texture)
        textures.append(dict(asset=texture.get_path_name(), properties={k: str(texture.get_editor_property(k)) for k in
                            ("srgb", "compression_settings", "never_stream", "max_texture_size", "mip_gen_settings", "filter")}))
    light = next((a for a in unreal.get_editor_subsystem(unreal.EditorActorSubsystem).get_all_level_actors()
                  if a.get_actor_label() == "M6Quarry_Sun"), None)
    if light is None:
        light = spawn(unreal.DirectionalLight, "Sun", rotation=unreal.MathLibrary.find_look_at_rotation(unreal.Vector(), unreal.Vector(*MANIFEST["expected_ue_travel"])))
    component = light.get_component_by_class(unreal.DirectionalLightComponent)
    component.set_mobility(unreal.ComponentMobility.MOVABLE)
    component.set_intensity(1000)
    component.set_light_color(unreal.LinearColor(1., .95, .85, 1.), srgb=True)
    component.set_cast_shadows(True)
    component.set_indirect_lighting_intensity(0)
    for key, value in (("light_source_angle", 4.584), ("light_source_soft_angle", 0.), ("contact_shadow_length", 0.), ("use_temperature", False)):
        component.set_editor_property(key, value)
    actual = component.get_light_color()
    levels.save_current_level()
    save("import.json", dict(engine=unreal.SystemLibrary.get_engine_version(), meshes=records, textures=textures,
                              light=dict(forward=xyz(light.get_actor_forward_vector()), rgb_lux=[1000 * v for v in (actual.r, actual.g, actual.b)],
                                         source_angle=component.get_editor_property("light_source_angle")),
                              import_and_asset_build_seconds=time.perf_counter() - start,
                              status="imported; image/render-buffer verification pending"))


def capture(game_world=None):
    if game_world is None:
        if not unreal.get_editor_subsystem(unreal.LevelEditorSubsystem).load_level(LEVEL):
            raise RuntimeError("prepared quarry level missing")
        world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
        actors = unreal.get_editor_subsystem(unreal.EditorActorSubsystem).get_all_level_actors()
    else:
        world = game_world
        actors = unreal.GameplayStatics.get_all_actors_of_class(world, unreal.Actor)
    light = next(a for a in actors if isinstance(a, unreal.DirectionalLight))
    light_component = light.get_component_by_class(unreal.DirectionalLightComponent)
    mode = os.environ.get("RENDERLAB_UE_QUARRY_SHADOW", "vsm-smrt")
    cvars = {"r.ShadowQuality": 5, "r.Shadow.Virtual.Enable": 1, "r.Shadow.Virtual.SMRT.RayCountDirectional": 8,
             "r.Shadow.Virtual.SMRT.SamplesPerRayDirectional": 8, "r.Shadow.Virtual.SMRT.AdaptiveRayCount": 1,
             "r.RayTracing.Shadows": int(mode == "hardware-rt-denoised"), "r.RayTracing.Shadows.SamplesPerPixel": 1,
             "r.Shadow.Denoiser": 1, "r.Shadow.Denoiser.TemporalAccumulation": 1,
             "r.AntiAliasingMethod": 0, "r.ScreenPercentage": 100, "r.MaxAnisotropy": 16,
             "r.RayTracing.Nanite.Mode": 1, "r.RayTracing.Nanite.CutError": 0,
             "r.RayTracing.Nanite.Streaming": 1, "r.RayTracing.Nanite.Streaming.LODBias": 0,
             "r.RayTracing.Nanite.Streaming.Offscreen.LODBias": 0, "r.RayTracing.Nanite.Streaming.Offscreen.MinCutError": 0,
             "r.Nanite.MaxPixelsPerEdge": .25}
    if mode not in ("vsm-smrt", "hardware-rt-denoised"):
        raise RuntimeError("unknown shadow mode")
    for key, value in cvars.items():
        unreal.SystemLibrary.execute_console_command(world, f"{key} {value}")
        unreal.SystemLibrary.execute_console_command(world, key)
    if game_world is None:
        camera = spawn(unreal.SceneCapture2D, "Capture")
    else:
        camera = next(a for a in actors if isinstance(a, unreal.SceneCapture2D))
    component = camera.get_component_by_class(unreal.SceneCaptureComponent2D)
    target = unreal.RenderingLibrary.create_render_target2d(world, 1920, 1080, unreal.TextureRenderTargetFormat.RTF_RGBA32F)
    component.set_editor_property("texture_target", target)
    component.set_editor_property("capture_source", unreal.SceneCaptureSource.SCS_SCENE_COLOR_HDR)
    component.set_editor_property("capture_every_frame", False)
    component.set_editor_property("capture_on_movement", False)
    component.set_editor_property("always_persist_rendering_state", True)
    component.set_editor_property("use_ray_tracing_if_enabled", True)
    component.set_editor_property("render_in_main_renderer", False)
    component.set_editor_property("override_custom_near_clipping_plane", True)
    component.set_editor_property("custom_near_clipping_plane", .5)
    component.set_editor_property("fov_angle", math.degrees(2 * math.atan(math.tan(.85 / 2) * 1920 / 1080)))
    disabled = ["GlobalIllumination", "LumenGlobalIllumination", "LumenReflections", "ScreenSpaceReflections", "ReflectionEnvironment",
                "AmbientOcclusion", "DistanceFieldAO", "ContactShadows", "SkyLighting", "Bloom", "Vignette", "Grain", "MotionBlur",
                "Fog", "VolumetricFog", "Atmosphere", "EyeAdaptation", "Tonemapper", "AntiAliasing"]
    environment = MODE == "environment-game"
    if environment:
        disabled = [n for n in disabled if n not in ("SkyLighting", "ReflectionEnvironment")]
    component.set_editor_property("show_flag_settings", [unreal.EngineShowFlagsSetting(show_flag_name=n, enabled=False) for n in disabled])
    post = unreal.PostProcessSettings()
    for key, value in (("auto_exposure_method", unreal.AutoExposureMethod.AEM_MANUAL),
                       ("auto_exposure_apply_physical_camera_exposure", False), ("auto_exposure_bias", 0.)):
        post.set_editor_property("override_" + key, True)
        post.set_editor_property(key, value)
    component.set_editor_property("post_process_settings", post)
    component.set_editor_property("post_process_blend_weight", 1.)
    unreal.AutomationLibrary.finish_loading_before_screenshot()
    options = unreal.ImageWriteOptions(format=unreal.DesiredImageFormat.EXR, compression_quality=1, overwrite_file=False, async_=False)
    records = []
    if game_world is not None:
        state = dict(index=0, frame=0, warmup=0)
        # Keep shadowed histories contiguous across the registered motion/return.
        schedule = [(pi, True) for pi in range(3)] + ([] if environment else [(pi, False) for pi in range(3)])
        def tick(delta):
            try:
                if state["warmup"] < 64:
                    pose = MANIFEST["poses"][0]
                    camera.set_actor_location(position(pose["eye"]), False, False)
                    camera.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(position(pose["eye"]), position(pose["target"])), False)
                    light_component.set_cast_shadows(True)
                    component.capture_scene()
                    state["warmup"] += 1
                    if state["warmup"] == 64:
                        unreal.SystemLibrary.execute_console_command(world, "D3D12.DumpRayTracingGeometries all")
                    return True
                if state["index"] >= len(schedule):
                    save("capture.json", dict(engine=unreal.SystemLibrary.get_engine_version(), shadow_mode=mode,
                        resolution=[1920, 1080], captures=records, engine_ticks=True, frames_per_pose=64, warmup_ticks=64,
                        cvars={key: unreal.SystemLibrary.get_console_variable_float_value(key) for key in cvars},
                        camera_fov=component.get_editor_property("fov_angle"), near_cm=.5,
                        direct_only=not environment, skylight_in_map_but_capture_show_flag_disabled=not environment,
                        timing_scope="capture includes readback/export and extra view; separate main-only profiling required"))
                    unreal.SystemLibrary.execute_console_command(world, "D3D12.DumpRayTracingGeometries all")
                    unreal.SystemLibrary.quit_game(world, None, unreal.QuitPreference.QUIT, True)
                    return False
                pi, shadows = schedule[state["index"]]
                pose = MANIFEST["poses"][pi]
                if state["frame"] == 0:
                    camera.set_actor_location(position(pose["eye"]), False, False)
                    camera.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(position(pose["eye"]), position(pose["target"])), False)
                    component.set_editor_property("camera_cut_this_frame", state["index"] in (0, 3))
                    light_component.set_cast_shadows(shadows)
                component.capture_scene()
                if (shadows and not environment and state["frame"] in (0, 1, 7, 31)) or state["frame"] == 63:
                    label = (("cut" if pi == 0 else "motion-first") if state["frame"] == 0 else
                             "steady" if state["frame"] == 63 else f"settle-{state['frame']}")
                    path = RUN / f"pose-{pi}-{label}-{'shadowed' if shadows else 'unshadowed'}.exr"
                    unreal.ImageWriteBlueprintLibrary.export_to_disk(target, str(path), options)
                    records.append(dict(pose=pi, state=label, engine_tick=64 + state["index"] * 64 + state["frame"],
                        casts_shadows=shadows, path=path.name, sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                        eye_cm=xyz(camera.get_actor_location()), forward=xyz(camera.get_actor_forward_vector()), up=xyz(camera.get_actor_up_vector())))
                state["frame"] += 1
                if state["frame"] == 64:
                    state["index"] += 1
                    state["frame"] = 0
                return True
            except Exception:
                import traceback
                save("failure.json", {"traceback": traceback.format_exc()})
                unreal.SystemLibrary.quit_game(world, None, unreal.QuitPreference.QUIT, True)
                return False
        unreal.register_ticker_callback(tick)
        return
    for pi, pose in enumerate(MANIFEST["poses"]):
        camera.set_actor_location(position(pose["eye"]), False, False)
        camera.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(position(pose["eye"]), position(pose["target"])), False)
        component.set_editor_property("camera_cut_this_frame", True)
        light_component.set_cast_shadows(True)
        for state, count in (("cut", 1), ("repeated-dispatch", 32)):
            for _ in range(count):
                component.capture_scene()
            path = RUN / f"pose-{pi}-{state}-shadowed.exr"
            unreal.ImageWriteBlueprintLibrary.export_to_disk(target, str(path), options)
            records.append(dict(pose=pi, state=state, casts_shadows=True, path=path.name, sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
        light_component.set_cast_shadows(False)
        component.capture_scene()
        path = RUN / f"pose-{pi}-unshadowed.exr"
        unreal.ImageWriteBlueprintLibrary.export_to_disk(target, str(path), options)
        records.append(dict(pose=pi, state="unshadowed", casts_shadows=False, path=path.name, sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
    save("capture.json", dict(engine=unreal.SystemLibrary.get_engine_version(), shadow_mode=mode, resolution=[1920, 1080], captures=records,
                               cvars={key: unreal.SystemLibrary.get_console_variable_float_value(key) for key in cvars},
                               camera_fov=component.get_editor_property("fov_angle"), near_cm=.5,
                               warning="commandlet repeated dispatches do not establish engine-frame temporal convergence; main-view steady-state still required",
                               timing_scope="no GPU/CPU frame time measured by this capture"))


def views():
    levels = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
    if not levels.load_level(LEVEL):
        raise RuntimeError("quarry map missing")
    filter_textures()
    actors = unreal.get_editor_subsystem(unreal.EditorActorSubsystem).get_all_level_actors()
    audit = []
    for actor in (a for a in actors if isinstance(a, unreal.StaticMeshActor)):
        mesh = actor.static_mesh_component.static_mesh
        index = int(mesh.get_name().split("_")[-1])
        expected = next(g for g in MANIFEST["geometry"] if g["placement"] == index)
        center, extent = actor.get_actor_bounds(False)
        actual = [xyz(center - extent), xyz(center + extent)]
        error = max(abs(a - b) for side_a, side_b in zip(actual, expected["expected_ue_bounds_cm"]) for a, b in zip(side_a, side_b))
        audit.append(dict(name=mesh.get_name(), triangles=mesh.get_num_triangles(0), vertices=mesh.get_num_vertices(0),
                          expected_triangles=expected["triangles"], bounds_cm=actual, maximum_bounds_error_cm=error))
        if error > .05:
            save("geometry-audit.json", audit)
            raise RuntimeError("importer axis/unit/bounds mismatch")
    save("geometry-audit.json", audit)
    # A plain GameModeBase would spawn a visible DefaultPawn sphere. Keep the
    # comparison scene free of gameplay geometry and physics-driven movement.
    mode_path = "/Game/M6Quarry/QuarryGameMode"
    blueprint = unreal.load_asset(mode_path)
    if blueprint is None:
        factory = unreal.BlueprintFactory()
        factory.set_editor_property("parent_class", unreal.GameModeBase)
        blueprint = unreal.AssetToolsHelpers.get_asset_tools().create_asset("QuarryGameMode", "/Game/M6Quarry", unreal.Blueprint, factory)
    unreal.BlueprintEditorLibrary.compile_blueprint(blueprint)
    game_class = unreal.BlueprintEditorLibrary.generated_class(blueprint)
    defaults = unreal.get_default_object(game_class)
    defaults.set_editor_property("default_pawn_class", None)
    defaults.set_editor_property("hud_class", None)
    unreal.EditorAssetLibrary.save_loaded_asset(blueprint)
    world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
    world.get_world_settings().set_editor_property("default_game_mode", game_class)
    camera = spawn(unreal.CameraActor, "GameCamera")
    camera.set_editor_property("auto_activate_for_player", unreal.AutoReceiveInput.PLAYER0)
    component = camera.get_component_by_class(unreal.CameraComponent)
    component.set_editor_property("field_of_view", math.degrees(2 * math.atan(math.tan(.85 / 2) * 1920 / 1080)))
    component.set_editor_property("aspect_ratio", 1920 / 1080)
    component.set_editor_property("constrain_aspect_ratio", False)
    post = unreal.PostProcessSettings()
    for key, value in (("auto_exposure_method", unreal.AutoExposureMethod.AEM_MANUAL),
                       ("auto_exposure_apply_physical_camera_exposure", False), ("auto_exposure_bias", -14.)):
        post.set_editor_property("override_" + key, True)
        post.set_editor_property(key, value)
    component.set_editor_property("post_process_settings", post)
    component.set_editor_property("post_process_blend_weight", 1.)
    records = []
    for pi, pose in enumerate(MANIFEST["poses"]):
        camera.set_actor_location(position(pose["eye"]), False, False)
        camera.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(position(pose["eye"]), position(pose["target"])), False)
        asset = f"/Game/M6Quarry/View{pi}"
        if not unreal.EditorLoadingAndSavingUtils.save_map(world, asset):
            raise RuntimeError("cannot save view map")
        records.append(dict(map=asset, eye_cm=xyz(camera.get_actor_location()), forward=xyz(camera.get_actor_forward_vector()),
                            up=xyz(camera.get_actor_up_vector()), fov=component.get_editor_property("field_of_view")))
    save("views.json", records)


def profile_motion():
    world = unreal.load_object(None, "/Game/M6Quarry/View0.View0")
    cameras = unreal.GameplayStatics.get_all_actors_of_class(world, unreal.CameraActor)
    if len(cameras) != 1:
        raise RuntimeError("expected the single registered main camera")
    camera = cameras[0]
    counter = [0]
    save("motion.json", dict(recipe="alternate exact registered poses 0 and 1 every engine tick, no cuts", poses=MANIFEST["poses"][:2]))
    def tick(delta):
        pose = MANIFEST["poses"][counter[0] % 2]
        camera.set_actor_location(position(pose["eye"]), False, False)
        camera.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(position(pose["eye"]), position(pose["target"])), False)
        counter[0] += 1
        return True
    unreal.register_ticker_callback(tick)


def filter_textures():
    records = []
    for path in unreal.EditorAssetLibrary.list_assets("/Game/M6Quarry/Imported/quarry/Textures", recursive=True):
        texture = unreal.load_asset(path)
        if not isinstance(texture, unreal.Texture2D):
            continue
        texture.set_editor_property("filter", unreal.TextureFilter.TF_DEFAULT)
        unreal.EditorAssetLibrary.save_loaded_asset(texture)
        records.append(dict(path=path, filter=str(texture.get_editor_property("filter"))))
    if len(records) != 12:
        raise RuntimeError("expected twelve base-color textures")
    save("filter.json", records)


def capture_actors():
    levels = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
    for pi in range(3):
        asset = f"/Game/M6Quarry/View{pi}"
        if not levels.load_level(asset):
            raise RuntimeError("missing registered view")
        actors = unreal.get_editor_subsystem(unreal.EditorActorSubsystem).get_all_level_actors()
        if not any(isinstance(a, unreal.SceneCapture2D) for a in actors):
            actor = spawn(unreal.SceneCapture2D, "Capture")
            component = actor.get_component_by_class(unreal.SceneCaptureComponent2D)
            component.set_editor_property("capture_every_frame", False)
            component.set_editor_property("capture_on_movement", False)
        if not levels.save_current_level():
            raise RuntimeError("cannot save capture actor")
    save("capture-actors.json", {"views": [0, 1, 2], "capture_every_frame": False})


def correct():
    """Own the glTF master, enable its Nanite permutations, and fix all saved maps."""
    library = unreal.MaterialEditingLibrary
    materials = [unreal.load_asset(f"/Game/M6Quarry/Imported/quarry/Materials/QuarryMaterial_{i:02d}") for i in range(12)]
    parents = {m.get_editor_property("parent").get_path_name() for m in materials}
    if len(parents) != 1:
        raise RuntimeError("material masters differ; inspect before reparenting")
    original = next(iter(parents))
    ancestor = unreal.load_asset(original)
    chain = []
    while isinstance(ancestor, unreal.MaterialInstanceConstant):
        chain.append(ancestor)
        ancestor = ancestor.get_editor_property("parent")
    if not isinstance(ancestor, unreal.Material):
        raise RuntimeError("cannot resolve glTF master ancestry")
    owned = "/Game/M6Quarry/Materials/M_QuarryNaniteMaster"
    master = unreal.load_asset(owned)
    if master is None:
        master = unreal.EditorAssetLibrary.duplicate_asset(ancestor.get_path_name(), owned)
    if not isinstance(master, unreal.Material):
        raise RuntimeError("expected a material master")
    library.set_material_usage(master, unreal.MaterialUsage.MATUSAGE_NANITE)
    library.recompile_material(master)
    if not library.has_material_usage(master, unreal.MaterialUsage.MATUSAGE_NANITE):
        raise RuntimeError("Nanite material usage did not stick")
    unreal.EditorAssetLibrary.save_loaded_asset(master)
    private_parent = master
    for index, source in enumerate(reversed(chain)):
        path = f"/Game/M6Quarry/Materials/MI_QuarryParent_{index}"
        instance = unreal.load_asset(path) or unreal.EditorAssetLibrary.duplicate_asset(source.get_path_name(), path)
        library.set_material_instance_parent(instance, private_parent)
        library.update_material_instance(instance)
        unreal.EditorAssetLibrary.save_loaded_asset(instance)
        private_parent = instance
    inventory = []
    for material in materials:
        before = material.get_editor_property("parent").get_path_name()
        library.set_material_instance_parent(material, private_parent)
        library.update_material_instance(material)
        unreal.EditorAssetLibrary.save_loaded_asset(material)
        inventory.append(dict(asset=material.get_path_name(), old_parent=before,
                              parent=material.get_editor_property("parent").get_path_name()))
    # Imported RGBA32F cube retains every source delta texel through an exact
    # basis permutation. Intensity 64 reverses its exact /64 storage scaling.
    cube_path = "/Game/M6Quarry/Environment/quarry-sky-v2"
    cube = unreal.load_asset(cube_path)
    if cube is None:
        task = unreal.AssetImportTask()
        for key, value in (("filename", str(ROOT / "environment-v2/quarry-sky-v2.dds")),
                           ("destination_path", "/Game/M6Quarry/Environment"), ("automated", True), ("save", True)):
            task.set_editor_property(key, value)
        unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([task])
        cube = unreal.load_asset(cube_path)
    if not isinstance(cube, unreal.TextureCube):
        raise RuntimeError("environment import did not produce TextureCube")
    for key, value in (("srgb", False), ("compression_settings", unreal.TextureCompressionSettings.TC_HDR_F32),
                       ("max_texture_size", 128), ("never_stream", True)):
        cube.set_editor_property(key, value)
    unreal.EditorAssetLibrary.save_loaded_asset(cube)
    levels = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
    maps = [LEVEL] + [f"/Game/M6Quarry/View{p}" for p in range(3)]
    map_records = []
    for path in maps:
        if not levels.load_level(path):
            raise RuntimeError("missing map " + path)
        actors = unreal.get_editor_subsystem(unreal.EditorActorSubsystem).get_all_level_actors()
        sky = next((a for a in actors if isinstance(a, unreal.SkyLight)), None) or spawn(unreal.SkyLight, "Environment")
        sky_component = sky.get_component_by_class(unreal.SkyLightComponent)
        sky_component.set_mobility(unreal.ComponentMobility.MOVABLE)
        sky_component.set_editor_property("source_type", unreal.SkyLightSourceType.SLS_SPECIFIED_CUBEMAP)
        sky_component.set_editor_property("cubemap", cube)
        sky_component.set_editor_property("cubemap_resolution", 128)
        sky_component.set_editor_property("lower_hemisphere_is_black", False)
        sky_component.set_editor_property("real_time_capture", False)
        sky_component.set_cast_shadows(False)
        sky_component.set_intensity(64.)
        sky_component.set_light_color(unreal.LinearColor(1, 1, 1, 1))
        sky_component.recapture_sky()
        volume = next((a for a in actors if isinstance(a, unreal.PostProcessVolume)), None) or spawn(unreal.PostProcessVolume, "Exposure")
        volume.set_editor_property("unbound", True)
        volume.set_editor_property("priority", 1000.)
        post = volume.get_editor_property("settings")
        for key, value in (("auto_exposure_method", unreal.AutoExposureMethod.AEM_MANUAL),
                           ("auto_exposure_apply_physical_camera_exposure", False), ("auto_exposure_bias", -14.)):
            post.set_editor_property("override_" + key, True)
            post.set_editor_property(key, value)
        volume.set_editor_property("settings", post)
        levels.save_current_level()
        map_records.append(dict(map=path, skylight_intensity=sky_component.get_editor_property("intensity"),
            cube=cube.get_path_name(), exposure_bias=post.get_editor_property("auto_exposure_bias")))
    save("correction.json", dict(engine=unreal.SystemLibrary.get_engine_version(), master=master.get_path_name(),
        nanite_usage=library.has_material_usage(master, unreal.MaterialUsage.MATUSAGE_NANITE), materials=inventory,
        maps=map_records, environment_transport=json.loads((ROOT / "environment-v2/transport.json").read_text())))


def full_residency():
    subsystem = mesh_subsystem()
    registry = unreal.AssetRegistryHelpers.get_asset_registry()
    records = []
    for index in range(12):
        path = f"/Game/M6Quarry/Imported/quarry/StaticMeshes/Quarry_{index:02d}"
        mesh = unreal.load_asset(path)
        settings = subsystem.get_nanite_settings(mesh)
        settings.set_editor_property("target_minimum_residency_in_kb", 4294967295)
        subsystem.set_nanite_settings(mesh, settings, True)
        unreal.EditorAssetLibrary.save_loaded_asset(mesh)
        asset = registry.get_asset_by_object_path(mesh.get_path_name())
        triangles = asset.get_tag_value("NaniteTriangles")
        records.append(dict(asset=mesh.get_path_name(), nanite_triangles=str(triangles),
                            minimum_residency_kb=settings.get_editor_property("target_minimum_residency_in_kb")))
        save("full-residency.json", records)


try:
    if not unreal.SystemLibrary.get_engine_version().startswith("5.7.4-51494982"):
        raise RuntimeError("UE build differs")
    if MODE in ("import", "assemble"):
        import_quarry()
    elif MODE == "capture":
        capture()
    elif MODE == "views":
        views()
    elif MODE in ("capture-game", "environment-game"):
        world = unreal.load_object(None, "/Game/M6Quarry/View0.View0")
        if world is None:
            raise RuntimeError("main game world unavailable")
        capture(world)
    elif MODE == "profile-motion":
        profile_motion()
    elif MODE == "filter":
        filter_textures()
    elif MODE == "capture-actors":
        capture_actors()
    elif MODE == "correct":
        correct()
    elif MODE == "full-residency":
        full_residency()
    else:
        raise RuntimeError("unknown run mode")
except Exception:
    import traceback
    save("failure.json", {"traceback": traceback.format_exc()})
    raise
