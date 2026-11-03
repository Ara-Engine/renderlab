"""UE-only public scripting calls for retained M5-A; never imported by CPU tests."""
import hashlib
import json
import math
from pathlib import Path
import time
from collections import Counter
import unreal

ROOT = Path(__file__).resolve().parents[2]
MANIFEST_BYTES = (ROOT / "manifest.json").read_bytes()
HASH = hashlib.sha256(MANIFEST_BYTES).hexdigest()
SCENE = json.loads(MANIFEST_BYTES)
EXPECTED = json.loads((ROOT / "expected.json").read_text(encoding="utf-8"))
LEVEL = "/Game/M5/Fixture"


def write_record(name, value):
    (ROOT / "Artifacts" / name).write_text(json.dumps(value, indent=2, allow_nan=False)+"\n", encoding="utf-8")


def check_inputs():
    if HASH != EXPECTED["manifest_sha256"]:
        raise RuntimeError("manifest changed; regenerate and prepare again")
    for file, digest in (("scene.json", SCENE["source_scene_sha256"]),
                         ("renderlab-lighting.json", EXPECTED["renderlab_lighting_sha256"]),
                         ("Content/Python/ue_m5_editor.py", EXPECTED["editor_script_sha256"])):
        if hashlib.sha256((ROOT / file).read_bytes()).hexdigest() != digest:
            raise RuntimeError("input/script hash changed: " + file)
    for obj in SCENE["objects"]:
        if hashlib.sha256((ROOT / obj["mesh"]).read_bytes()).hexdigest() != obj["mesh_file_sha256"]:
            raise RuntimeError("mesh bytes changed: " + obj["mesh"])
    version = unreal.SystemLibrary.get_engine_version()
    if not version.startswith("5.7.4-51494982"):
        raise RuntimeError("unexpected installed UE build: " + version)


def vector(value):
    return unreal.Vector(*value)


def position(metres):
    x, y, z = metres
    return unreal.Vector(-100*z, 100*x, 100*y)


def xyz(value):
    return [value.x, value.y, value.z]


def spawn(actor_class, name, location, rotation=None):
    actor = unreal.get_editor_subsystem(unreal.EditorActorSubsystem).spawn_actor_from_class(
        actor_class, location, rotation or unreal.Rotator())
    if actor is None:
        raise RuntimeError("cannot spawn " + name)
    actor.set_actor_label("M5_" + name)
    return actor


def make_material(index, spec):
    name = f"M_Fixture_{index:02d}"
    material = unreal.load_asset("/Game/M5/" + name)
    if material is None:
        material = unreal.AssetToolsHelpers.get_asset_tools().create_asset(
            name, "/Game/M5", unreal.Material, unreal.MaterialFactoryNew())
    material.set_editor_property("shading_model", unreal.MaterialShadingModel.MSM_DEFAULT_LIT)
    material.set_editor_property("two_sided", spec["two_sided"])
    edit = unreal.MaterialEditingLibrary
    edit.delete_all_material_expressions(material)
    colour = edit.create_material_expression(material, unreal.MaterialExpressionConstant3Vector)
    colour.set_editor_property("constant", unreal.LinearColor(*spec["base_color_linear"]))
    actual_colour = colour.get_editor_property("constant")
    constants = {"base_color_linear": [actual_colour.r, actual_colour.g, actual_colour.b, actual_colour.a]}
    if not edit.connect_material_property(colour, "", unreal.MaterialProperty.MP_BASE_COLOR):
        raise RuntimeError("cannot connect base colour")
    # UE's documented dielectric F0 = 0.08 * Specular: 0.5 gives source F0=.04.
    for prop, value in ((unreal.MaterialProperty.MP_SPECULAR, spec["specular"]),
                        (unreal.MaterialProperty.MP_METALLIC, spec["metallic"]),
                        (unreal.MaterialProperty.MP_ROUGHNESS, spec["roughness"])):
        node = edit.create_material_expression(material, unreal.MaterialExpressionConstant)
        node.set_editor_property("r", value)
        constants[str(prop)] = node.get_editor_property("r")
        if not edit.connect_material_property(node, "", prop):
            raise RuntimeError("cannot connect material scalar")
    edit.recompile_material(material)
    if not unreal.EditorAssetLibrary.save_loaded_asset(material):
        raise RuntimeError("cannot save material")
    properties = {key: str(material.get_editor_property(key)) for key in
                  ("shading_model", "two_sided", "tangent_space_normal", "blend_mode")}
    return material, {"properties": properties, "actual_graph_constants": constants}


