"""R0a adapter for the qualified U0b importer; hidden NullRHI commandlet only.

Reuse its geometry, normals, distance-field, light, exposure and camera checks.
The source AST is restricted to imports and function definitions: U0b's original
top-level import loop is never run. Its output namespace and content path are
isolated. The source hash is recorded, so changes to the adapter's parent are visible.
"""
import ast
import hashlib
import json
import os
from pathlib import Path
import traceback

ROOT = Path("E:/renderlab-work/m7-r0a")
SCENES = ROOT/"scenes"
SOURCE = Path(__file__).with_name("m7-u0b-ue-prepare.py")


def run():
    text = SOURCE.read_text(encoding="utf-8")
    old = '"/Game/M7U0b/"'
    if text.count(old) != 1:
        raise RuntimeError("U0b importer content namespace changed; inspect adapter")
    tree = ast.parse(text.replace(old, '"/Game/M7R0a/"'), filename=str(SOURCE))
    tree.body = [node for node in tree.body if isinstance(node, (ast.Import, ast.ImportFrom, ast.FunctionDef))]
    namespace = dict(ROOT=ROOT, SCENES=SCENES)
    exec(compile(tree, str(SOURCE), "exec"), namespace)
    fixtures = os.environ["RENDERLAB_R0A_FIXTURES"].split(",")
    for name in fixtures:
        record = ROOT/"import"/(name+".json")
        current_sha = hashlib.sha256((SCENES/name/"scene.json").read_bytes()).hexdigest()
        marker = ROOT/"import/toon-adapter.json"
        adapter = json.loads(marker.read_text()) if marker.exists() else {}
        source_sha = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
        toon_current = name!="RF10" or (adapter.get("source_scene_sha256")==current_sha and adapter.get("adapter_source_sha256")==source_sha)
        if record.exists() and json.loads(record.read_text())["source_sha256"]==current_sha and toon_current:
            continue
        scene = json.loads((SCENES/name/"scene.json").read_text())
        if any(m.get("program") not in (None, "toon") for m in scene["materials"]):
            raise ValueError(name+": unsupported custom program")
        if not record.exists() or json.loads(record.read_text())["source_sha256"]!=current_sha:
            namespace["prepare"](name)
        for i, material in enumerate(scene["materials"]):
            if material.get("program")=="toon":
                prepare_toon(namespace["unreal"], name, i, material, scene)
    (ROOT/"import/lineage.json").write_text(json.dumps(dict(
        importer_sha256=hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        project="E:/renderlab-work/m7-u0/ue-project/M5Calibration.uproject",
        parent_cook="E:/renderlab-work/m7-u0b/cook/b/identity.json", fixtures=fixtures,
        content_namespace="/Game/M7R0a", source_scenes=str(SCENES)), indent=2)+"\n")


def prepare_toon(unreal, name, index, parameters, scene):
    """RF10's unoccluded front-lit toon closure, including reflected view-vector rim.

    An unlit material exports authored radiance. No Lumen surface-cache output is
    used as the toon reference. Hit-lighting parity still requires measured proof.
    """
    if scene["sun"]["to_light"][2]<=0:
        raise ValueError("toon front-light registration changed")
    material = unreal.load_asset(f"/Game/M7R0a/{name}/M_{index}")
    lib = unreal.MaterialEditingLibrary
    lib.delete_all_material_expressions(material)
    material.set_editor_property("shading_model", unreal.MaterialShadingModel.MSM_UNLIT)
    node = lib.create_material_expression(material, unreal.MaterialExpressionCustom)
    node.set_editor_property("output_type", unreal.CustomMaterialOutputType.CMOT_FLOAT3)
    entries = []
    for label, cls in (("N", unreal.MaterialExpressionVertexNormalWS), ("V", unreal.MaterialExpressionCameraVectorWS),
                       ("Side", unreal.MaterialExpressionTwoSidedSign)):
        entry = unreal.CustomInput(); entry.set_editor_property("input_name", label); entries.append(entry)
    node.set_editor_property("inputs", entries)
    for label, cls in (("N", unreal.MaterialExpressionVertexNormalWS), ("V", unreal.MaterialExpressionCameraVectorWS),
                       ("Side", unreal.MaterialExpressionTwoSidedSign)):
        source = lib.create_material_expression(material, cls)
        lib.connect_material_expressions(source, "", node, label)
    hlsl = lambda v: "float3("+",".join(str(float(x)) for x in v)+")"
    sx, sy, sz = scene["sun"]["to_light"]
    sun_world = [-sz, sx, sy]  # The same source-Y-up to Unreal-Z-up basis as the importer.
    code = f"""float3 n=normalize(N)*Side; float nv=saturate(dot(n,normalize(V)));
float band=floor(saturate((dot(n,{hlsl(sun_world)})-.12)/.78)*3+.5)/3;
float3 base={hlsl(parameters['base_color'])};
return base*{hlsl(scene['sky']['constant'])}*.25+{hlsl(scene['sun']['lux'])}*base*lerp(float3(.12,.08,.24),1,band)/3.14159265358979
+float3(.12,.2,.3)*pow(1-nv,4)*{2.0**(-scene['exposure_stops'])};"""
    node.set_editor_property("code", code)
    lib.connect_material_property(node, "", unreal.MaterialProperty.MP_EMISSIVE_COLOR)
    lib.recompile_material(material)
    if not unreal.EditorAssetLibrary.save_loaded_asset(material):
        raise RuntimeError("toon material save failed")
    (ROOT/"import/toon-adapter.json").write_text(json.dumps(dict(material=material.get_path_name(), code=code,
        adapter_source_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        source_scene_sha256=hashlib.sha256((SCENES/name/"scene.json").read_bytes()).hexdigest(), sun_world=sun_world,
        status="generated; parity and unobstructed-band proof required before RF10 qualification"), indent=2)+"\n")


try:
    run()
except Exception:
    (ROOT/"import/failure.txt").write_text(traceback.format_exc())
    raise
