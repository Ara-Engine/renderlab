"""UE-only public scripting calls, copied into the private generated project.

No UE implementation source is embedded here. Never import this in CPU checks.
"""

import hashlib
import json
import math
from pathlib import Path
import time

import unreal


ROOT = Path(__file__).resolve().parents[2]
SCENE = json.loads((ROOT / "scene.json").read_text(encoding="utf-8"))
HASH = hashlib.sha256(json.dumps(SCENE, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()
LEVEL = "/Game/M5/Calibration"


def write_record(name, value):
    (ROOT / "Artifacts" / name).write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def position(metres):
    x, y, z = metres
    return unreal.Vector(-100*z, 100*x, 100*y)


def spawn(actor_class, name, location, rotation=None):
    actor = unreal.get_editor_subsystem(unreal.EditorActorSubsystem).spawn_actor_from_class(
        actor_class, location, rotation or unreal.Rotator())
    if actor is None:
        raise RuntimeError(f"cannot spawn {name}")
    actor.set_actor_label("M5_" + name)
    return actor


def make_material():
    path = "/Game/M5/M_Lambert"
    material = unreal.load_asset(path)
    if material is None:
        material = unreal.AssetToolsHelpers.get_asset_tools().create_asset(
            "M_Lambert", "/Game/M5", unreal.Material, unreal.MaterialFactoryNew())
    edit = unreal.MaterialEditingLibrary
    edit.delete_all_material_expressions(material)
    colour = edit.create_material_expression(material, unreal.MaterialExpressionConstant3Vector)
    colour.set_editor_property("constant", unreal.LinearColor(*SCENE["diffuse_linear_rgb"], 1))
    if not edit.connect_material_property(colour, "", unreal.MaterialProperty.MP_BASE_COLOR):
        raise RuntimeError("cannot connect base colour")
    for prop, value in ((unreal.MaterialProperty.MP_SPECULAR, SCENE["specular"]),
                        (unreal.MaterialProperty.MP_METALLIC, SCENE["metallic"]),
                        (unreal.MaterialProperty.MP_ROUGHNESS, SCENE["roughness"])):
        node = edit.create_material_expression(material, unreal.MaterialExpressionConstant)
        node.set_editor_property("r", value)
        if not edit.connect_material_property(node, "", prop):
            raise RuntimeError("cannot connect scalar material property")
    edit.recompile_material(material)
    if not unreal.EditorAssetLibrary.save_loaded_asset(material):
        raise RuntimeError("cannot save material")
    return material


def prepare():
    (ROOT / "Artifacts/prepared.json").unlink(missing_ok=True)
    levels = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
    actors = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
    if unreal.EditorAssetLibrary.does_asset_exist(LEVEL):
        if not levels.load_level(LEVEL):
            raise RuntimeError("cannot load calibration level")
        # Only this generated namespace is owned; refuse to destroy unknown authored content.
        existing = actors.get_all_level_actors()
        if any(not actor.get_actor_label().startswith("M5_")
               and not isinstance(actor, (unreal.WorldSettings, unreal.Brush)) for actor in existing):
            raise RuntimeError("foreign actor in generated level; use a separate project")
        for actor in existing:
            if actor.get_actor_label().startswith("M5_"):
                if not actors.destroy_actor(actor):
                    raise RuntimeError("cannot replace generated actor")
    elif not levels.new_level(LEVEL):
        raise RuntimeError("cannot create calibration level")
    material = make_material()
    bounds = []
    for primitive in SCENE["primitives"]:
        actor = spawn(unreal.StaticMeshActor, primitive["name"], position(primitive["center_m"]))
        mesh = actor.static_mesh_component
        mesh.set_mobility(unreal.ComponentMobility.MOVABLE)
        asset_name = "Plane" if primitive["kind"] == "plane" else "Cube"
        if not mesh.set_static_mesh(unreal.load_asset("/Engine/BasicShapes/" + asset_name)):
            raise RuntimeError("cannot assign built-in mesh")
        x, y, z = primitive["size_m"]
        # Built-in Plane and Cube are 100 cm across. Plane's native normal is UE +Z.
        actor.set_actor_scale3d(unreal.Vector(z, x, y if primitive["kind"] == "box" else 1))
        mesh.set_material(0, material)
        mesh.set_cast_shadow(False)
        origin, extent = actor.get_actor_bounds(False)
        bounds.append({"name": primitive["name"], "center_cm": [origin.x, origin.y, origin.z],
                       "half_extent_cm": [extent.x, extent.y, extent.z]})
    rotation = unreal.MathLibrary.find_look_at_rotation(unreal.Vector(), position(SCENE["directional"]["travel"]))
    light = spawn(unreal.DirectionalLight, "directional", unreal.Vector(0, 0, 500), rotation)
    component = light.get_component_by_class(unreal.DirectionalLightComponent)
    component.set_mobility(unreal.ComponentMobility.MOVABLE)
    component.set_intensity(SCENE["directional"]["lux"])
    component.set_light_color(unreal.LinearColor(1, 1, 1, 1))
    component.set_cast_shadows(False)
    component.set_editor_property("contact_shadow_length", 0)
    component.set_editor_property("light_source_angle", 0)
    component.set_editor_property("light_source_soft_angle", 0)
    if not levels.save_current_level():
        raise RuntimeError("cannot save calibration level")
    write_record("prepared.json", {"scene_sha256": HASH, "bounds": bounds,
                                  "status": "assets saved only; no image verification"})


def capture():
    (ROOT / "Artifacts/capture.json").unlink(missing_ok=True)
    prepared = json.loads((ROOT / "Artifacts/prepared.json").read_text(encoding="utf-8"))
    if prepared["scene_sha256"] != HASH:
        raise RuntimeError("scene changed: rerun prepare-assets before capturing")
    if not unreal.get_editor_subsystem(unreal.LevelEditorSubsystem).load_level(LEVEL):
        raise RuntimeError("cannot load prepared level")
    world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
    camera = SCENE["camera"]
    rotation = unreal.MathLibrary.find_look_at_rotation(position(camera["eye_m"]), position(camera["target_m"]))
    actor = spawn(unreal.SceneCapture2D, "capture", position(camera["eye_m"]), rotation)
    capture_component = actor.get_component_by_class(unreal.SceneCaptureComponent2D)
    width, height = SCENE["resolution"]
    rt = unreal.RenderingLibrary.create_render_target2d(world, width, height,
                                                       unreal.TextureRenderTargetFormat.RTF_RGBA16F)
    capture_component.set_editor_property("texture_target", rt)
    capture_component.set_editor_property("capture_source", unreal.SceneCaptureSource.SCS_SCENE_COLOR_HDR)
    capture_component.set_editor_property("capture_every_frame", False)
    capture_component.set_editor_property("capture_on_movement", False)
    capture_component.set_editor_property("render_in_main_renderer", False)
    horizontal_fov = math.degrees(2 * math.atan(math.tan(math.radians(camera["vertical_fov_deg"])/2)*width/height))
    capture_component.set_editor_property("fov_angle", horizontal_fov)
    disabled = ["GlobalIllumination", "LumenGlobalIllumination", "LumenReflections", "ScreenSpaceReflections", "ReflectionEnvironment",
                "AmbientOcclusion", "DistanceFieldAO", "ContactShadows", "DynamicShadows", "SkyLighting",
                "Bloom", "Vignette", "Grain", "MotionBlur", "Fog", "VolumetricFog", "Atmosphere",
                "EyeAdaptation", "Tonemapper", "AntiAliasing"]
    capture_component.set_show_flag_settings([
        unreal.EngineShowFlagsSetting(show_flag_name=name, enabled=False) for name in disabled])
    post = unreal.PostProcessSettings()
    for name, value in (("auto_exposure_method", unreal.AutoExposureMethod.AEM_MANUAL),
                        ("auto_exposure_apply_physical_camera_exposure", False), ("auto_exposure_bias", 0.0)):
        post.set_editor_property("override_" + name, True)
        post.set_editor_property(name, value)
    capture_component.set_editor_property("post_process_settings", post)
    capture_component.set_editor_property("post_process_blend_weight", 1.0)
    cvars = ["r.ForwardShading", "r.AntiAliasingMethod", "r.MSAACount", "r.ScreenPercentage", "r.UsePreExposure",
             "r.DynamicGlobalIlluminationMethod", "r.ReflectionMethod", "r.SSR.Quality", "r.AmbientOcclusionLevels",
             "r.DistanceFieldAO", "r.ContactShadows", "r.ShadowQuality", "r.Shadow.Virtual.Enable",
             "r.BloomQuality", "r.EyeAdaptationQuality", "r.MotionBlurQuality", "r.VolumetricFog", "r.SkyAtmosphere",
             "r.AllowStaticLighting", "r.Substrate"]
    effective = {}
    for name in cvars:
        # The log query also preserves existence/help/priority; numeric zero alone cannot prove existence.
        unreal.SystemLibrary.execute_console_command(world, name)
        effective[name] = unreal.SystemLibrary.get_console_variable_float_value(name)
    if effective["r.AntiAliasingMethod"] != 0 or effective["r.UsePreExposure"] != 0:
        raise RuntimeError("effective AA/pre-exposure settings disagree with calibration")
    loading_start = time.perf_counter()
    unreal.AutomationLibrary.finish_loading_before_screenshot()
    loading_ms = 1000 * (time.perf_counter() - loading_start)
    output = ROOT / "Artifacts/scene-linear.exr"
    # Remove a stale output so export failure cannot be mistaken for a successful new capture.
    output.unlink(missing_ok=True)
    options = unreal.ImageWriteOptions(format=unreal.DesiredImageFormat.EXR, compression_quality=1,
                                      overwrite_file=True, async_=False)
    start = time.perf_counter()
    capture_component.capture_scene()
    unreal.ImageWriteBlueprintLibrary.export_to_disk(rt, str(output), options)
    elapsed = 1000 * (time.perf_counter() - start)
    if not output.is_file() or output.stat().st_size < 8 or output.read_bytes()[:4] != b"\x76\x2f\x31\x01":
        raise RuntimeError("EXR export failed")
    write_record("capture.json", {
        "scene_sha256": HASH, "engine_version": unreal.SystemLibrary.get_engine_version(),
        "resolution": [width, height], "samples_requested": 1, "cvars": effective,
        "show_flags_disabled": disabled, "horizontal_fov_deg": horizontal_fov,
        "camera_ue_cm": list((-100*camera["eye_m"][2], 100*camera["eye_m"][0], 100*camera["eye_m"][1])),
        "capture_source": "SCS_SCENE_COLOR_HDR RGB (alpha is inverse opacity, not radiance)",
        "target": "RGBA16F, linear scene colour; no UE display curve; r.UsePreExposure=0",
        "manual_exposure": {"physical_camera": False, "bias": 0, "offline": SCENE["offline_display"]},
        "loading_and_shader_completion_ms": loading_ms,
        "capture_and_synchronous_exr_export_ms": elapsed,
        "timing_scope": "CPU wall span including submission, GPU completion/readback and EXR encoding; NOT GPU frame time or steady-state throughput",
        "exr_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
        "verification": "capture exists only; scale/projection, radiance, finite/clipping and full-image checks still required",
    })


if MODE == "prepare-assets":
    prepare()
elif MODE == "capture":
    capture()
else:
    raise ValueError("unknown generated command mode")