def validate_dynamic(mesh, expected):
    # Check original vertex/triangle IDs before any static-mesh build/reordering.
    if mesh.get_vertex_count() != len(expected["positions"]) or mesh.get_triangle_count()*3 != len(expected["indices"]):
        raise RuntimeError("UE dynamic mesh changed fixture counts")
    max_error = 0.0
    for i, p in enumerate(expected["positions"]):
        actual, valid = mesh.get_vertex_position(i)
        if not valid:
            raise RuntimeError("missing mesh vertex")
        max_error = max(max_error, *(abs(a-b) for a, b in zip(xyz(actual), p)))
    for i in range(mesh.get_triangle_count()):
        actual, valid = mesh.get_triangle_indices(i)
        wanted = expected["indices"][3*i:3*i+3]
        if not valid or xyz(actual) != wanted:
            raise RuntimeError("UE changed authored topology/winding")
        _, a, b, c, valid = mesh.get_triangle_normals(i)
        if not valid:
            raise RuntimeError("UE lost authored normals")
        for n, vertex_id in zip((a, b, c), wanted):
            max_error = max(max_error, *(abs(v-w) for v, w in zip(xyz(n), expected["normals"][vertex_id])))
    if max_error > 1e-6:
        raise RuntimeError(f"UE buffer conversion error {max_error}")
    return {"vertices": mesh.get_vertex_count(), "triangles": mesh.get_triangle_count(),
            "max_position_cm_or_normal_error": max_error, "indices": "exact original ID/order"}


