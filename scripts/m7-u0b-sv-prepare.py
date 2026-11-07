"""SV diffuse sky-visibility control; hidden NullRHI, reusing the registered meshes."""
import json
import math
import os
from pathlib import Path
import unreal

ROOT=Path(os.environ["RENDERLAB_U0B_ROOT"])
source=Path("E:/renderlab-work/m7-u0/scenes/SV/scene.json")
scene=json.loads(source.read_text())
inventory=json.loads((ROOT/"import/SV.json").read_text())["meshes"]
path="/Game/M7U0b/SVSky"
levels=unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
if not levels.new_level(path+"/Bench"):raise RuntimeError("cannot create SV sky control")
actors=unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
lib=unreal.MaterialEditingLibrary
tools=unreal.AssetToolsHelpers.get_asset_tools()


def vector(p):return unreal.Vector(-100*p[2],100*p[0],100*p[1])


def material(name,sky=False):
    asset=unreal.load_asset(path+"/"+name) or tools.create_asset(name,path,unreal.Material,unreal.MaterialFactoryNew())
    lib.delete_all_material_expressions(asset)
    asset.set_editor_property("two_sided",True)
    if sky:
        asset.set_editor_property("shading_model",unreal.MaterialShadingModel.MSM_UNLIT)
        asset.set_editor_property("is_sky",True)
        node=lib.create_material_expression(asset,unreal.MaterialExpressionConstant3Vector)
        node.set_editor_property("constant",unreal.LinearColor(*scene["sky"]["constant"],1))
        lib.connect_material_property(node,"",unreal.MaterialProperty.MP_EMISSIVE_COLOR)
    else:
        node=lib.create_material_expression(asset,unreal.MaterialExpressionConstant3Vector)
        node.set_editor_property("constant",unreal.LinearColor(.5,.5,.5,1))
        lib.connect_material_property(node,"",unreal.MaterialProperty.MP_BASE_COLOR)
        for value,prop in ((.8,unreal.MaterialProperty.MP_ROUGHNESS),(0.,unreal.MaterialProperty.MP_METALLIC),(0.,unreal.MaterialProperty.MP_SPECULAR)):
            node=lib.create_material_expression(asset,unreal.MaterialExpressionConstant)
            node.set_editor_property("r",value);lib.connect_material_property(node,"",prop)
    lib.recompile_material(asset);unreal.EditorAssetLibrary.save_loaded_asset(asset)
    return asset


diffuse=material("M_Diffuse");sky_material=material("M_PrimarySky",True)
for m in scene["materials"]:
    if m["base_color"]!=[.5,.5,.5] or m["roughness"]!=.8 or m["metallic"]!=0:
        raise RuntimeError("SV source material changed")
for item in inventory:
    actor=actors.spawn_actor_from_class(unreal.StaticMeshActor,unreal.Vector())
    actor.set_actor_label("SVSky_"+item["source"])
    actor.static_mesh_component.set_static_mesh(unreal.load_asset(item["asset"]))
    actor.static_mesh_component.set_material(0,diffuse)  # component override; original assets stay intact

# Match G1's visible constant primary sky before temporal reconstruction. It affects
# neither distance fields, Lumen cards, shadow rays nor the specified lighting cube.
sphere=next(i for i in inventory if "sphere" in i["source"])
dome=actors.spawn_actor_from_class(unreal.StaticMeshActor,unreal.Vector())
dome.set_actor_label("SVSky_PrimaryBackground")
component=dome.static_mesh_component
component.set_static_mesh(unreal.load_asset(sphere["asset"]))
origin,_=dome.get_actor_bounds(False)
dome.set_actor_scale3d(unreal.Vector(10000,10000,10000))
dome.set_actor_location(origin*-10000,False,False)
component.set_material(0,sky_material)
for key in ("cast_shadow","affect_distance_field_lighting","affect_dynamic_indirect_lighting","visible_in_ray_tracing"):
    component.set_editor_property(key,False)
component.set_collision_enabled(unreal.CollisionEnabled.NO_COLLISION)

sun=actors.spawn_actor_from_class(unreal.DirectionalLight,unreal.Vector())
sun.get_component_by_class(unreal.DirectionalLightComponent).set_intensity(0)
sun.get_component_by_class(unreal.DirectionalLightComponent).set_editor_property("light_source_angle",0.)
sky=actors.spawn_actor_from_class(unreal.SkyLight,unreal.Vector())
sc=sky.get_component_by_class(unreal.SkyLightComponent)
sc.set_mobility(unreal.ComponentMobility.MOVABLE)
for key,value in (("source_type",unreal.SkyLightSourceType.SLS_SPECIFIED_CUBEMAP),
                  ("cubemap",unreal.load_asset("/Game/BenchContactFold/constant")),
                  ("lower_hemisphere_is_black",False),("real_time_capture",False),("cubemap_resolution",16)):
    sc.set_editor_property(key,value)
sc.set_intensity(4000);sc.set_light_color(unreal.LinearColor(1,1,1,1));sc.set_cast_shadows(True);sc.recapture_sky()
pose=scene["poses"][0]
camera=actors.spawn_actor_from_class(unreal.CameraActor,vector(pose["eye"]))
camera.set_actor_rotation(unreal.MathLibrary.find_look_at_rotation(vector(pose["eye"]),vector(pose["target"])),False)
camera.set_editor_property("auto_activate_for_player",unreal.AutoReceiveInput.PLAYER0)
cc=camera.get_component_by_class(unreal.CameraComponent)
cc.set_editor_property("field_of_view",math.degrees(2*math.atan(math.tan(math.radians(45)/2)*1920/1080)))
cc.set_editor_property("aspect_ratio",1920/1080);cc.set_editor_property("constrain_aspect_ratio",False)
post=unreal.PostProcessSettings()
for key,value in (("auto_exposure_method",unreal.AutoExposureMethod.AEM_MANUAL),
                  ("auto_exposure_apply_physical_camera_exposure",False),("auto_exposure_bias",-11.)):
    post.set_editor_property("override_"+key,True);post.set_editor_property(key,value)
cc.set_editor_property("post_process_settings",post);cc.set_editor_property("post_process_blend_weight",1.)
world=unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
world.get_world_settings().set_editor_property("default_game_mode",unreal.BlueprintEditorLibrary.generated_class(unreal.load_asset("/Game/BenchContactFold/S1GameMode")))
if not levels.save_current_level():raise RuntimeError("SV sky level save failed")
(ROOT/"import/SVSky.json").write_text(json.dumps(dict(level=path+"/Bench",source=str(source),
    specular=0,diffuse=.5,roughness=.8,sky=4000,meshes=inventory,
    primary_sky=dict(unlit=True,is_sky=True,ray_tracing=False,distance_fields=False,lumen_cards=False)),indent=2))
