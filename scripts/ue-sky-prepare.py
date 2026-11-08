"""Owned sky maps/material; run by ue-sky.py in the isolated hidden NullRHI project."""
import json
import math
import os
from pathlib import Path

import unreal

ROOT = Path(os.environ["RENDERLAB_UE_SKY_ROOT"])
DEST = "/Game/UESky"
sequence = json.loads((ROOT / "inputs/sky-walk.json").read_text())
pose = sequence["frames"][0]
levels = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
actors = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
lib = unreal.MaterialEditingLibrary
assets = unreal.AssetToolsHelpers.get_asset_tools()


def vector(p):
    return unreal.Vector(-100 * p[2], 100 * p[0], 100 * p[1])


cube = unreal.load_asset("/Game/M6Quarry/Environment/quarry-sky-v2")
if not isinstance(cube, unreal.TextureCube):
    raise RuntimeError("registered quarry cube missing")
material = unreal.load_asset(DEST + "/M_PrimarySky") or assets.create_asset("M_PrimarySky", DEST, unreal.Material, unreal.MaterialFactoryNew())
if material is None:
    raise RuntimeError("cannot create owned sky material")
lib.delete_all_material_expressions(material)


def link(source, output, target, pin):
    if not lib.connect_material_expressions(source, output, target, pin):
        raise RuntimeError("material connection failed: " + pin + "; available " + str(lib.get_material_expression_input_names(target)))


material.set_editor_property("two_sided", True)
material.set_editor_property("shading_model", unreal.MaterialShadingModel.MSM_UNLIT)
material.set_editor_property("is_sky", True)
camera = lib.create_material_expression(material, unreal.MaterialExpressionCameraVectorWS)
negative = lib.create_material_expression(material, unreal.MaterialExpressionMultiply)
negative.set_editor_property("const_b", -1.)
link(camera, "", negative, "A")
sample = lib.create_material_expression(material, unreal.MaterialExpressionTextureSample)
sample.set_editor_property("texture", cube)
sample.set_editor_property("sampler_type", unreal.MaterialSamplerType.SAMPLERTYPE_LINEAR_COLOR)
sample.set_editor_property("mip_value_mode", unreal.TextureMipValueMode.TMVM_MIP_LEVEL)
sample.set_editor_property("const_mip_value", 0)
link(negative, "", sample, "UVs")
scale = lib.create_material_expression(material, unreal.MaterialExpressionMultiply)
scale.set_editor_property("const_b", 64.)
link(sample, "RGB", scale, "A")
if not lib.connect_material_property(scale, "", unreal.MaterialProperty.MP_EMISSIVE_COLOR):
    raise RuntimeError("emissive connection failed")
lib.recompile_material(material)
unreal.EditorAssetLibrary.save_loaded_asset(material)

records = []
for name in ("SkyWalk", "SkyCalibration"):
    path = DEST + "/" + name
    if not unreal.EditorAssetLibrary.does_asset_exist(path) and not unreal.EditorAssetLibrary.duplicate_asset("/Game/M6Quarry/View0", path):
        raise RuntimeError("cannot duplicate quarry level " + path)
    if not levels.load_level(path):
        raise RuntimeError("cannot load " + path)
    if name == "SkyCalibration":
        for actor in actors.get_all_level_actors():
            if isinstance(actor, unreal.StaticMeshActor):
                actors.destroy_actor(actor)
    dome = next((a for a in actors.get_all_level_actors() if a.get_actor_label() == "UESky_PrimaryBackground"), None)
    dome = dome or actors.spawn_actor_from_class(unreal.StaticMeshActor, unreal.Vector())
    dome.set_actor_label("UESky_PrimaryBackground")
    component = dome.static_mesh_component
    component.set_static_mesh(unreal.load_asset("/Engine/BasicShapes/Sphere"))
    dome.set_actor_scale3d(unreal.Vector(10000, 10000, 10000))
    component.set_material(0, material)
    for key in ("cast_shadow", "affect_distance_field_lighting", "affect_dynamic_indirect_lighting", "visible_in_ray_tracing"):
        component.set_editor_property(key, False)
    component.set_collision_enabled(unreal.CollisionEnabled.NO_COLLISION)
    cameras = [a for a in actors.get_all_level_actors() if isinstance(a, unreal.CameraActor)]
    if len(cameras) != 1:
        raise RuntimeError("expected one registered camera")
    camera = cameras[0]
    camera.set_actor_location(vector(pose["eye"]), False, False)
    camera.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(vector(pose["eye"]), vector(pose["target"])), False)
    cc = camera.get_component_by_class(unreal.CameraComponent)
    cc.set_editor_property("field_of_view", math.degrees(2 * math.atan(math.tan(pose["fov_y_radians"] / 2) * 1920 / 1080)))
    inventory = []
    for actor in actors.get_all_level_actors():
        if isinstance(actor, unreal.StaticMeshActor):
            c = actor.static_mesh_component
            inventory.append(dict(label=actor.get_actor_label(), mesh=c.static_mesh.get_path_name(),
                cast_shadow=c.get_editor_property("cast_shadow"), rt=c.get_editor_property("visible_in_ray_tracing")))
    if not levels.save_current_level():
        raise RuntimeError("map save failed")
    records.append(dict(map=path, meshes=inventory))

(ROOT / "prepare/content.json").write_text(json.dumps(dict(engine=unreal.SystemLibrary.get_engine_version(),
    cube=cube.get_path_name(), srgb=cube.get_editor_property("srgb"),
    compression=str(cube.get_editor_property("compression_settings")), max_texture_size=cube.get_editor_property("max_texture_size"),
    material=material.get_path_name(), emissive_scale=64, mip=0, direction="-CameraVectorWS; UE basis (-Z,X,Y)",
    sky_yaw=0, units="cd/m2", maps=records), indent=2) + "\n")