def make_mesh(index, obj, material):
    data = json.loads((ROOT / obj["mesh"]).read_text(encoding="utf-8"))
    buffers = unreal.GeometryScriptSimpleMeshBuffers(
        vertices=[vector(v) for v in data["positions"]],
        normals=[vector(n) for n in data["normals"]],
        uv0=[unreal.Vector2D(0, 0) for _ in data["positions"]],
        vertex_colors=[unreal.LinearColor(1, 1, 1, 1) for _ in data["positions"]],
        triangles=[unreal.IntVector(*data["indices"][i:i+3]) for i in range(0, len(data["indices"]), 3)])
    dynamic = unreal.DynamicMesh()
    dynamic.append_buffers_to_mesh(buffers)
    imported = validate_dynamic(dynamic, data)
    path = f"/Game/M5/SM_Fixture_{index:02d}"
    mesh = unreal.load_asset(path)
    if mesh is None:
        mesh, outcome = unreal.GeometryScript_NewAssetUtils.create_new_static_mesh_asset_from_mesh(
            dynamic, path, unreal.GeometryScriptCreateNewStaticMeshAssetOptions(
                enable_recompute_normals=False, enable_recompute_tangents=False,
                enable_nanite=False, enable_collision=False, use_original_vertex_order=True))
        if outcome != unreal.GeometryScriptOutcomePins.SUCCESS or mesh is None:
            raise RuntimeError("cannot create exact fixture static mesh")
    _, outcome = unreal.GeometryScript_AssetUtils.copy_mesh_to_static_mesh(
        dynamic, mesh, unreal.GeometryScriptCopyMeshToAssetOptions(
            enable_recompute_normals=False, enable_recompute_tangents=False, enable_remove_degenerates=False,
            use_original_vertex_order=True, use_build_scale=False, replace_materials=True, new_materials=[material]),
        unreal.GeometryScriptMeshWriteLOD(lod_index=0))
    if outcome != unreal.GeometryScriptOutcomePins.SUCCESS:
        raise RuntimeError("cannot write exact mesh")
    subsystem = unreal.get_editor_subsystem(unreal.StaticMeshEditorSubsystem)
    # This module loads after commandlet subsystem initialization. These public
    # mesh-setting methods are stateless; its class default object also works.
    if subsystem is None:
        subsystem = unreal.get_default_object(unreal.StaticMeshEditorSubsystem)
    settings = subsystem.get_lod_build_settings(mesh, 0)
    for key, value in (("recompute_normals", False), ("recompute_tangents", False),
                       ("remove_degenerates", False), ("generate_lightmap_u_vs", False),
                       ("use_high_precision_tangent_basis", True), ("use_full_precision_u_vs", True)):
        settings.set_editor_property(key, value)
    subsystem.set_lod_build_settings(mesh, 0, settings)
    if not unreal.EditorAssetLibrary.save_loaded_asset(mesh):
        raise RuntimeError("cannot save fixture mesh")
    # Static build may reorder/split vertices; retain counts and explicitly require render-data inspection.
    source_copy = unreal.DynamicMesh()
    _, outcome = unreal.GeometryScript_AssetUtils.copy_mesh_from_static_mesh(
        mesh, source_copy, unreal.GeometryScriptCopyMeshFromAssetOptions(
            apply_build_settings=False, request_tangents=False, use_build_scale=False),
        unreal.GeometryScriptMeshReadLOD(lod_type=unreal.GeometryScriptLODType.SOURCE_MODEL, lod_index=0))
    if outcome != unreal.GeometryScriptOutcomePins.SUCCESS or source_copy.get_triangle_count() != obj["source"]["triangles"]:
        raise RuntimeError("static source model changed triangle count")
    # Compare oriented corner data independently of source-model vertex splitting/reindexing.
    def corner_key(p, n):
        return tuple(round(v, 5) for v in p) + tuple(round(v, 6) for v in n)
    def oriented_key(corners):
        return min(tuple(corners[i:]+corners[:i]) for i in range(3))
    expected_corners = Counter(oriented_key([
        corner_key(data["positions"][v], data["normals"][v]) for v in data["indices"][i:i+3]])
        for i in range(0, len(data["indices"]), 3))
    actual_corners = Counter()
    for i in range(source_copy.get_triangle_count()):
        valid, p1, p2, p3 = source_copy.get_triangle_positions(i)
        _, n1, n2, n3, normals_valid = source_copy.get_triangle_normals(i)
        if not valid or not normals_valid:
            raise RuntimeError("static source lost positions/normals")
        actual_corners[oriented_key([corner_key(xyz(p), xyz(n)) for p, n in zip((p1, p2, p3), (n1, n2, n3))])] += 1
    if actual_corners != expected_corners:
        raise RuntimeError("static source changed authored oriented triangle positions/normals")
    return mesh, {"input": imported, "source_model_vertices": source_copy.get_vertex_count(),
                  "source_model_triangles": source_copy.get_triangle_count(),
                  "source_model_corners": "oriented triangle multiset equal at 1e-5 cm / 1e-6 normal rounding",
                  "normal_recompute": False, "degenerate_removal": False, "high_precision_tangent_basis": True,
                  "render_buffer_precision_and_order": "UNVERIFIED; static normal packing/reordering is not source-byte identity"}


