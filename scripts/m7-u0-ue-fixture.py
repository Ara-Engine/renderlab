"""M7-U0c Unreal editor script: D3D12 SM6 recapture of the M5 directional-black fixture.

Runs inside UnrealEditor-Cmd -run=pythonscript (launched by m7-u0-ue.py fixture). Loads the
already-prepared /Game/M5/Fixture level without saving anything, then captures scene HDR,
decoded normals and base colour with the retained M5 camera and show flags, once per sun source
angle. Outputs go to RENDERLAB_M7_FIXTURE_OUT only.
"""
import json
import math
import os
from pathlib import Path
import time

import unreal

OUT = Path(os.environ["RENDERLAB_M7_FIXTURE_OUT"])
SCENE = json.loads(Path("D:/Coding/renderlab-work/ue-m5/manifest.json").read_text(encoding="utf-8"))
ANGLES = (0.0, 0.5357, 4.583662361)  # degrees: delta light, UE default, the quarry's 0.04 rad radius disk


def position(metres):
    x, y, z = metres
    return unreal.Vector(-100 * z, 100 * x, 100 * y)


def xyz(v):
    return [v.x, v.y, v.z]


def main():
    record = dict(engine=unreal.SystemLibrary.get_engine_version(), rhi="requested -dx12 -sm6", captures=[])
    if not unreal.get_editor_subsystem(unreal.LevelEditorSubsystem).load_level("/Game/M5/Fixture"):
        raise RuntimeError("cannot load prepared fixture")
    actors = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
    world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
    lights = [a for a in actors.get_all_level_actors() if isinstance(a, unreal.DirectionalLight)]
    if len(lights) != 1:
        raise RuntimeError("expected one directional light")
    light = lights[0].get_component_by_class(unreal.DirectionalLightComponent)
    camera = SCENE["camera"]
    rotation = unreal.MathLibrary.find_look_at_rotation(position(camera["eye"]), position(camera["target"]))
    actor = actors.spawn_actor_from_class(unreal.SceneCapture2D, position(camera["eye"]), rotation)
    component = actor.get_component_by_class(unreal.SceneCaptureComponent2D)
    width, height = SCENE["resolution"]
    component.set_editor_property("capture_every_frame", False)
    component.set_editor_property("capture_on_movement", False)
    component.set_editor_property("fov_angle", math.degrees(2 * math.atan(math.tan(camera["fov_y_radians"] / 2) * width / height)))
    component.set_editor_property("override_custom_near_clipping_plane", True)
    component.set_editor_property("custom_near_clipping_plane", 100 * camera["near"])
    disabled = ["GlobalIllumination", "LumenGlobalIllumination", "LumenReflections", "ScreenSpaceReflections",
                "ReflectionEnvironment", "AmbientOcclusion", "DistanceFieldAO", "ContactShadows", "DynamicShadows",
                "SkyLighting", "Bloom", "Vignette", "Grain", "MotionBlur", "Fog", "VolumetricFog", "Atmosphere",
                "EyeAdaptation", "Tonemapper", "AntiAliasing"]
    component.set_editor_property("show_flag_settings", [unreal.EngineShowFlagsSetting(show_flag_name=n, enabled=False) for n in disabled])
    post = unreal.PostProcessSettings()
    for name, value in (("auto_exposure_method", unreal.AutoExposureMethod.AEM_MANUAL),
                        ("auto_exposure_apply_physical_camera_exposure", False), ("auto_exposure_bias", 0.0)):
        post.set_editor_property("override_" + name, True)
        post.set_editor_property(name, value)
    component.set_editor_property("post_process_settings", post)
    component.set_editor_property("post_process_blend_weight", 1.0)
    for name in ("r.AntiAliasingMethod", "r.ScreenPercentage", "r.Substrate", "r.GBufferFormat", "r.MinRoughnessOverride",
                 "r.Shading.EnergyConservation", "r.Shading.EnergyPreservation", "r.DynamicGlobalIlluminationMethod",
                 "r.ReflectionMethod"):
        unreal.SystemLibrary.execute_console_command(world, name)  # values land in ue.log with LastSetBy
    unreal.AutomationLibrary.finish_loading_before_screenshot()
    options = unreal.ImageWriteOptions(format=unreal.DesiredImageFormat.EXR, compression_quality=1, overwrite_file=True, async_=False)
    targets = {}
    for source, fmt, name in ((unreal.SceneCaptureSource.SCS_SCENE_COLOR_HDR, unreal.TextureRenderTargetFormat.RTF_RGBA16F, "scene-linear"),
                              (unreal.SceneCaptureSource.SCS_NORMAL, unreal.TextureRenderTargetFormat.RTF_RGBA32F, "normal"),
                              (unreal.SceneCaptureSource.SCS_BASE_COLOR, unreal.TextureRenderTargetFormat.RTF_RGBA32F, "basecolor")):
        targets[name] = (source, unreal.RenderingLibrary.create_render_target2d(world, width, height, fmt))
    for angle in ANGLES:
        light.set_editor_property("light_source_angle", angle)
        light.set_editor_property("light_source_soft_angle", 0.0)
        for name, (source, rt) in targets.items():
            if name != "scene-linear" and angle != ANGLES[0]:
                continue
            component.set_editor_property("texture_target", rt)
            component.set_editor_property("capture_source", source)
            path = OUT / f"{name}-angle-{angle:.4f}.exr"
            start = time.perf_counter()
            component.capture_scene()
            unreal.ImageWriteBlueprintLibrary.export_to_disk(rt, str(path), options)
            record["captures"].append(dict(name=name, angle_degrees=angle, path=str(path),
                                           readback_light_source_angle=light.get_editor_property("light_source_angle"),
                                           seconds=time.perf_counter() - start))
    record.update(camera_ue_cm=xyz(actor.get_actor_location()), camera_forward=xyz(actor.get_actor_forward_vector()),
                  camera_right=xyz(actor.get_actor_right_vector()), camera_up=xyz(actor.get_actor_up_vector()),
                  horizontal_fov_deg=component.get_editor_property("fov_angle"),
                  light_forward_ue=xyz(lights[0].get_actor_forward_vector()),
                  light_rgb=[light.get_light_color().r * light.get_editor_property("intensity"),
                             light.get_light_color().g * light.get_editor_property("intensity"),
                             light.get_light_color().b * light.get_editor_property("intensity")],
                  feature_level=str(unreal.SystemLibrary.get_rendering_detail_mode()) if hasattr(unreal.SystemLibrary, "get_rendering_detail_mode") else None)
    (OUT / "capture.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")


main()
