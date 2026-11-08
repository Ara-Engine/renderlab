"""M7-U0b generated bench import. Runs only in a hidden NullRHI commandlet on E:."""
import hashlib
import json
import math
import os
from pathlib import Path
import traceback

import unreal

ROOT = Path(os.environ["RENDERLAB_U0B_ROOT"])
SCENES = Path("E:/renderlab-work/m7-u0/scenes")


def vector(v):
    return unreal.Vector(-100*v[2], 100*v[0], 100*v[1])


def prepare(name):
    source = SCENES/name/"scene.json"
    scene = json.loads(source.read_text())
    path = "/Game/M7U0b/"+name
    levels = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
    if unreal.EditorAssetLibrary.does_asset_exist(path+"/Bench"):
        if not levels.load_level(path+"/Bench"):raise RuntimeError("cannot resume "+path)
    elif not levels.new_level(path+"/Bench"):raise RuntimeError("cannot create "+path)
    if not unreal.EditorAssetLibrary.list_assets(path+"/Imported",recursive=True):
        task = unreal.AssetImportTask()
        for key, value in dict(filename=str(SCENES/name/"scene.gltf"), destination_path=path+"/Imported",
                               automated=True, save=True, replace_existing=False).items():
            task.set_editor_property(key, value)
        unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([task])
    meshes = [unreal.load_asset(p) for p in unreal.EditorAssetLibrary.list_assets(path+"/Imported", recursive=True)]
    meshes = [m for m in meshes if isinstance(m, unreal.StaticMesh)]
    if len(meshes) != len(scene["meshes"]):
        raise RuntimeError(f"{name}: mesh count {len(meshes)}")
    lib = unreal.MaterialEditingLibrary
    materials = []
    for i, m in enumerate(scene["materials"]):
        material = unreal.load_asset(path+"/M_"+str(i)) or unreal.AssetToolsHelpers.get_asset_tools().create_asset("M_"+str(i), path, unreal.Material, unreal.MaterialFactoryNew())
        lib.delete_all_material_expressions(material)
        material.set_editor_property("two_sided", m["double_sided"])
        for rgb, prop in ((m["base_color"], unreal.MaterialProperty.MP_BASE_COLOR), (m["emission"], unreal.MaterialProperty.MP_EMISSIVE_COLOR)):
            node = lib.create_material_expression(material, unreal.MaterialExpressionConstant3Vector)
            node.set_editor_property("constant", unreal.LinearColor(*rgb, 1))
            lib.connect_material_property(node, "", prop)
        for value, prop in ((m["roughness"], unreal.MaterialProperty.MP_ROUGHNESS), (m["metallic"], unreal.MaterialProperty.MP_METALLIC),
                            (.5*m["specular"], unreal.MaterialProperty.MP_SPECULAR)):
            node = lib.create_material_expression(material, unreal.MaterialExpressionConstant)
            node.set_editor_property("r", value)
            lib.connect_material_property(node, "", prop)
        lib.recompile_material(material)
        unreal.EditorAssetLibrary.save_loaded_asset(material)
        materials.append(material)
    actors = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
    for actor in actors.get_all_level_actors():
        if isinstance(actor,(unreal.StaticMeshActor,unreal.CameraActor,unreal.DirectionalLight,unreal.SkyLight)):
            actors.destroy_actor(actor)
    subsystem = unreal.get_editor_subsystem(unreal.StaticMeshEditorSubsystem) or unreal.get_default_object(unreal.StaticMeshEditorSubsystem)
    expected = {m["name"]: m for m in scene["meshes"]}
    inventory = []
    for mesh in meshes:
        actor = actors.spawn_actor_from_class(unreal.StaticMeshActor, unreal.Vector())
        actor.static_mesh_component.set_static_mesh(mesh)
        origin, extent = actor.get_actor_bounds(False)
        bounds = [[origin.x-extent.x, origin.y-extent.y, origin.z-extent.z], [origin.x+extent.x, origin.y+extent.y, origin.z+extent.z]]
        def error(m):
            p = [vector(v) for v in m["positions"]]
            wanted = [[min(getattr(v,k) for v in p) for k in ("x","y","z")], [max(getattr(v,k) for v in p) for k in ("x","y","z")]]
            return max(abs(bounds[i][j]-wanted[i][j]) for i in range(2) for j in range(3))
        key = min(expected, key=lambda k:error(expected[k]))
        m = expected.pop(key)
        delta = error(m)
        if delta > .005:
            raise RuntimeError(f"import geometry mismatch {key}: bounds error {delta}, triangles {mesh.get_num_triangles(0)} expected {len(m['indices'])//3}")
        actor.set_actor_label("U0b_"+key)
        settings = subsystem.get_nanite_settings(mesh)
        settings.set_editor_property("enabled", False)
        subsystem.set_nanite_settings(mesh, settings, True)
        build = subsystem.get_lod_build_settings(mesh, 0)
        for k,v in (("remove_degenerates",False),("recompute_normals",False),("use_high_precision_tangent_basis",True),
                    ("distance_field_resolution_scale",1.0),("generate_distance_field_as_if_two_sided",True)):
            build.set_editor_property(k,v)
        subsystem.set_lod_build_settings(mesh, 0, build)
        if mesh.get_num_triangles(0) != len(m["indices"])//3:
            raise RuntimeError(f"built triangle mismatch {key}: {mesh.get_num_triangles(0)} expected {len(m['indices'])//3}")
        mesh.set_material(0, materials[m["material"]])
        unreal.EditorAssetLibrary.save_loaded_asset(mesh)
        if m.get("moving"):
            actor.static_mesh_component.set_mobility(unreal.ComponentMobility.MOVABLE)
        inventory.append(dict(source=key, asset=mesh.get_path_name(), actor=actor.get_name(), triangles=mesh.get_num_triangles(0),
                              bounds_error_cm=delta, distance_field_resolution_scale=1, distance_field_two_sided=True))
    light = actors.spawn_actor_from_class(unreal.DirectionalLight, unreal.Vector())
    light.set_actor_label("U0b_Sun")
    lc = light.get_component_by_class(unreal.DirectionalLightComponent)
    lc.set_mobility(unreal.ComponentMobility.MOVABLE)
    if scene["sun"]:
        sun = scene["sun"]
        light.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(unreal.Vector(), vector([-v for v in sun["to_light"]])),False)
        scale = max(sun["lux"])
        lc.set_intensity(scale)
        lc.set_light_color(unreal.LinearColor(*(v/scale for v in sun["lux"]),1),False)
        lc.set_editor_property("light_source_angle",math.degrees(2*sun["angular_radius"]))
    else:
        lc.set_intensity(0)
    lc.set_indirect_lighting_intensity(1)
    cube = unreal.load_asset("/Game/BenchContactFold/constant")
    if cube is None:
        raise RuntimeError("registered uniform cube missing")
    sky = actors.spawn_actor_from_class(unreal.SkyLight, unreal.Vector())
    sc = sky.get_component_by_class(unreal.SkyLightComponent)
    sc.set_mobility(unreal.ComponentMobility.MOVABLE)
    for k,v in (("source_type",unreal.SkyLightSourceType.SLS_SPECIFIED_CUBEMAP),("cubemap",cube),
                ("lower_hemisphere_is_black",False),("real_time_capture",False),("cubemap_resolution",16)):
        sc.set_editor_property(k,v)
    rgb = scene["sky"]["constant"]
    scale = max(rgb)
    sc.set_intensity(scale)
    sc.set_light_color(unreal.LinearColor(*(v/max(scale,1e-12) for v in rgb),1))
    sc.set_cast_shadows(True)
    sc.recapture_sky()
    pose = scene["poses"][0]
    camera = actors.spawn_actor_from_class(unreal.CameraActor,vector(pose["eye"]))
    camera.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(vector(pose["eye"]),vector(pose["target"])),False)
    camera.set_editor_property("auto_activate_for_player",unreal.AutoReceiveInput.PLAYER0)
    cc = camera.get_component_by_class(unreal.CameraComponent)
    cc.set_editor_property("field_of_view",math.degrees(2*math.atan(math.tan(math.radians(scene["fov_y_degrees"])/2)*1920/1080)))
    cc.set_editor_property("aspect_ratio",1920/1080)
    cc.set_editor_property("constrain_aspect_ratio",False)
    post=unreal.PostProcessSettings()
    for k,v in (("auto_exposure_method",unreal.AutoExposureMethod.AEM_MANUAL),("auto_exposure_apply_physical_camera_exposure",False),("auto_exposure_bias",scene["exposure_stops"])):
        post.set_editor_property("override_"+k,True);post.set_editor_property(k,v)
    cc.set_editor_property("post_process_settings",post)
    cc.set_editor_property("post_process_blend_weight",1.)
    cls=unreal.BlueprintEditorLibrary.generated_class(unreal.load_asset("/Game/BenchContactFold/S1GameMode"))
    world=unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
    world.get_world_settings().set_editor_property("default_game_mode",cls)
    if not levels.save_current_level():
        raise RuntimeError("cannot save bench")
    result=dict(bench=name,source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),meshes=inventory,engine=unreal.SystemLibrary.get_engine_version())
    (ROOT/"import"/(name+".json")).write_text(json.dumps(result,indent=2))


try:
    for name in ("SV","S4","S5","S2G"):
        if not (ROOT/"import"/(name+".json")).exists():
            prepare(name)
except Exception:
    (ROOT/"import/failure.txt").write_text(traceback.format_exc())
    raise