def make_light(index, spec):
    kind = spec["source"]["type"]
    rotation = (unreal.MathLibrary.find_look_at_rotation(unreal.Vector(), vector(spec["travel_ue"]))
                if "travel_ue" in spec else unreal.Rotator())
    actor = spawn({"directional": unreal.DirectionalLight, "point": unreal.PointLight, "spot": unreal.SpotLight}[kind],
                  "light_"+str(index), vector(spec.get("position_cm", [0, 0, 0])), rotation)
    component = actor.get_component_by_class(unreal.LightComponent)
    component.set_mobility(unreal.ComponentMobility.MOVABLE)
    component.set_cast_shadows(False)
    component.set_editor_property("contact_shadow_length", 0)
    component.set_editor_property("use_temperature", False)
    component.set_indirect_lighting_intensity(0)
    if kind == "directional":
        component.set_editor_property("light_source_angle", 0)
        component.set_editor_property("light_source_soft_angle", 0)
    else:
        component.set_editor_property("intensity_units", unreal.LightUnits.CANDELAS)
        component.set_editor_property("use_inverse_squared_falloff", True)
        component.set_editor_property("attenuation_radius", spec["attenuation_radius_cm"])
        component.set_editor_property("source_radius", 0)
        component.set_editor_property("soft_source_radius", 0)
        component.set_editor_property("source_length", 0)
        if kind == "spot":
            component.set_editor_property("inner_cone_angle", spec["inner_cone_half_angle_degrees"])
            component.set_editor_property("outer_cone_angle", spec["outer_cone_half_angle_degrees"])
    component.set_intensity(spec["intensity"])
    component.set_light_color(unreal.LinearColor(*spec["linear_color"], 1), srgb=True)
    actual = component.get_light_color()
    intensity = component.get_editor_property("intensity")
    rgb = [actual.r, actual.g, actual.b]
    properties = ["intensity", "light_color", "cast_shadows", "use_temperature"]
    properties += (["light_source_angle", "light_source_soft_angle"] if kind == "directional" else
                   ["intensity_units", "use_inverse_squared_falloff", "attenuation_radius",
                    "source_radius", "soft_source_radius", "source_length"])
    if kind == "spot":
        properties += ["inner_cone_angle", "outer_cone_angle"]
    return {"source": spec["source"], "actual_linear_rgb": rgb, "actual_intensity": intensity,
            "reconstructed_rgb_lux_or_cd": [v*intensity for v in rgb],
            "properties": {key: str(component.get_editor_property(key)) for key in properties},
            "actor_location_cm": xyz(actor.get_actor_location()), "actor_forward": xyz(actor.get_actor_forward_vector())}


def prepare():
    (ROOT / "Artifacts/prepared.json").unlink(missing_ok=True)
    levels = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
    actors = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
    if unreal.EditorAssetLibrary.does_asset_exist(LEVEL):
        if not levels.load_level(LEVEL):
            raise RuntimeError("cannot load generated fixture level")
        existing = actors.get_all_level_actors()
        if any(not a.get_actor_label().startswith("M5_") and not isinstance(a, (unreal.WorldSettings, unreal.Brush)) for a in existing):
            raise RuntimeError("foreign actors in generated level; refusing replacement")
        for actor in existing:
            if actor.get_actor_label().startswith("M5_") and not actors.destroy_actor(actor):
                raise RuntimeError("cannot replace generated actor")
    elif not levels.new_level(LEVEL):
        raise RuntimeError("cannot create fixture level")
    records = []
    for i, obj in enumerate(SCENE["objects"]):
        material, material_record = make_material(i, obj["material"])
        mesh, mesh_record = make_mesh(i, obj, material)
        actor = spawn(unreal.StaticMeshActor, obj["source"]["name"], unreal.Vector())
        component = actor.static_mesh_component
        component.set_mobility(unreal.ComponentMobility.MOVABLE)
        if not component.set_static_mesh(mesh):
            raise RuntimeError("cannot assign generated fixture mesh")
        component.set_material(0, material)
        component.set_cast_shadow(False)
        origin, extent = actor.get_actor_bounds(False)
        records.append({"name": obj["source"]["name"], "bounds_center_cm": xyz(origin),
                        "bounds_half_extent_cm": xyz(extent), "material": material_record, "mesh": mesh_record})
    lights = [make_light(i, spec) for i, spec in enumerate(SCENE["lights"])]
    if not levels.save_current_level():
        raise RuntimeError("cannot save fixture level")
    write_record("prepared.json", {"manifest_sha256": HASH, "objects": records, "lights": lights,
                                  "status": "assets saved only; image/energy/settings verification still UNRUN"})


def capture():
    if SCENE["variant"] != "directional-black":
        raise RuntimeError("full fixture environment is PENDING; refusing silently black/fake-filled capture")
    (ROOT / "Artifacts/capture.json").unlink(missing_ok=True)
    prepared = json.loads((ROOT / "Artifacts/prepared.json").read_text(encoding="utf-8"))
    if prepared["manifest_sha256"] != HASH:
        raise RuntimeError("inputs changed: rerun prepare-assets before capture")
    if not unreal.get_editor_subsystem(unreal.LevelEditorSubsystem).load_level(LEVEL):
        raise RuntimeError("cannot load prepared fixture")
    world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
    camera = SCENE["camera"]
    rotation = unreal.MathLibrary.find_look_at_rotation(position(camera["eye"]), position(camera["target"]))
    actor = spawn(unreal.SceneCapture2D, "capture", position(camera["eye"]), rotation)
    component = actor.get_component_by_class(unreal.SceneCaptureComponent2D)
    width, height = SCENE["resolution"]
    rt = unreal.RenderingLibrary.create_render_target2d(world, width, height, unreal.TextureRenderTargetFormat.RTF_RGBA16F)
    component.set_editor_property("texture_target", rt)
    component.set_editor_property("capture_source", unreal.SceneCaptureSource.SCS_SCENE_COLOR_HDR)
    component.set_editor_property("capture_every_frame", False)
    component.set_editor_property("capture_on_movement", False)
    component.set_editor_property("render_in_main_renderer", False)
    horizontal_fov = math.degrees(2*math.atan(math.tan(camera["fov_y_radians"]/2)*width/height))
    component.set_editor_property("fov_angle", horizontal_fov)
    component.set_editor_property("override_custom_near_clipping_plane", True)
    component.set_editor_property("custom_near_clipping_plane", 100*camera["near"])
    disabled = ["GlobalIllumination", "LumenGlobalIllumination", "LumenReflections", "ScreenSpaceReflections",
                "ReflectionEnvironment", "AmbientOcclusion", "DistanceFieldAO", "ContactShadows", "DynamicShadows",
                "SkyLighting", "Bloom", "Vignette", "Grain", "MotionBlur", "Fog", "VolumetricFog", "Atmosphere",
                "EyeAdaptation", "Tonemapper", "AntiAliasing"]
    component.set_editor_property("show_flag_settings", [
        unreal.EngineShowFlagsSetting(show_flag_name=n, enabled=False) for n in disabled])
    post = unreal.PostProcessSettings()
    for name, value in (("auto_exposure_method", unreal.AutoExposureMethod.AEM_MANUAL),
                        ("auto_exposure_apply_physical_camera_exposure", False), ("auto_exposure_bias", 0.0)):
        post.set_editor_property("override_"+name, True)
        post.set_editor_property(name, value)
    component.set_editor_property("post_process_settings", post)
    component.set_editor_property("post_process_blend_weight", 1.0)
    required = ["r.ForwardShading", "r.AntiAliasingMethod", "r.MSAACount", "r.ScreenPercentage", "r.UsePreExposure",
                "r.DynamicGlobalIlluminationMethod", "r.ReflectionMethod", "r.SSR.Quality", "r.AmbientOcclusionLevels",
                "r.DistanceFieldAO", "r.ContactShadows", "r.ShadowQuality", "r.Shadow.Virtual.Enable", "r.BloomQuality",
                "r.EyeAdaptationQuality", "r.MotionBlurQuality", "r.VolumetricFog", "r.SkyAtmosphere",
                "r.AllowStaticLighting", "r.Substrate"]
    # Discovery queries are intentionally not assumed to exist. Console log must show values/help,
    # or the energy-model part of the comparison remains unresolved.
    energy_queries = ["r.Shading.EnergyConservation", "r.Shading.EnergyPreservation", "r.Material.EnergyConservation"]
    effective = {}
    for name in required+energy_queries:
        unreal.SystemLibrary.execute_console_command(world, name)
        effective[name] = {"numeric_query": unreal.SystemLibrary.get_console_variable_float_value(name),
                           "existence": "check console query log; a numeric zero does NOT prove existence"}
    if any(effective[n]["numeric_query"] != 0 for n in ("r.AntiAliasingMethod", "r.UsePreExposure", "r.Substrate")):
        raise RuntimeError("effective AA/pre-exposure/Substrate disagrees with explicit legacy 1x setup")
    loading_start = time.perf_counter()
    unreal.AutomationLibrary.finish_loading_before_screenshot()
    loading_ms = 1000*(time.perf_counter()-loading_start)
    output = ROOT / "Artifacts/scene-linear.exr"
    output.unlink(missing_ok=True)
    options = unreal.ImageWriteOptions(format=unreal.DesiredImageFormat.EXR, compression_quality=1,
                                      overwrite_file=True, async_=False)
    start = time.perf_counter()
    component.capture_scene()
    unreal.ImageWriteBlueprintLibrary.export_to_disk(rt, str(output), options)
    elapsed = 1000*(time.perf_counter()-start)
    if not output.is_file() or output.stat().st_size < 8 or output.read_bytes()[:4] != b"\x76\x2f\x31\x01":
        raise RuntimeError("EXR export failed")
    write_record("capture.json", {
        "manifest_sha256": HASH, "source_scene_sha256": SCENE["source_scene_sha256"], "variant": SCENE["variant"],
        "engine_version": unreal.SystemLibrary.get_engine_version(), "resolution": [width, height],
        "samples_requested": 1, "actual_sample_resolve": "UNVERIFIED; no 4x claim",
        "cvars": effective, "energy_discovery_queries": energy_queries, "show_flags_requested_disabled": disabled,
        "show_flags_readback": [{"name": flag.show_flag_name, "enabled": flag.enabled}
                               for flag in component.get_editor_property("show_flag_settings")],
        "horizontal_fov_deg": component.get_editor_property("fov_angle"), "camera_ue_cm": xyz(actor.get_actor_location()),
        "camera_forward": xyz(actor.get_actor_forward_vector()),
        "camera_right": xyz(actor.get_actor_right_vector()), "camera_up": xyz(actor.get_actor_up_vector()),
        "near_cm": component.get_editor_property("custom_near_clipping_plane"),
        "capture_source": "SCS_SCENE_COLOR_HDR: RGB scene linear; alpha inverse opacity",
        "target": "RGBA16F; r.UsePreExposure=0 requested and numerically queried; actual scale calibration PENDING",
        "exposure": {"ue_bias": post.get_editor_property("auto_exposure_bias"),
                     "method": str(post.get_editor_property("auto_exposure_method")),
                     "physical_camera": post.get_editor_property("auto_exposure_apply_physical_camera_exposure"),
                     "renderlab_pre_exposure": 2**SCENE["active_lighting"]["exposure_stops"],
                     "offline_display_stops": SCENE["active_lighting"]["exposure_stops"],
                     "offline_curve": "per-channel Reinhard then sRGB on UNEXPOSED linear RGB"},
        "loading_and_shader_completion_ms": loading_ms, "capture_and_synchronous_exr_export_ms": elapsed,
        "timing_scope": "CPU wall includes submission, GPU completion/readback and EXR encoding; NOT GPU frame time/throughput",
        "exr_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
        "verification": "file exists only; full image, projection, radiance, clipping, light-color/energy checks PENDING",
        "known_mismatches": SCENE["known_mismatches"],
    })


check_inputs()
if MODE == "prepare-assets":
    prepare()
elif MODE == "capture":
    capture()
else:
    raise ValueError("unknown generated command mode")
